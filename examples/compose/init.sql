-- One-time DB provisioning for the compose stack.
--
-- Postgres runs files under /docker-entrypoint-initdb.d/ exactly
-- once when the data directory is first initialized. We use that
-- hook for the operator-equivalent steps from
-- `MIGRATION_RX.md` "One-time provisioning":
--
--   1. pgcrypto for `gen_random_uuid()`.
--   2. The `knievel` schema.
--   3. Default search_path on the application role so unqualified
--      DDL in migrations lands in `knievel`, not `public`.
--
-- The application binary's `database.auto_migrate: true` then runs
-- the rest. Migrations are idempotent — calling them again on a
-- pre-provisioned cluster is a no-op.

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE SCHEMA IF NOT EXISTS knievel AUTHORIZATION knievel_app;
ALTER ROLE knievel_app SET search_path = knievel, public;
