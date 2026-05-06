-- Fixture: rule 3 — CREATE TABLE in knievel without ENABLE RLS rejected.
SET search_path TO knievel, public;

CREATE TABLE knievel.baz (
    id         bigserial PRIMARY KEY,
    project_id bigint    NOT NULL
);
-- intentionally no ENABLE ROW LEVEL SECURITY for knievel.baz
