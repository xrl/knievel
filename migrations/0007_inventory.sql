-- Inventory chain — sites, zones.
--
-- Refs: API.md §§ 3.7–3.8, REQUIREMENTS.md § 5.
--
-- `aliases` is a Postgres text[]. Per API.md § 3.7 the canonical
-- url and every alias must be unique within the project across
-- both fields together; the handler enforces this at write time
-- by checking against a UNION of (url) and unnest(aliases). A
-- partial expression-index for the union is straightforward and
-- can land in 3.12 if write throughput needs it.

SET search_path TO knievel, public;

CREATE TABLE IF NOT EXISTS knievel.sites (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    channel_id  bigint,
    external_id text,
    name        text         NOT NULL,
    url         text         NOT NULL,
    aliases     text[]       NOT NULL DEFAULT '{}',
    is_active   boolean      NOT NULL DEFAULT true,
    etag        text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id),
    UNIQUE (project_id, url)
);
CREATE INDEX IF NOT EXISTS sites_project_id_idx ON knievel.sites (project_id);

ALTER TABLE knievel.sites ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.sites FORCE ROW LEVEL SECURITY;
CREATE POLICY sites_tenant ON knievel.sites
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));

CREATE TABLE IF NOT EXISTS knievel.zones (
    id          bigserial    PRIMARY KEY,
    org_id      text         NOT NULL REFERENCES knievel.organizations(id),
    project_id  text         NOT NULL REFERENCES knievel.projects(id),
    site_id     bigint       NOT NULL REFERENCES knievel.sites(id),
    external_id text,
    name        text         NOT NULL,
    is_active   boolean      NOT NULL DEFAULT true,
    etag        text         NOT NULL DEFAULT encode(gen_random_bytes(8), 'hex'),
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz  NOT NULL DEFAULT now(),
    UNIQUE (project_id, external_id)
);
CREATE INDEX IF NOT EXISTS zones_project_id_idx ON knievel.zones (project_id);
CREATE INDEX IF NOT EXISTS zones_site_id_idx ON knievel.zones (site_id);

ALTER TABLE knievel.zones ENABLE ROW LEVEL SECURITY;
ALTER TABLE knievel.zones FORCE ROW LEVEL SECURITY;
CREATE POLICY zones_tenant ON knievel.zones
    USING (project_id = current_setting('knievel.project_id', true))
    WITH CHECK (project_id = current_setting('knievel.project_id', true));
