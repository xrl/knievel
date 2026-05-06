-- Fixture: rule 1 — DISABLE ROW LEVEL SECURITY must be rejected.
SET search_path TO knievel, public;

CREATE TABLE knievel.foo (id bigserial PRIMARY KEY);
ALTER TABLE knievel.foo ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.foo FORCE ROW LEVEL SECURITY;

CREATE POLICY foo_isolation ON knievel.foo
    FOR ALL
    USING (project_id = current_setting('knievel.project_id')::bigint);

ALTER TABLE knievel.foo DISABLE ROW LEVEL SECURITY;
