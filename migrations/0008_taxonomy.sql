-- Read-only taxonomy — channels, priorities, ad_types.
--
-- Refs: API.md § 3.9, REQUIREMENTS.md § 5.
--
-- These are project-scoped (per-project lists) but read-only via
-- the API in v0; they're seeded at project creation by a future
-- handler hook in Phase 3.13. Write endpoints land post-v0
-- (REQUIREMENTS.md § 11 roadmap).
--
-- Priorities are ordered by `tier` (lower tier wins; the decision
-- API picks the highest non-empty priority). Default seeds give
-- callers a 1..3 ladder out of the box.

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.channels (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    name        text         NOT NULL,
    created_at  timestamptz  NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS channels_project_id_idx
    ON knievel.channels (project_id);

ALTER TABLE knievel.channels ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.channels FORCE ROW LEVEL SECURITY;
CREATE POLICY channels_tenant ON knievel.channels
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

CREATE TABLE IF NOT EXISTS knievel.priorities (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    name        text         NOT NULL,
    tier        int          NOT NULL,
    created_at  timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, tier)
);
CREATE INDEX IF NOT EXISTS priorities_project_id_idx
    ON knievel.priorities (project_id);

ALTER TABLE knievel.priorities ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.priorities FORCE ROW LEVEL SECURITY;
CREATE POLICY priorities_tenant ON knievel.priorities
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

CREATE TABLE IF NOT EXISTS knievel.ad_types (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    name        text         NOT NULL,
    width       int,
    height      int,
    created_at  timestamptz  NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS ad_types_project_id_idx
    ON knievel.ad_types (project_id);

ALTER TABLE knievel.ad_types ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.ad_types FORCE ROW LEVEL SECURITY;
CREATE POLICY ad_types_tenant ON knievel.ad_types
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));
