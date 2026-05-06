-- Fixture: rule 2 — NO FORCE ROW LEVEL SECURITY must be rejected.
SET search_path TO knievel, public;

CREATE TABLE knievel.bar (id bigserial PRIMARY KEY);
ALTER TABLE knievel.bar ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.bar FORCE ROW LEVEL SECURITY;

CREATE POLICY bar_isolation ON knievel.bar
    FOR ALL
    USING (project_id = current_setting('knievel.project_id')::bigint);

ALTER TABLE knievel.bar NO FORCE ROW LEVEL SECURITY;
