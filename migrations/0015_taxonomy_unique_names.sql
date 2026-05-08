-- Fix(taxonomy): UNIQUE (project_id, name) on channels and ad_types.
--
-- Audit finding: opus O23. Re-running seed_default_taxonomy on an
-- existing project silently double-inserts rows because neither
-- knievel.channels nor knievel.ad_types has a unique constraint on
-- (project_id, name). knievel.priorities already has UNIQUE
-- (project_id, tier) from 0008_taxonomy.sql; the analogous name
-- constraint was missing on the other two tables.
--
-- A name uniqueness constraint on priorities is added for
-- symmetry — a project with tier 1 named "House" and a second tier
-- 1 named "House2" would already collide on the tier UNIQUE, but the
-- name constraint catches duplicate names at different tiers too.
--
-- This migration adds no new tables, so no ENABLE ROW LEVEL SECURITY
-- block is needed. Existing RLS on all three tables is untouched.

SET search_path TO knievel, public;

-- Unique name per project on channels.
ALTER TABLE knievel.channels
    ADD CONSTRAINT channels_project_id_name_key
        UNIQUE (project_id, name);

-- Unique name per project on ad_types.
ALTER TABLE knievel.ad_types
    ADD CONSTRAINT ad_types_project_id_name_key
        UNIQUE (project_id, name);

-- Unique name per project on priorities (tier uniqueness already
-- existed; name uniqueness prevents a second "House" at a different
-- tier, which would be confusing to callers).
ALTER TABLE knievel.priorities
    ADD CONSTRAINT priorities_project_id_name_key
        UNIQUE (project_id, name);
