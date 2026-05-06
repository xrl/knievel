-- audit_log — append-only, monthly range-partitioned by ts.
--
-- Refs:
--   REQUIREMENTS.md § 7.3 (events tables — audit_log spec),
--   AUTH.md "Audit and observability".
--
-- One row per privileged or sensitive operation: `force.*` decision
-- overrides, HMAC-secret rotation, project deletion, member role
-- changes, token mint/revoke. Default retention 365 days,
-- partitioned monthly. Writers land incrementally — first writer
-- is token mint in 3.6; force.* writes land in 3.19.
--
-- Append-only is enforced by RLS: only `FOR SELECT` and `FOR INSERT`
-- policies exist. Postgres' default-deny semantics for FORCE'd RLS
-- mean that an UPDATE or DELETE statement finds zero rows visible
-- to the operation, so `audit_log` rows are effectively immutable.

SET search_path TO knievel, public;

-- Parent table. Partitioned tables can carry FKs to non-partitioned
-- tables (Postgres 12+); RLS on the parent applies to all leaves.
CREATE TABLE IF NOT EXISTS knievel.audit_log (
    ts            timestamptz NOT NULL DEFAULT now(),
    org_id        text NOT NULL REFERENCES knievel.organizations(id),
    project_id    text REFERENCES knievel.projects(id),
    actor         text NOT NULL,        -- token name (opaque) or `(iss, sub, azp)` (JWT)
    operation     text NOT NULL,        -- e.g. "tokens.mint", "decisions.force"
    payload_hash  text,
    reason        text,
    request_id    text
) PARTITION BY RANGE (ts);

-- Seed leaf covering the current year. Partition manager's
-- monthly cadence lands in a later phase; until then this gives
-- writers somewhere to land. Wide enough that v0 development
-- writes always have a partition without operator action.
CREATE TABLE IF NOT EXISTS knievel.audit_log_p_seed
    PARTITION OF knievel.audit_log
    FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');

CREATE INDEX IF NOT EXISTS audit_log_org_id_ts_idx
    ON knievel.audit_log (org_id, ts DESC);

ALTER TABLE knievel.audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.audit_log FORCE ROW LEVEL SECURITY;
-- The leaf carries its own ENABLE/FORCE so the migration linter's
-- rule 3 (every CREATE TABLE in knievel must enable RLS) is happy.
-- Functionally redundant — the parent's policies cover all leaves.
ALTER TABLE knievel.audit_log_p_seed ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.audit_log_p_seed FORCE ROW LEVEL SECURITY;

-- SELECT: tenant-scoped read. Org-scoped sessions see their own
-- org's rows; project-scoped sessions see rows belonging to their
-- parent org via the inheritance subquery.
CREATE POLICY audit_log_select
    ON knievel.audit_log
    FOR SELECT
    USING (
        org_id = current_setting('knievel.org_id', true)
        OR org_id IN (
            SELECT p.org_id FROM knievel.projects p
            WHERE p.id = current_setting('knievel.project_id', true)
        )
    );

-- INSERT: writers must run under an explicit org binding. The
-- linter's rule 4 (Phase 3.4 generalization: either project_id
-- or org_id is acceptable) is satisfied by the org_id reference.
CREATE POLICY audit_log_insert
    ON knievel.audit_log
    FOR INSERT
    WITH CHECK (org_id = current_setting('knievel.org_id', true));
