-- API tokens — opaque-token storage for AUTH.md "Opaque Tokens".
--
-- Refs:
--   REQUIREMENTS.md § 4.3 (token modes), AUTH.md "Opaque Tokens",
--   API.md § 2.2 (Tokens API).
--
-- Wire format of the secret is `kvl_<env>_<scope>_<id_short>_<secret>`
-- (REQUIREMENTS.md § 4.3). The `id_short` segment routes to a row
-- here by `id = 'tok_' || id_short`; the `secret` segment is verified
-- against `secret_hash` (argon2id). The plaintext secret is never
-- stored — the row carries only the salted hash.

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.api_tokens (
    id            text PRIMARY KEY,
    org_id        text NOT NULL REFERENCES knievel.organizations(id),
    project_id    text REFERENCES knievel.projects(id),
    scope         text NOT NULL CHECK (scope IN ('org', 'project')),
    role          text NOT NULL CHECK (role IN
                      ('reader', 'editor', 'admin', 'org-admin', 'org-owner')),
    name          text NOT NULL,
    secret_hash   text NOT NULL,
    ip_allowlist  text[] NOT NULL DEFAULT '{}',
    expires_at    timestamptz,
    revoked_at    timestamptz,
    last_used_at  timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    -- Scope sanity: a project-scoped token must name a project; an
    -- org-scoped token must not.
    CHECK (
        (scope = 'project' AND project_id IS NOT NULL)
        OR (scope = 'org' AND project_id IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS api_tokens_org_id_idx ON knievel.api_tokens (org_id);
CREATE INDEX IF NOT EXISTS api_tokens_project_id_idx ON knievel.api_tokens (project_id);

ALTER TABLE knievel.api_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.api_tokens FORCE ROW LEVEL SECURITY;

-- Three USING branches:
--   1. Tenant access by `org_id` (org-scoped sessions).
--   2. Tenant access via the project-binding inheritance path
--      (project-scoped sessions reading their own org's tokens).
--   3. Auth-bootstrap bypass: the Principal extractor (Phase 3.3)
--      sets `knievel.auth_lookup_id` to a single token id before
--      the verify lookup, which scopes the bypass to one row by
--      primary key. This is the chicken-and-egg fix — the auth
--      layer has to read `secret_hash` before any `org_id` is
--      known. The bypass is single-row and read-only via the WHERE
--      clause that always accompanies it (`WHERE id = $1`).
-- WITH CHECK is intentionally tight — writes only land under the
-- explicit `org_id` binding; the bootstrap GUC cannot be used to
-- escalate writes.
CREATE POLICY api_tokens_tenant_isolation
    ON knievel.api_tokens
    USING (
        org_id = current_setting('knievel.org_id', true)
        OR org_id IN (
            SELECT p.org_id
            FROM knievel.projects p
            WHERE p.id = current_setting('knievel.project_id', true)
        )
        OR id = current_setting('knievel.auth_lookup_id', true)
    )
    WITH CHECK (org_id = current_setting('knievel.org_id', true));
