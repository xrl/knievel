-- Fixture: rule 4 — CREATE POLICY USING without tenant binding rejected.
SET search_path TO knievel, public;

CREATE TABLE knievel.qux (
    id         bigserial PRIMARY KEY,
    project_id bigint    NOT NULL
);
ALTER TABLE knievel.qux ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.qux FORCE ROW LEVEL SECURITY;

-- USING (true) does NOT reference current_setting('knievel.project_id')
CREATE POLICY qux_global ON knievel.qux
    FOR ALL
    USING (true);
