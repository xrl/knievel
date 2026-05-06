-- Ad Library — org-scoped catalog of reusable creatives.
--
-- Refs:
--   API.md § 2.4, § 3.4
--   REQUIREMENTS.md § 5.1
--   PHASES.md task 3.28
--
-- Library items live at the org level (one catalog per org,
-- shared across that org's projects). Project Ads reference
-- items via `ads.ad_library_item_id` (column already reserved
-- in 0006_demand.sql so this migration is purely additive).

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.ad_library_items (
    id                text         PRIMARY KEY,
    org_id            text         NOT NULL REFERENCES knievel.organizations(id),
    external_id       text,
    name              text         NOT NULL,
    -- Same `kind` discriminator as project creatives; per-kind
    -- columns mirror the wire shape from API.md § 3.5.
    kind              text         NOT NULL CHECK (kind IN ('image', 'html', 'native')),
    image_url         text,
    width             int,
    height            int,
    alt               text,
    body              text,
    template_id       bigint,
    values            jsonb,
    click_through_url text,
    is_active         boolean      NOT NULL DEFAULT true,
    etag              text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at        timestamptz  NOT NULL DEFAULT now(),
    updated_at        timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (org_id, external_id)
);
CREATE INDEX IF NOT EXISTS ad_library_items_org_id_idx
    ON knievel.ad_library_items (org_id);

ALTER TABLE knievel.ad_library_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.ad_library_items FORCE ROW LEVEL SECURITY;
CREATE POLICY ad_library_items_tenant
    ON knievel.ad_library_items
    USING (org_id = current_setting('knievel.org_id', true))
    WITH CHECK (org_id = current_setting('knievel.org_id', true));
