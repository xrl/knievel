-- Fixture: a clean knievel-schema table — RLS, FORCE, and a
-- tenant-bound policy. Linter accepts.
SET search_path TO knievel, public;

CREATE TABLE knievel.advertisers (
    id          bigserial   PRIMARY KEY,
    org_id      bigint      NOT NULL,
    project_id  bigint      NOT NULL,
    external_id text,
    name        text        NOT NULL,
    is_active   boolean     NOT NULL DEFAULT true,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE knievel.advertisers ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.advertisers FORCE  ROW LEVEL SECURITY;

CREATE POLICY advertisers_tenant_isolation ON knievel.advertisers
    FOR ALL
    USING (project_id = current_setting('knievel.project_id')::bigint);
