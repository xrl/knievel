-- Initial migration.
--
-- The `knievel` schema and required extensions (`pgcrypto`) are
-- provisioned by the operator before knievel starts — see
-- `MIGRATION_RX.md` "One-time provisioning" and `REQUIREMENTS.md`
-- § 7.1. Migrations run as `knievel_app` with `search_path =
-- knievel, public`, so unqualified objects land in the knievel
-- schema (`REQUIREMENTS.md` § 7.7).
--
-- This migration creates the bookkeeping object every other
-- migration depends on:
--   - knievel.config_version  (REQUIREMENTS.md § 7.2)

SET search_path TO knievel, public;

-- Monotonic configuration version. Tenant-data mutations bump it;
-- the in-memory snapshot loader watches it via `LISTEN
-- config_changed` plus a 5 s poll backstop (REQUIREMENTS.md § 7.2).
--
-- Implemented as a SEQUENCE rather than a single-row table so the
-- migration linter's "every CREATE TABLE in knievel needs RLS"
-- rule (gate 2 of REQUIREMENTS.md § 7.1.1) does not trip on a
-- non-tenant bookkeeping object. Same semantics:
--
--   SELECT last_value FROM knievel.config_version;          -- read
--   SELECT nextval('knievel.config_version');               -- bump
CREATE SEQUENCE IF NOT EXISTS knievel.config_version
    AS bigint
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
