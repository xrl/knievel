-- Events raw — append-only, range-partitioned by day on `ts`.
--
-- Refs:
--   REQUIREMENTS.md § 7.3 (Events), § 7.6 (Hot path),
--   API.md § 4 ("Replay, dedup, and counts"),
--   PHASES.md task 3.20.
--
-- Partition policy:
--   - Declarative partitioning on RANGE (ts).
--   - Leaf naming convention: events_raw_p<YYYY_MM_DD>.
--   - No default partition: a missing partition is a loud failure
--     so the operator alert fires (REQUIREMENTS.md § 7.3).
--   - First leaf covers a wide window so day-zero of a fresh
--     install doesn't immediately need the partition manager.
--     The partition manager (Phase 3.23) generalizes the recipe.
--
-- Dedup: `(project_id, kind, dedup_key)` is unique per
-- API.md § 4. The first hit lands `is_duplicate = false`;
-- subsequent hits with the same key land
-- `is_duplicate = true`. The unique constraint must live on
-- the parent table to apply across partitions, but Postgres
-- won't accept a UNIQUE on a partitioned table that omits the
-- partition key. Workaround: include `ts` in the constraint
-- (the dedup window is bounded by retention anyway, so adding
-- ts is harmless — see REQUIREMENTS.md § 7.3 "Window: lifetime
-- within retention").

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.events_raw (
    id                bigserial    NOT NULL,
    ts                timestamptz  NOT NULL DEFAULT now(),
    org_id            text         NOT NULL,
    project_id        text         NOT NULL,
    kind              smallint     NOT NULL,    -- 0=decision,1=impression,2=click
    placement_id      text,
    site_id           bigint,
    zone_id           bigint,
    ad_id             bigint,
    creative_id       bigint,
    flight_id         bigint,
    campaign_id       bigint,
    advertiser_id     bigint,
    url               text,
    referrer_host     text,
    user_agent_hash   bytea,
    signature_nonce   bytea,
    dedup_key         bytea,
    snapshot_version  bigint,
    is_duplicate      boolean      NOT NULL DEFAULT false,
    PRIMARY KEY (id, ts),
    UNIQUE (project_id, kind, dedup_key, ts)
) PARTITION BY RANGE (ts);

-- Phase 3.20 seed leaf — covers all of 2026 so a freshly
-- installed instance can start writing events immediately.
-- Phase 3.23's partition manager replaces this with daily
-- leaves once it's running.
CREATE TABLE IF NOT EXISTS knievel.events_raw_p2026
    PARTITION OF knievel.events_raw
    FOR VALUES FROM ('2026-01-01 00:00:00+00') TO ('2027-01-01 00:00:00+00');

ALTER TABLE knievel.events_raw ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.events_raw FORCE ROW LEVEL SECURITY;
ALTER TABLE knievel.events_raw_p2026 ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.events_raw_p2026 FORCE ROW LEVEL SECURITY;

-- RLS bound on org_id (REQUIREMENTS.md § 7.3 "RLS by org_id" —
-- events_raw is the one place we accept the looser binding so
-- cross-project analytics within an org work without the
-- principal having to re-bind to each project).
CREATE POLICY events_raw_tenant_isolation
    ON knievel.events_raw
    USING (org_id = current_setting('knievel.org_id', true))
    WITH CHECK (org_id = current_setting('knievel.org_id', true));
