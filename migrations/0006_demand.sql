-- Demand chain — advertisers, campaigns, flights, ads,
-- creatives, creative_templates.
--
-- Refs:
--   API.md §§ 3.1–3.6
--   REQUIREMENTS.md § 5 (domain model), § 7.1 (schema/isolation)
--
-- All tables are project-scoped: they carry both `org_id` and
-- `project_id`, and RLS policies bind on
-- `current_setting('knievel.project_id')`. The org_id duplicate
-- on every row makes downstream reporting joins (REPORTING.md)
-- cheap and sidesteps a chain of FKs to recover the org.
--
-- v0 schema is minimal; targeting / day-parting / freq-cap
-- columns land per REQUIREMENTS.md § 11 roadmap.

SET search_path TO knievel, public;

-- Advertisers ----------------------------------------------------
CREATE TABLE IF NOT EXISTS knievel.advertisers (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    external_id text,
    name        text         NOT NULL,
    is_active   boolean      NOT NULL DEFAULT true,
    etag        text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id)
);
CREATE INDEX IF NOT EXISTS advertisers_project_id_idx
    ON knievel.advertisers (project_id);

ALTER TABLE knievel.advertisers ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.advertisers FORCE ROW LEVEL SECURITY;
CREATE POLICY advertisers_tenant ON knievel.advertisers
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Campaigns ------------------------------------------------------
CREATE TABLE IF NOT EXISTS knievel.campaigns (
    id            bigserial    PRIMARY KEY,
    org_id        text         NOT NULL REFERENCES knievel.organizations(id),
    project_id    text         NOT NULL REFERENCES knievel.projects(id),
    advertiser_id bigint       NOT NULL REFERENCES knievel.advertisers(id),
    external_id   text,
    name          text         NOT NULL,
    is_active     boolean      NOT NULL DEFAULT true,
    etag          text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at    timestamptz  NOT NULL DEFAULT now(),
    updated_at    timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id)
);
CREATE INDEX IF NOT EXISTS campaigns_project_id_idx
    ON knievel.campaigns (project_id);
CREATE INDEX IF NOT EXISTS campaigns_advertiser_id_idx
    ON knievel.campaigns (advertiser_id);

ALTER TABLE knievel.campaigns ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.campaigns FORCE ROW LEVEL SECURITY;
CREATE POLICY campaigns_tenant ON knievel.campaigns
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Creative Templates --------------------------------------------
-- The `schema` column is an arbitrary JSON Schema document. Round-
-- trip through poem-openapi is the cross-cutting risk #1 — spike
-- in Phase 3.10 before the handler lands.
CREATE TABLE IF NOT EXISTS knievel.creative_templates (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    external_id text,
    name        text         NOT NULL,
    schema      jsonb        NOT NULL,
    version     int          NOT NULL DEFAULT 1,
    etag        text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id),
    UNIQUE (project_id, name)
);
CREATE INDEX IF NOT EXISTS creative_templates_project_id_idx
    ON knievel.creative_templates (project_id);

ALTER TABLE knievel.creative_templates ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.creative_templates FORCE ROW LEVEL SECURITY;
CREATE POLICY creative_templates_tenant ON knievel.creative_templates
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Creatives ------------------------------------------------------
-- `kind` is the discriminator for the API.md § 3.5 oneOf:
--   'image' / 'html' / 'native'.
-- Type-specific columns are nullable; the handler enforces shape
-- consistency at write time.
CREATE TABLE IF NOT EXISTS knievel.creatives (
    id                  bigserial    PRIMARY KEY,
    org_id              text         NOT NULL REFERENCES knievel.organizations(id),
    project_id          text         NOT NULL REFERENCES knievel.projects(id),
    advertiser_id       bigint       NOT NULL REFERENCES knievel.advertisers(id),
    external_id         text,
    name                text,
    kind                text         NOT NULL CHECK (kind IN ('image', 'html', 'native')),
    image_url           text,
    width               int,
    height              int,
    alt                 text,
    body                text,
    template_id         bigint       REFERENCES knievel.creative_templates(id),
    values              jsonb,
    click_through_url   text,
    is_active           boolean      NOT NULL DEFAULT true,
    etag                text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at          timestamptz  NOT NULL DEFAULT now(),
    updated_at          timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id)
);
CREATE INDEX IF NOT EXISTS creatives_project_id_idx
    ON knievel.creatives (project_id);
CREATE INDEX IF NOT EXISTS creatives_advertiser_id_idx
    ON knievel.creatives (advertiser_id);

ALTER TABLE knievel.creatives ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.creatives FORCE ROW LEVEL SECURITY;
CREATE POLICY creatives_tenant ON knievel.creatives
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Flights --------------------------------------------------------
-- `site_ids` / `zone_ids` empty array = "any in the project"
-- (API.md § 3.3). `ad_types` is required and non-empty (handler
-- enforces non-empty; schema permits empty for migration safety).
CREATE TABLE IF NOT EXISTS knievel.flights (
    id            bigserial    PRIMARY KEY,
    org_id        text         NOT NULL REFERENCES knievel.organizations(id),
    project_id    text         NOT NULL REFERENCES knievel.projects(id),
    campaign_id   bigint       NOT NULL REFERENCES knievel.campaigns(id),
    external_id   text,
    name          text         NOT NULL,
    priority_id   bigint       NOT NULL,
    start_date    timestamptz,
    end_date      timestamptz,
    site_ids      bigint[]     NOT NULL DEFAULT '{}',
    zone_ids      bigint[]     NOT NULL DEFAULT '{}',
    ad_types      bigint[]     NOT NULL DEFAULT '{}',
    is_active     boolean      NOT NULL DEFAULT true,
    etag          text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at    timestamptz  NOT NULL DEFAULT now(),
    updated_at    timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id)
);
CREATE INDEX IF NOT EXISTS flights_project_id_idx
    ON knievel.flights (project_id);
CREATE INDEX IF NOT EXISTS flights_campaign_id_idx
    ON knievel.flights (campaign_id);

ALTER TABLE knievel.flights ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.flights FORCE ROW LEVEL SECURITY;
CREATE POLICY flights_tenant ON knievel.flights
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

-- Ads ------------------------------------------------------------
-- API.md § 3.4 oneOf: an Ad EITHER inlines a project-scoped
-- creative (creative_id) OR references an org-scoped Ad Library
-- item (ad_library_item_id). The `kind_check` constraint enforces
-- exactly one is set at the schema layer; the handler also
-- validates at write time. Library reference column is nullable
-- here so 3.28 (Ad Library) lands additively.
CREATE TABLE IF NOT EXISTS knievel.ads (
    id                  bigserial    PRIMARY KEY,
    org_id              text         NOT NULL REFERENCES knievel.organizations(id),
    project_id          text         NOT NULL REFERENCES knievel.projects(id),
    flight_id           bigint       NOT NULL REFERENCES knievel.flights(id),
    creative_id         bigint       REFERENCES knievel.creatives(id),
    ad_library_item_id  text,
    external_id         text,
    weight              int          NOT NULL DEFAULT 100,
    is_active           boolean      NOT NULL DEFAULT true,
    etag                text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at          timestamptz  NOT NULL DEFAULT now(),
    updated_at          timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id),
    CONSTRAINT ads_kind_check CHECK (
        (creative_id IS NOT NULL AND ad_library_item_id IS NULL)
        OR (creative_id IS NULL AND ad_library_item_id IS NOT NULL)
    )
);
CREATE INDEX IF NOT EXISTS ads_project_id_idx ON knievel.ads (project_id);
CREATE INDEX IF NOT EXISTS ads_flight_id_idx ON knievel.ads (flight_id);

ALTER TABLE knievel.ads ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.ads FORCE ROW LEVEL SECURITY;
CREATE POLICY ads_tenant ON knievel.ads
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));
