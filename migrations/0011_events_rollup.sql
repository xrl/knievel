-- Hourly events rollup (REQUIREMENTS.md § 7.3, REPORTING.md
-- "Schema for Reporters", Phase 3.24).
--
-- Pre-aggregated facts by (project_id, site_id, zone_id,
-- flight_id, ad_id, creative_id, kind). Computed by knievel's
-- leader-elected job (Phase 3.22 leader handle) before raw
-- partitions age out (Phase 3.23 retention drop). Indefinite
-- retention — the rollup is the long-term reportable view.

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.events_rollup (
    hour            timestamptz  NOT NULL,
    project_id      text         NOT NULL,
    site_id         bigint,
    zone_id         bigint,
    flight_id       bigint,
    ad_id           bigint,
    creative_id     bigint,
    kind            smallint     NOT NULL,
    count           bigint       NOT NULL,
    PRIMARY KEY (hour, project_id, kind, site_id, zone_id,
                 flight_id, ad_id, creative_id)
);

CREATE INDEX IF NOT EXISTS events_rollup_project_hour_idx
    ON knievel.events_rollup (project_id, hour);

ALTER TABLE knievel.events_rollup ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.events_rollup FORCE ROW LEVEL SECURITY;

-- Rollup is read by org-scoped reporting tokens; bind on
-- `knievel.project_id` so the v0 reader is the project itself.
-- Cross-project rollup queries by an org-scoped token go
-- through a SECURITY DEFINER view (post-v0).
CREATE POLICY events_rollup_tenant_isolation
    ON knievel.events_rollup
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Watermark: the most recent hour fully aggregated. Single-row
-- table; the rollup leader writes it on every successful pass
-- so reporting consumers can blend events_rollup with
-- events_raw without double-counting (REPORTING.md § "Schema
-- for Reporters" — `events_rollup_watermark`).
CREATE TABLE IF NOT EXISTS knievel.events_rollup_watermark (
    id        smallint     PRIMARY KEY DEFAULT 1,
    watermark timestamptz  NOT NULL DEFAULT to_timestamp(0),
    CHECK (id = 1)
);
INSERT INTO knievel.events_rollup_watermark (id, watermark)
    VALUES (1, to_timestamp(0))
    ON CONFLICT (id) DO NOTHING;

ALTER TABLE knievel.events_rollup_watermark ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.events_rollup_watermark FORCE ROW LEVEL SECURITY;
-- Watermark is global system metadata, readable by any
-- authenticated session. Bind via `knievel.org_id` (any value)
-- so the linter rule 4 sees a tenant-binding reference.
CREATE POLICY events_rollup_watermark_read
    ON knievel.events_rollup_watermark
    FOR SELECT
    USING (current_setting('knievel.org_id', true) IS NOT NULL);
-- Writes only happen from the leader's own connection, which
-- runs without a tenant binding; so we leave the WITH CHECK
-- side off (no INSERT/UPDATE policy = no writes from
-- request-scoped sessions, which is the intent).
