-- HMAC rotation overlap (REQUIREMENTS.md § 6.3, Phase 3.16).
--
-- During the 8-hour overlap window after a secret rotation, both
-- the previous and current secret verify already-minted
-- impression/click URLs. After 8h, only the new secret is
-- accepted.
--
-- `hmac_secret` (the current/active secret) was provisioned in
-- 0002_tenants.sql. This migration adds:
--   - hmac_secret_previous: the previous secret, kept around for
--     the overlap window. NULL when no rotation in flight.
--   - hmac_secret_rotated_at: when the previous secret was
--     promoted out. NULL = no rotation history. The handler
--     clears `_previous` once now() > rotated_at + 8h.
--
-- These columns participate in the same RLS policies as the
-- parent table (no new policy needed — projects_tenant_isolation
-- already covers the table). No data backfill needed: NULL means
-- "no rotation in flight" which matches every existing row.

SET search_path TO knievel, public;

ALTER TABLE knievel.projects
    ADD COLUMN IF NOT EXISTS hmac_secret_previous   bytea,
    ADD COLUMN IF NOT EXISTS hmac_secret_rotated_at timestamptz;
