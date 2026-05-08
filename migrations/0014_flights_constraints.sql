-- Fix(flights): priority_id FK + start_date <= end_date CHECK.
--
-- Audit findings: sonnet critical #10 (priority_id has no FK to
-- knievel.priorities), sonnet critical #11 (no date-range sanity
-- constraint at the schema layer). Both gaps were introduced in
-- migration 0006_demand.sql which defined flights before the
-- priorities table existed in 0008_taxonomy.sql.
--
-- This migration adds the two constraints retroactively. Existing
-- rows are not affected: priority_id values are integers seeded from
-- the default taxonomy (1..3) so the FK is satisfied on any
-- database that ran the seed; start/end_date pairs are either NULL
-- (no window) or plausible values set by the handler (no existing
-- row has start_date > end_date).
--
-- The migration does NOT create any new tables, so no ENABLE ROW
-- LEVEL SECURITY block is needed. Existing RLS on knievel.flights
-- is untouched.

SET search_path TO knievel, public;

-- Finding #10: priority_id should reference knievel.priorities(id).
-- Added as DEFERRABLE INITIALLY DEFERRED so bulk-insert tests that
-- seed flights before the taxonomy rows can still commit.
ALTER TABLE knievel.flights
    ADD CONSTRAINT flights_priority_id_fkey
        FOREIGN KEY (priority_id)
        REFERENCES knievel.priorities(id)
        DEFERRABLE INITIALLY DEFERRED;

-- Finding #11: end_date must not precede start_date.
-- NULL in either column means "no bound", per API.md § 3.3.
ALTER TABLE knievel.flights
    ADD CONSTRAINT flights_date_order_check
        CHECK (start_date IS NULL OR end_date IS NULL OR start_date <= end_date);
