-- idempotency_keys — 24h replay store for `Idempotency-Key`.
--
-- Refs: API.md "Idempotency", REQUIREMENTS.md § 10.9
-- ("Idempotency cache miss / corruption" degraded mode), TESTING.md
-- § 6.4 (`create_idempotency_key_replay`,
-- `create_idempotency_key_mismatch_body`).
--
-- Lookup key is `(org_id, project_id, key, route)`. `body_hash`
-- distinguishes "same key, same body" (replay) from "same key,
-- different body" (409 idempotency_conflict). Cleanup runs on the
-- leader (Phase 3.22).
--
-- project_id is NULLable for org-scoped writes (e.g.
-- `POST /v1/orgs/{orgId}/projects`). The unique index uses
-- `coalesce(project_id, '')` so the lookup is well-defined for
-- both org-scoped and project-scoped writes on Postgres 14
-- without relying on PG15's `NULLS NOT DISTINCT`.

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.idempotency_keys (
    org_id           text NOT NULL REFERENCES knievel.organizations(id),
    project_id       text REFERENCES knievel.projects(id),
    key              text NOT NULL,
    route            text NOT NULL,
    body_hash        text NOT NULL,
    response_status  int  NOT NULL,
    response_body    bytea NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    expires_at       timestamptz NOT NULL DEFAULT (now() + interval '24 hours')
);

CREATE UNIQUE INDEX IF NOT EXISTS idempotency_keys_lookup_idx
    ON knievel.idempotency_keys (org_id, coalesce(project_id, ''), key, route);

CREATE INDEX IF NOT EXISTS idempotency_keys_expires_at_idx
    ON knievel.idempotency_keys (expires_at);

ALTER TABLE knievel.idempotency_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.idempotency_keys FORCE ROW LEVEL SECURITY;

-- One FOR ALL policy: tenant-scoped on read, tight on write.
-- The auth-bootstrap GUC isn't needed here — by the time a handler
-- writes an idempotency row, the tenant binding is already
-- established via `db::begin_bound`.
CREATE POLICY idempotency_keys_tenant
    ON knievel.idempotency_keys
    FOR ALL
    USING (
        org_id = current_setting('knievel.org_id', true)
        OR project_id = current_setting('knievel.project_id', true)
    )
    WITH CHECK (org_id = current_setting('knievel.org_id', true));
