-- Fixture: a table in `public` is not knievel's concern.
-- The linter accepts CREATE TABLE outside the knievel schema
-- without RLS — operators may have their own non-tenant tables.
CREATE TABLE public.something (
    id bigserial PRIMARY KEY
);
