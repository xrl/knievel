# RX → Knievel Migration Guide

How RX moves from Kevel to knievel without behavior change. Companion
to `REQUIREMENTS.md` and `API.md`; **not part of the platform spec**.
Knievel is a general-purpose ad platform; this document is one
consumer's mapping.

## Database Setup

Knievel runs against a dedicated schema inside RX's existing Aurora
Postgres cluster. No separate database, no separate cluster.

### One-time provisioning (per environment)

Run as a Postgres superuser (or via RX's IaC):

```sql
-- Extension (one-time, cluster-level).
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Dedicated schema and role.
CREATE SCHEMA knievel;
CREATE ROLE knievel_app LOGIN PASSWORD :'knievel_password';

GRANT USAGE, CREATE ON SCHEMA knievel TO knievel_app;

ALTER ROLE knievel_app SET search_path = knievel, public;
```

`knievel_app` has **no grants on RX's tables**. Defense in depth
against accidental joins or query bugs. Knievel does not require
`pg_partman` or any other non-vanilla extension — partition lifecycle
is managed in-process by a leader-elected tokio task using standard
Postgres declarative partitioning.

### Connection budget

Each knievel pod uses ~12 connections against the cluster:

- 1 long-lived `LISTEN` connection (writer endpoint, never reader).
- 1 long-lived advisory-lock connection (leader election; held by all
  pods, not just the current leader).
- 8 query-pool connections.
- 2 event-flusher `COPY` connections.

Two replicas → 24 connections. Coordinate with whoever owns the
Aurora pgbouncer / connection limits.

### Aurora endpoints

`KNIEVEL_DATABASE_URL` must point at the **cluster writer endpoint**.
`LISTEN/NOTIFY` does not propagate to readers, so a reader endpoint
silently breaks snapshot refresh. Knievel reconnects with backoff
across Aurora failovers automatically.

### Backup blast radius

`events_raw` partitions are part of RX's existing backup window. To
keep that bounded, knievel's **default events retention is 30 days**
(see `events.retention_days` in `config.yaml`). Bump only after
checking with whoever owns the backup story.

### Where the schema lives across environments

| RX environment | Cluster | Schema | Role |
|---|---|---|---|
| `production` | `rx-prod-aurora` | `knievel` | `knievel_app` |
| `staging` | `rx-staging-aurora` | `knievel` | `knievel_app` |

Same schema name in both; isolation comes from the cluster boundary.

### dbt access

The data science team's existing dbt pipeline reads from `public.*`
tables in the same Aurora clusters. Knievel's tables become
additional sources, joined freely with the existing models. Concrete
sources YAML, bronze/silver/gold examples, and snapshot patterns
live in `REPORTING.md`.

Knievel ships with a `knievel_reader` role for this purpose:

```sql
CREATE ROLE knievel_reader;
GRANT USAGE ON SCHEMA knievel TO knievel_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA knievel TO knievel_reader;
ALTER DEFAULT PRIVILEGES FOR ROLE knievel_app IN SCHEMA knievel
  GRANT SELECT ON TABLES TO knievel_reader;

-- Grant to RX's existing dbt service account.
GRANT knievel_reader TO dbt_service;
```

Point dbt at the Aurora **reader endpoint** for these queries —
keeps the writer's I/O budget focused on knievel's hot path. (Knievel
itself still uses the writer endpoint for `LISTEN/NOTIFY`.)

## Topology

| RX environment | Knievel Org | Knievel Projects |
|---|---|---|
| `production` | `scientist-com-prod` | one per RX Organization |
| `staging` | `scientist-com-staging` | one per RX Organization |

- One **knievel Org** per RX environment.
- One **knievel Project** per RX Organization (`az`, `pfizer`, …) —
  including the long tail of small marketplaces. Project provisioning
  is a single idempotent API call, so spinning up a new RX Organization
  costs one round-trip.
- One **Org Editor token** per environment, held by the Rails app.
  Used for both sync and decision calls. Project ID is supplied per
  call (`/v1/projects/{projectId}/...`).

## Concept Map

| RX | Kevel today | Knievel |
|---|---|---|
| Environment (`prod`, `staging`) | API key + `KEVEL_NETWORK_ID` | Org |
| RX Organization (marketplace) | (implicit; resolved by URL at decision time) | Project |
| Provider | Advertiser | Advertiser |
| `KevelAdConfiguration.campaign_name` | Campaign name | Campaign |
| `KevelAd` (one per ad) | 1 Flight + 1 Ad + 1 Creative | 1 Flight + 1 Ad + 1 Creative (orchestrated by gem helper) |
| `KevelAd.site_urls` | siteIds via URL lookup | (implicit per-project; sites scoped to the Project) |
| Provider published / Org archived (post-filter) | `blockedCreatives` | `block.creativeIds` |
| `AdConfiguration.priority_id` | Priority | Priority |
| `AdConfiguration.ad_type_id` | AdType | AdType |
| `AdConfiguration.zone_id` | Zone | Zone |
| `AdConfiguration.creative_template_id` | CreativeTemplate | CreativeTemplate |
| `current_organization.host` | site lookup → `siteId` | project routing in caller; optional `siteUrl` shorthand |

## Data Model Additions on the RX Side

Add to existing tables:

- `organizations.knievel_project_id` (string, nullable until backfilled).
- `providers.knievel_advertiser_id` (string) — analogous to existing
  `kevel_advertiser_id`; live in parallel during rollout.
- `kevel_ads.knievel_ad_id`, `knievel_flight_id`, `knievel_creative_id`,
  `knievel_campaign_id` — analogous to the existing `kevel_*` columns;
  live in parallel.

These columns mirror today's `kevel_*` shape so the rollout flag can
flip per-marketplace without losing the old IDs.

(Optional: rename later, once Kevel is decommissioned.)

## Sync Job Changes

`Kevel::SyncKevelRecordsFromAdJob` becomes
`Knievel::SyncKnievelRecordsFromAdJob`. Same trigger
(`KevelAd after_save`), same orchestration shape, different client.

### 1. Project provisioning (new step)

When an RX Organization is created — or first seen by the new sync —
upsert the Project:

```ruby
project = client.org("scientist-com-#{Rails.env}").projects.upsert(
  external_id: "rx_org:#{rx_org.id}",
  name:        rx_org.name
)
rx_org.update!(knievel_project_id: project.id)
```

Idempotent on `external_id`; safe to call repeatedly.

### 2. Default Site provisioning

Each RX Organization typically needs one Site representing the
marketplace itself, plus its zones. Create on first sync into a
Project:

```ruby
project_client = client.project(rx_org.knievel_project_id)
site = project_client.sites.upsert_by_url(
  url:  "https://#{rx_org.host}",
  name: rx_org.name
)
```

If the Organization has multiple hostnames, pass them via `aliases:` on
the upsert.

### 3. Provider → Advertiser

```ruby
advertiser = project_client.advertisers.upsert(
  external_id: "provider:#{provider.id}",
  name:        provider.name
)
provider.update!(knievel_advertiser_id: advertiser.id)
```

### 4. Campaign

```ruby
campaign = project_client.campaigns.upsert(
  external_id:   "advertiser:#{advertiser.id}:campaign:#{ad_config.campaign_name}",
  advertiser_id: advertiser.id,
  name:          ad_config.campaign_name
)
```

### 5. Flight + Ad + Creative

Today: three sequential round-trips. The Ruby gem ships a hand-rolled
helper that does all three in one call (no new wire endpoint — it just
orchestrates standard upserts):

```ruby
result = project_client.ads.upsert_with_flight_and_creative(
  external_id:   "kevel_ad:#{kevel_ad.id}",
  advertiser_id: advertiser.id,
  campaign_id:   campaign.id,
  flight: {
    external_id: "kevel_ad:#{kevel_ad.id}:flight",
    site_ids:    [site.id],            # the project's default site
    zone_ids:    [ad_config.zone_id],
    ad_types:    [ad_config.ad_type_id],
    priority_id: ad_config.priority_id,
    start_date:  kevel_ad.starts_at,
    end_date:    kevel_ad.ends_at
  },
  creative: {
    external_id:  "kevel_ad:#{kevel_ad.id}:creative",
    type:         :native,
    template_id:  ad_config.creative_template_id,
    values:       kevel_ad.dynamic_values
  },
  weight: 100
)

kevel_ad.update!(
  knievel_ad_id:       result.ad.id,
  knievel_flight_id:   result.flight.id,
  knievel_creative_id: result.creative.id,
  knievel_campaign_id: campaign.id
)
```

### 6. Multi-marketplace ads

The existing `type_option` enum has `subscription` (active),
`per_marketplace` (not wired), and `all_marketplaces` (not wired).

- `subscription` stays single-marketplace: one Project per RX
  Organization, sync runs in that Project only.
- When `all_marketplaces` is wired, sync iterates the org's Projects
  and upserts the same ad into each. Until then, no change.

## Decision Call Changes

`AdDecisionRequestsController` swaps `Kevel::Decision` for the gem:

```ruby
# Before
Kevel::Decision.new(
  network_id:        ENV["KEVEL_NETWORK_ID"],
  site_id:           Kevel::Site.find_by(url: "https://#{current_organization.host}").id,
  ad_types:          [...],
  zone_ids:          [...],
  blocked_creatives: blocked_creative_ids
).call

# After
client = Knievel::Client.new(token: ENV["KNIEVEL_ORG_TOKEN"])
client.project(current_organization.knievel_project_id).decisions.create(
  context: {
    url:        request.url,
    referrer:   request.referer,
    user_agent: request.user_agent
  },
  placements: [{
    id:       "main",
    site_url: "https://#{current_organization.host}",  # resolved server-side
    zone_ids: [...],
    ad_types: [...],
    count:    1
  }],
  block: { creative_ids: blocked_creative_ids }
)
```

The `blocked_creative_ids` computation
(`AdDecisionRequestsController#blocked_creative_ids` — unpublished
providers + archived organizations) **stays exactly as today**. It's
RX state, knievel doesn't model it.

## Authentication (Keycloak)

RX's Rails app authenticates to knievel via JWTs issued by RX's
existing Keycloak (`keycloak.scientist.com`). One Keycloak client per
knievel environment.

### Keycloak setup (per environment)

Create a confidential client in the appropriate realm (e.g. `scientist`):

- **Client ID**: `knievel-prod` (and `knievel-staging`).
- **Client authentication**: ON.
- **Service accounts roles**: ON.
- **Standard flow** / **direct access grants**: OFF.

Add two protocol mappers on the client:

1. **Audience mapper** — *Mapper Type: Audience*, included custom
   audience `knievel`, add to access token.
2. **Hardcoded claim mapper** — *Token Claim Name: `knievel`*, *Claim
   JSON Type: JSON*, *Claim value*:
   ```json
   {"scope": "org", "org_id": "scientist-com-prod", "role": "editor"}
   ```
   (substitute `scientist-com-staging` for staging).

The Rails app obtains tokens via standard OAuth2 client credentials:

```
POST https://keycloak.scientist.com/realms/scientist/protocol/openid-connect/token
grant_type=client_credentials
client_id=knievel-prod
client_secret=<from keycloak>
```

The returned access token (15-minute TTL by default) goes in the
`Authorization: Bearer ...` header on every knievel call. The Ruby gem
caches and refreshes automatically.

### Knievel-side config (Helm values)

```yaml
auth:
  modes: [jwt]
  jwt:
    issuers:
      # Keycloak: out-of-cluster service-to-service + future human OIDC.
      - issuer:    https://keycloak.scientist.com/realms/scientist
        audience:  knievel
        algorithms: [RS256]
        claim:     knievel

      # Kubernetes SA tokens: in-cluster pods (RX Rails app).
      - issuer:    https://kubernetes.default.svc.cluster.local
        audience:  knievel
        algorithms: [RS256]
        claim_mapping:
          rules:
            - match: { sub: system:serviceaccount:rx-prod:rx-rails }
              principal: { scope: org, org_id: scientist-com-prod, role: editor }
            - match: { sub: system:serviceaccount:rx-staging:rx-rails }
              principal: { scope: org, org_id: scientist-com-staging, role: editor }
```

No opaque tokens needed in steady state.

### Recommended path: Kubernetes SA tokens for the Rails app

RX runs on Kubernetes; the simplest zero-trust answer is to skip
Keycloak entirely for the in-cluster Rails app and use its own
ServiceAccount token as the Bearer credential to knievel.

In RX's deployment manifest (per environment):

```yaml
spec:
  serviceAccountName: rx-rails
  containers:
    - name: rails
      volumeMounts:
        - { name: knievel-token, mountPath: /var/run/secrets/knievel, readOnly: true }
  volumes:
    - name: knievel-token
      projected:
        sources:
          - serviceAccountToken:
              path: token
              audience: knievel
              expirationSeconds: 600
```

The Rails app reads `/var/run/secrets/knievel/token`, sends it as
`Authorization: Bearer …`, and re-reads on a timer (the kubelet
auto-rotates the file). No client_secret to rotate, no Keycloak hop
in the request path. See `AUTH.md` "Kubernetes ServiceAccount
Tokens" for the full picture.

Keycloak stays in play for human OIDC (admin UI, future) and for any
out-of-cluster integrations that need to talk to knievel.

## Local Development for RX Engineers

RX engineers run RX as a native `bin/rails server` process, with
knievel coming up from docker-compose alongside Postgres. Auth in
this setup is opaque-token-only — no Keycloak, no Kubernetes SA, no
IdP dependencies. The token is provisioned automatically on first
`docker compose up`.

### The compose pieces

In RX's repo (alongside the existing `compose.yaml` for Postgres,
Redis, etc.), add a knievel section:

```yaml
services:
  knievel-postgres:
    image: postgres:16
    environment:
      POSTGRES_USER:     knievel_app
      POSTGRES_PASSWORD: dev
      POSTGRES_DB:       knievel
    volumes:
      - knievel-pgdata:/var/lib/postgresql/data
      - ./dev/knievel-init.sql:/docker-entrypoint-initdb.d/00-init.sql:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U knievel_app -d knievel"]
      interval: 2s
      retries: 30

  knievel:
    image: ghcr.io/xrl/knievel:latest
    depends_on:
      knievel-postgres: { condition: service_healthy }
    environment:
      KNIEVEL_CONFIG: /etc/knievel/config.yaml
      DB_HOST: knievel-postgres
      DB_NAME: knievel
      DB_PASSWORD: dev
    volumes:
      - ./dev/knievel-config.yaml:/etc/knievel/config.yaml:ro
    ports: ["8080:8080"]
    healthcheck:
      test: ["CMD-SHELL", "curl -fsS http://localhost:8080/readyz"]
      interval: 2s
      retries: 30

  knievel-seed:
    image: ghcr.io/xrl/knievel:latest
    depends_on:
      knievel: { condition: service_healthy }
    entrypoint:
      - knievel-cli
      - seed-demo
      - --org-external-id=scientist-com-dev
      - --project-external-id=rx-org-dev
      - --token=kvl_dev_rx_local
      - --write-token-to=/out/knievel-dev-token
    environment:
      DB_HOST: knievel-postgres
      DB_NAME: knievel
      DB_PASSWORD: dev
    volumes:
      - ./tmp:/out
    restart: "no"

volumes:
  knievel-pgdata:
```

`dev/knievel-init.sql` runs once at first DB boot:

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE SCHEMA IF NOT EXISTS knievel AUTHORIZATION knievel_app;
ALTER ROLE knievel_app SET search_path = knievel, public;
```

`dev/knievel-config.yaml` (only the auth/dev-relevant bits; copy
the rest from `config.example.yaml`):

```yaml
api:
  bind_addr: 0.0.0.0:8080
  public_base_url: http://localhost:8080
database:
  url: postgres://knievel_app:${DB_PASSWORD}@${DB_HOST}/${DB_NAME}?sslmode=disable
  schema: knievel
  auto_migrate: true
auth:
  modes: [opaque]    # no Keycloak, no SA tokens in dev
errors:
  sentry: { enabled: false, dsn: "" }
tracing:
  otel: { enabled: false }
```

### Rails side

Add to RX's `.env.development`:

```
KNIEVEL_BASE_URL=http://localhost:8080
KNIEVEL_TOKEN_FILE=tmp/knievel-dev-token
```

The gem reads the token from disk at boot (and on
`SIGUSR1`-triggered re-reads, in case the seed re-ran). For
engineers who'd rather hardcode, set
`KNIEVEL_TOKEN=kvl_dev_rx_local` directly — the token's value is
fixed in the compose file via `--token=`.

`tmp/knievel-dev-token` should be gitignored.

### Common workflows

- **Wipe and start over:** `docker compose down -v knievel
  knievel-postgres && docker compose up`. The seed sidecar re-runs
  and re-mints the same token (because `--token=` is fixed in
  compose).
- **Update knievel:** `docker compose pull knievel && docker
  compose up -d`. Migrations run automatically on startup
  (`auto_migrate: true`).
- **Inspect ad-serving state:** `docker compose exec
  knievel-postgres psql -U knievel_app -d knievel` — connect
  directly with the dev role.
- **Reset just demo data:** `knievel-cli seed-demo --reset` rebuilds
  the demo content without touching auth state or migrations.

### Why no auth bypass for dev

Tempting to skip auth entirely in dev, easy to ship that flag
somewhere it shouldn't be. The opaque-token path is one
`docker compose up` and zero runtime ceremony for engineers; the
real auth code path runs every time. Cheap insurance.

### When you'd want Keycloak in the dev stack

Only when actively debugging the JWT path — claim-mapping rules,
protocol-mapper changes on the Keycloak side, OIDC discovery
behavior. For everyday RX feature work it's pure overhead. See
`AUTH.md` "Local Development" → "Testing the JWT path locally" for
how to layer Keycloak into the same compose stack.

## Configuration (RX side)

What RX's Rails app needs to set, to talk to knievel:

| Old (Kevel) | New (Knievel) |
|---|---|
| `KEVEL_API_KEY` | (replaced by Keycloak client credentials below) |
| `KEVEL_NETWORK_ID` | (replaced by per-call `projectId`) |
| `https://e-{network}.adzerk.net/api/v2` | `KNIEVEL_BASE_URL` (e.g. `https://ads.scientist.com`) |
| (none) | `KEYCLOAK_TOKEN_URL` (e.g. `https://keycloak.scientist.com/realms/scientist/protocol/openid-connect/token`) |
| (none) | `KNIEVEL_KC_CLIENT_ID` (`knievel-prod` / `knievel-staging`) |
| (none) | `KNIEVEL_KC_CLIENT_SECRET` |
| (none) | `KNIEVEL_ORG_EXTERNAL_ID` (e.g. `scientist-com-prod`) |

## Configuration (knievel deployment)

Knievel itself is configured via `config.yaml` rendered from the Helm
chart's `values.yaml`. RX's relevant Helm values for the deployment
within RX's cluster:

```yaml
database:
  host: rx-prod-aurora.cluster-xyz.us-east-1.rds.amazonaws.com
  port: 5432
  name: rx_production
  schema: knievel
  sslMode: require
  existingSecret: knievel-db
  maxConnections: 8

events:
  retentionDays: 30        # bounded for shared-cluster backups

sentry:
  enabled: true
  existingSecret: knievel-sentry
  environment: production

otel:
  enabled: true
  endpoint: http://otel-collector.observability:4317
  serviceName: knievel
  resourceAttributes:
    deployment.environment: production

api:
  publicBaseUrl: https://ads.scientist.com
```

## Rollout Strategy

Phased per-marketplace. Both clients (`Kevel::*` and `Knievel::*`)
coexist throughout.

1. **Stand up knievel in staging.** Provision Org
   `scientist-com-staging` and one pilot Project (a small staging
   marketplace).
2. **Sync writes go to both** during rollout. Add a feature flag
   `dual_write_knievel: true` to the sync job; it runs the existing
   Kevel sync and the new knievel sync in series. Errors on the knievel
   side log but don't fail the job.
3. **Backfill.** One-time job walks existing `KevelAd`s for the pilot
   marketplace and runs the knievel sync. Verify decisions match
   (golden-file diff against Kevel responses for a sample of placements).
4. **Per-marketplace decision flip.** Feature flag
   `use_knievel_for_decisions` on RX Organization. Flip for the pilot
   marketplace first; monitor latency, error rate, fill rate; expand.
5. **Production cutover.** Provision Org `scientist-com-prod`, run
   dual-write across all marketplaces, backfill, then flip decision
   flags marketplace-by-marketplace.
6. **Decommission Kevel.** Once all marketplaces are stable on knievel:
   stop dual-write, remove `Kevel::*` code, drop the `kevel_*` columns
   in a follow-up.

Rollback at any stage: flip the decision flag back. Sync continues to
both as long as dual-write is enabled.

## Call-Site Inventory (PR-Sized Chunks)

Approximate one PR per item:

1. Add `knievel_*` columns to `organizations`, `providers`, `kevel_ads`.
2. Vendor / install the `knievel-ruby` gem.
3. Implement `Knievel::SyncKnievelRecordsFromAdJob` (mirrors existing
   sync, no flag wiring yet).
4. Wire `dual_write_knievel` flag in the existing sync job.
5. Implement `Knievel::Decision` shim in `app/services/knievel/`.
6. Add `use_knievel_for_decisions` flag check in
   `AdDecisionRequestsController`.
7. Backfill rake task: walk existing `KevelAd`s, run sync, verify ID
   columns populated.
8. Backoffice updates: provider-facing screens read knievel IDs once
   populated, fall back to Kevel.
9. Per-environment ENV / secrets rollout.
10. Per-marketplace flag flip (one or more PRs as rollout progresses).
11. Decommission: remove `Kevel::*`, drop columns.

## What Doesn't Move to Knievel

These stay in RX because they're product/business logic, not ad-server
concerns:

- `KevelAd.type_option` enum — RX product surface.
- `Kevel::SubscriptionAdValidator` — RX business rule (per-provider
  marketplace caps).
- Provider/Organization scoping rules — RX auth model.
- `closest_advertisable_organization` fallback — RX-specific routing.
- The `blocked_creative_ids` computation — RX state (publish status,
  archival).

Knievel exposes the primitives (`block.creativeIds`, etc.) that let RX
keep all of the above on the RX side without knievel needing to know
about Providers, Organizations, subscriptions, or archival.
