-- Tenant model: organizations + projects, with row-level security
-- bound on session-scoped GUCs.
--
-- Refs:
--   REQUIREMENTS.md § 4 (multi-tenancy), § 7.1 (schema and isolation),
--   § 7.1.1 (tenant isolation verification — RLS gates).
--   AUTH.md "Authorization" (org/project scope semantics).
--
-- Session bindings (set per request by the Principal extractor in
-- Phase 3.2; in tests by `testlib::tenant`):
--   knievel.org_id      — always set on authenticated requests.
--   knievel.project_id  — set when the request targets a specific
--                         project (project-scoped token, or org-scoped
--                         token addressing /v1/projects/{p}/...).
-- Policies use the `, true` form of `current_setting` so an unset
-- GUC returns NULL (no rows) rather than erroring.

SET search_path TO knievel, public;

-- Organizations: billing entity, top of the tenant hierarchy.
-- ID is text (e.g. `org_AbCd...`) per API.md "Path Structure";
-- external_id is the caller-assigned URL-safe alternate.
CREATE TABLE IF NOT EXISTS knievel.organizations (
    id          text PRIMARY KEY,
    external_id text UNIQUE,
    name        text NOT NULL,
    is_active   boolean NOT NULL DEFAULT true,
    etag        text NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Projects: the isolated ad-serving workspace. Carries the
-- per-project HMAC signing secret (REQUIREMENTS.md § 6.3) and the
-- force-overrides project flag (API.md § 1, AUTH.md "Endpoint →
-- minimum role"). Rotation overlap (previous secret) lands in
-- Phase 3.16 — single secret column for now.
CREATE TABLE IF NOT EXISTS knievel.projects (
    id                    text PRIMARY KEY,
    org_id                text NOT NULL REFERENCES knievel.organizations(id),
    external_id           text,
    name                  text NOT NULL,
    is_active             boolean NOT NULL DEFAULT true,
    hmac_secret           bytea NOT NULL DEFAULT gen_random_bytes(32),
    allow_force_decision  boolean NOT NULL DEFAULT false,
    etag                  text NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now(),
    UNIQUE (org_id, external_id)
);

CREATE INDEX IF NOT EXISTS projects_org_id_idx ON knievel.projects (org_id);

ALTER TABLE knievel.organizations ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.organizations FORCE ROW LEVEL SECURITY;
ALTER TABLE knievel.projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.projects FORCE ROW LEVEL SECURITY;

-- Organization isolation: a session bound to org X sees only X.
-- A project-scoped session sees the parent org of the bound project
-- (so a project token can read its own org's name without an
-- additional grant). The subquery references knievel.project_id so
-- the migration linter (REQUIREMENTS.md § 7.1.1 gate (2) rule 4)
-- sees the binding it expects.
-- WITH CHECK is intentionally tighter than USING — writes only land
-- under the explicit org binding, never via the project-scope
-- inheritance path.
CREATE POLICY organizations_tenant_isolation
    ON knievel.organizations
    USING (
        id = current_setting('knievel.org_id', true)
        OR id IN (
            SELECT p.org_id
            FROM knievel.projects p
            WHERE p.id = current_setting('knievel.project_id', true)
        )
    )
    WITH CHECK (id = current_setting('knievel.org_id', true));

-- Project isolation: a session bound to project P sees only P; a
-- session bound to an org sees every project under that org.
-- WITH CHECK requires org_id to match — a project-scoped session
-- cannot create rows in the projects table (and an org-scoped
-- session cannot mis-attribute a project to a different org).
CREATE POLICY projects_tenant_isolation
    ON knievel.projects
    USING (
        org_id = current_setting('knievel.org_id', true)
        OR id = current_setting('knievel.project_id', true)
    )
    WITH CHECK (
        org_id = current_setting('knievel.org_id', true)
    );
