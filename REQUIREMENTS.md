# Knievel Requirements

**Tagline:** Fearlessly fast ad delivery that steals the show.

Knievel is a Rust ad-serving platform inspired by [Kevel](https://dev.kevel.com)'s
domain model, with its own clean OpenAPI-defined wire format. It targets
multi-tenant deployments — one process can host many isolated workspaces —
and ships a generated client library alongside the server so calling apps
speak the API through real types rather than hand-rolled HTTP.

This document is the working spec. Wrong in places — we iterate.

## 1. Goals

1. Deliver a clean OpenAPI 3.1 surface for ad serving + management. The spec
   is the contract; both the Rust server and the generated client libraries
   are derived from it.
2. Native multi-tenancy: one binary serves many isolated Projects without
   per-tenant infrastructure.
3. Sub-millisecond p50 decision latency on a single node.
4. Statically-linked Rust binary + Postgres. No second datastore in v0.
5. A foundation that grows into the broader Kevel feature surface (UserDB,
   geo, frequency capping, auctions, reporting) without rewrites.
6. Minimal operator burden — a small team should be able to run knievel
   without specialized ops staff.

## 2. Non-Goals (v0)

- Kevel wire compatibility. Drop-in compatibility for existing integrations
  is the job of generated client libraries, not the server.
- Browser-direct ad calls. Knievel assumes a trusted server-to-server
  caller in v0; browser-direct is a v1+ deployment mode.
- UserDB / per-user behavioral targeting.
- Geo, IP, lat/long, radius, day-parting, keyword, custom-property
  targeting.
- Frequency capping, eCPM auctions, second-price clearing, pricing data.
- Reporting API surface (callers track from impression/click pings in v0).
- Web admin UI. CLI + API only.
- Header bidding, OpenRTB, DSP/SSP integration.

These are explicitly future work, not "never." See §11.

## 3. Architecture

```
┌──────────────┐     POST /v1/projects/{p}/decisions    ┌──────────────┐
│ Calling app  │ ──────────────────────────────────────▶│   knievel    │
│ (via gen'd   │ ◀──────────────────────────────────────│ (Rust/poem)  │
│  client lib) │                                         └──────┬───────┘
└──────────────┘                                                │
       ▲                                                in-mem  │
       │                                              snapshot  ▼
       │                                                  ┌──────────────┐
       └─ impression/click pings ─────────────────────────▶│   Postgres   │
          GET /e/i/<sig>                                   │  (config +   │
          GET /e/c/<sig>                                   │  partitioned │
                                                           │   events)    │
                                                           └──────────────┘
```

- **Web framework:** [`poem`](https://github.com/poem-web/poem) +
  [`poem-openapi`](https://docs.rs/poem-openapi/). Handlers and request/
  response types are annotated; the OpenAPI spec is generated from the
  binary and exposed at `GET /openapi.json`.
- **Datastore:** Postgres-native. Knievel targets a dedicated **schema**
  inside operator-supplied Postgres (Aurora-compatible). The `pg_partman`
  extension is a hard requirement; no portable fallback. See §7.
- **Hot path:** the configuration snapshot lives in process memory, keyed
  by `(project_id, resource)`, refreshed on change notification. Decision
  requests touch RAM only.
- **Event path:** decision/impression/click events are buffered in an
  in-process channel and `COPY`'d to the partitioned events table in
  batches every 1–2 s. Postgres routes rows to the correct partition.
- **Observability:** structured JSON logs via `tracing`, OpenTelemetry
  spans exported via OTLP, pervasive Sentry error reporting. All three
  carry the same `request_id` / `trace_id` for correlation. See §10.
- **Configuration:** layered — built-in defaults, then a `config.yaml`
  file with `${VAR}` env-interpolation, then individual env-var
  overrides. See §10.1.

## 4. Multi-Tenancy

Knievel uses a two-level tenant hierarchy:

- **Organization** — billing entity, user roster, owns API tokens that may
  span its Projects.
- **Project** — an isolated ad-serving workspace. Has its own Advertisers,
  Campaigns, Flights, Ads, Creatives, Sites, Zones, taxonomies. Hard
  isolation between Projects in the same Org.

A single-tenant deployment is just one Org with one Project — the same
shape Kevel calls a "Network."

### 4.1 Common deployment patterns

Knievel supports — and stays ergonomic across — three shapes:

- **Single-project deployment.** One publisher, one Org, one Project. The
  simplest shape; analogous to one Kevel Network.
- **Project-per-environment.** `prod`, `staging`, `dev` as sibling Projects
  under one Org. A single Org token can address all three.
- **Project-per-tenant.** A multi-tenant calling app spins up one Project
  per end customer. Hundreds of small Projects are fine; provisioning is
  a single idempotent API call.

Knievel does not pick one for you.

### 4.2 Integration shape

Knievel is designed for **server-to-server** calls from a trusted upstream
(the calling app). Browsers do not call the Decision API directly in v0;
they only hit the public event endpoints (`/e/...`).

Why proxy-first:

- **Ad-blocker resistance.** Decision requests originate from the calling
  app's own infrastructure, not a known ad-network domain.
- **Server-side enrichment.** The calling app can enrich the request with
  trusted context (auth state, A/B bucket, subscription tier).
- **Caller-driven post-filtering** via `block.creativeIds` / etc., for
  state knievel doesn't model (e.g., "this listing was just unpublished").
- **Simpler auth.** One Bearer token, not per-browser bot filtering +
  signed requests + CORS preflights.

Concretely:

- Decision and Management endpoints take `Authorization: Bearer <token>`.
- Plain `application/json`. No `text/plain` workaround.
- CORS off by default. Browser-direct mode (CORS, anonymous decision,
  bot filtering) is a v1+ feature.
- Impression/click pings are GETs that browsers hit directly; their
  signatures are HMAC-minted at decision time.

### 4.3 Authentication & authorization

**Token types:**

- **Org token** — scoped to an Org and a role. May address any Project
  within that Org via `/v1/projects/{projectId}/...`. The calling app's
  primary credential.
- **Project token** — scoped to a single Project and a role. For
  per-tenant access (the eventual admin UI; per-customer integrations).

**Roles:**

| Role | Scope | Capabilities |
|---|---|---|
| Org Owner | Org | Manage org, billing, projects, members, all tokens. |
| Org Admin | Org | Same minus billing and ownership transfer. |
| Project Admin | Project | Full CRUD on resources; manage project members and tokens. |
| Project Editor | Project | CRUD on Advertisers/Campaigns/Flights/Ads/Creatives/Templates/Sites/Zones. The integration role. |
| Project Reader | Project | `GET` everything in the project, including issuing decisions. |

Org tokens carry a project-level role applied to every project they
address (typically `org-admin` ⇒ Project Admin, or `org-editor` ⇒
Project Editor).

Token format: `kvl_<env>_<scope>_<random>` (e.g.
`kvl_prod_org_AbCd_8f2a...`). Stored argon2id-hashed; never recoverable
after creation. Revocable. Last-used timestamp tracked.

### 4.4 Site Group scoping (roadmap)

A future Site Group entity will let Project members and tokens be scoped
to a subset of Sites within a Project — for cases where multiple
sub-tenants share a Project but should be admin-isolated from each
other. Not v0; called out so the data model leaves room for it.

## 5. Domain Model

```
Organization                                  ← billing, users
  └── Project                                  ← isolated workspace
        ├── Advertiser → Campaign → Flight → Ad → Creative
        ├── Channel → Site → Zone              ← inventory
        ├── Priority                           ← waterfall tier
        ├── AdType                             ← format/size identifier
        └── CreativeTemplate                   ← native-ad value schema
```

The inventory chain (Channel → Site → Zone) and demand chain (Advertiser →
Campaign → Flight → Ad → Creative) are unchanged from Kevel; only the
top-level grouping is renamed (Project) and bumped under an Org.

Every persistable entity carries:

| Field | Type | Notes |
|---|---|---|
| `id` | string | Server-assigned, project-scoped (or org-scoped for orgs/projects). |
| `externalId` | string \| null | Caller-assigned, unique within `(project, resource)`. |
| `etag` | string | Opaque concurrency token for `If-Match`. |
| `createdAt` | RFC 3339 | |
| `updatedAt` | RFC 3339 | |
| `isActive` | bool | Soft-delete via `isActive: false`; v0 has no hard delete. |

Sites additionally carry:

| Field | Type | Notes |
|---|---|---|
| `url` | string | Unique within the project (across `url` + `aliases`). |
| `aliases` | string[] | Additional URLs that resolve to this site. |

The Decision API can resolve a placement's site by `siteId`, `siteUrl`,
or `siteExternalId`. URL lookup matches `url` and `aliases`.

## 6. API Surface (v0)

OpenAPI 3.1, served at `/openapi.json`. Bearer auth. Cursor pagination.
`Idempotency-Key` documented as part of the spec. Path-prefixed:

- `/v1/orgs/{orgId}/...` — org-level operations (provisioning, tokens,
  members).
- `/v1/projects/{projectId}/...` — everything else (resources, decisions).
- `/e/...` — public event tracking (HMAC-signed, no auth).
- `/healthz`, `/readyz`, `/metrics`, `/openapi.json`, `/version` — system.

Full endpoint list, request/response shapes, and conventions live in
`API.md`.

### 6.1 Decision API summary

`POST /v1/projects/{projectId}/decisions`

```json
{
  "context": {
    "url":       "https://example.com/article/42",
    "referrer":  "https://www.google.com/...",
    "userAgent": "Mozilla/5.0 ..."
  },
  "placements": [
    {
      "id":      "header",
      "siteUrl": "https://example.com",
      "zoneIds": [34],
      "adTypes": [16],
      "count":   1
    }
  ],
  "block": {
    "creativeIds":   [],
    "advertiserIds": [],
    "campaignIds":   []
  }
}
```

Selection algorithm:

1. Filter to flights active at request time (date window).
2. Filter to ads matching `siteId`/`zoneIds`/`adTypes`.
3. Apply `force.*` overrides (debug only; not for production traffic).
4. Apply `block.*` exclusions (caller-derived state).
5. Group by priority tier; highest non-empty tier wins.
6. Within tier: weighted random by ad weight.
7. Mint HMAC-signed click and impression URLs.

`decisions[<id>]` is always an array. Empty = no eligible ad. `creative`
is a typed `oneOf` (image / html / native).

`context` fields are informational: stored on event rows, available to
future URL-pattern targeting; never used for tenant routing (the project
ID in the path is the only authoritative tenant signal).

`block` is the post-filter exclusion set. Knievel doesn't model the
caller's domain (subscription state, archival, etc.); the caller computes
the blocklist and passes it.

### 6.2 Management API summary

Full CRUD plus bulk-upsert for the demand and inventory chains:

| Resource | Endpoints |
|---|---|
| Advertiser | CRUD + `:batchUpsert` |
| Campaign | CRUD + `:batchUpsert` |
| Flight | CRUD + `:batchUpsert` |
| Ad | CRUD + `:batchUpsert` |
| Creative | CRUD + image upload |
| CreativeTemplate | CRUD |
| Site | CRUD + `:batchUpsert` + `:upsertByUrl` |
| Zone | CRUD + `:batchUpsert` |

Read-only inventory taxonomy (per-project but rarely changed):

| Resource | Endpoints |
|---|---|
| Channel | List, Get |
| Priority | List, Get |
| AdType | List, Get |

All write endpoints accept `Idempotency-Key`. Bulk upserts are atomic
per batch and keyed on `externalId`. Sites' `:upsertByUrl` is a
first-class natural-key endpoint for URL-driven flows.

### 6.3 Event tracking

- `GET /e/i/{signed}` — impression. `204 No Content` (or 1×1 GIF if
  `?fmt=gif`).
- `GET /e/c/{signed}` — click. `302` to the creative's
  `clickThroughUrl`.

HMAC-SHA256 signatures over `(project_id, ad_id, creative_id,
placement_id_hash, issued_at, nonce)` with a per-project secret. TTL
configurable per project (default 24 h).

## 7. Storage

Knievel is Postgres-native. It targets a single schema (default
`knievel`) inside operator-supplied Postgres, and depends on the
`pg_partman` extension for partition lifecycle management.
Aurora Postgres is a first-class supported deployment; both
`pg_partman` and `pgcrypto` are available there.

### 7.1 Schema and isolation

All knievel tables live in a single schema, default name `knievel`,
configurable per deployment. The schema is owned by a dedicated
Postgres role (`knievel_app`) with grants only on its own schema:

```sql
CREATE SCHEMA knievel;
CREATE ROLE knievel_app LOGIN PASSWORD '...';
GRANT USAGE, CREATE ON SCHEMA knievel TO knievel_app;
ALTER ROLE knievel_app SET search_path = knievel, public;
```

Knievel migrations only touch its own schema. The `_sqlx_migrations`
tracking table lives in `knievel` too.

Knievel runs against the cluster **writer endpoint** in shared-Aurora
deployments. `LISTEN/NOTIFY` does not propagate to readers; the
snapshot loader needs the writer. Reconnect-with-backoff handles
Aurora failovers.

All entities carry `(org_id, project_id)` columns. Isolation is
enforced at the query layer and via Postgres row-level security
policies (defense in depth).

### 7.2 Configuration store

Source of truth for all knievel-managed entities. Mutated only via the
Management API. The snapshot loader subscribes via `LISTEN/NOTIFY` to a
`config_changed` channel; on notify it pulls diffs and atomically swaps
the in-memory snapshot. Cold-start hydration is one query per table.

The in-memory snapshot is keyed by `(project_id, resource)` so a single
process can serve thousands of small Projects efficiently.

### 7.3 Events

Two tables:

- **`events_raw`** — append-only, range-partitioned by day on `ts`,
  managed by `pg_partman`. Columns: `ts`, `org_id`, `project_id`,
  `kind` (`decision` | `impression` | `click`), `placement_id`,
  `site_id`, `zone_id`, `ad_id`, `creative_id`, `flight_id`,
  `campaign_id`, `advertiser_id`, `url`, `referrer_host`,
  `user_agent_hash`, `signature_nonce`, `dedup_key`. **Default
  retention 30 days** (conservative because backups are the operator's
  responsibility in shared-DB deployments); operator-configurable.
- **`events_rollup`** — hourly aggregates by `(project_id, site_id,
  zone_id, flight_id, ad_id, creative_id, kind)`. Computed by a
  periodic job before raw partitions age out. Indefinite retention.

`pg_partman` configuration:

- `p_premake => 4` — always keep 4 days of future partitions ready.
- No default partition. A failed COPY due to a missing partition is a
  loud signal that maintenance is broken; we want the alert, not a
  silent catch-all.
- Partition maintenance runs **in-process** via a tokio task that
  calls `partman.run_maintenance('knievel.events_raw')` hourly. Avoids
  putting cron jobs into the operator's database.

### 7.4 Write path

Per-request DB I/O is forbidden on the hot path. Events go to a
bounded `tokio::sync::mpsc` channel. A flusher task drains every 1–2 s
(or 5 k events, whichever first) and `COPY`s into the parent
`knievel.events_raw` table — Postgres routes each row to the
appropriate partition by `ts`. Channel saturation surfaces as `503` on
the decision endpoint rather than silent loss. Failures are reported
to Sentry with the batch size and offending row sample.

### 7.5 Migrations

Plain SQL files in `migrations/`, embedded into the binary via
`sqlx::migrate!()`. Run on startup behind `KNIEVEL_AUTO_MIGRATE=true`,
or via `knievel-cli migrate`. All files start with
`SET search_path TO knievel, public;` so migrations are
schema-targeted regardless of session defaults.

### 7.6 Connection budget

Knievel competes with the host application for the cluster's
connection pool. Per knievel instance, default budget:

- **1** long-lived `LISTEN` connection (writer endpoint).
- **8** query pool connections (sqlx default), env-overridable.
- **2** dedicated event-flusher connections for `COPY`.

Total ≈ **11** per instance. Operators with pgbouncer in front of
Aurora should size accordingly.

### 7.7 What's deferred

- **Redis** — only needed when frequency capping or per-user pacing
  ships.
- **TimescaleDB / ClickHouse** — escape hatches if/when partitioned
  Postgres stops keeping up. Not v0.

## 8. Deliverables

1. **`knievel`** Rust binary — server, snapshot loader, event flusher,
   partition maintenance task, migrations.
2. **`openapi.yaml`** — generated from the binary by `cargo xtask
   openapi`, committed to the repo, served at `/openapi.json`.
3. **Generated client libraries** — at minimum a Ruby gem
   (`knievel-ruby`) generated via `openapi-generator-cli` in CI,
   published on tag. Other languages on demand.
4. **`knievel-cli`** — admin CLI for project provisioning, token
   rotation, snapshot inspection, migration replay. Shares the OpenAPI
   client.
5. **Container image** — minimal distroless or `gcr.io/distroless/cc`
   based, multi-arch (`amd64` + `arm64`), published on tag.
6. **Helm chart** (`charts/knievel`) — first-class deployment artifact,
   not an afterthought. See §8.1.
7. **Compose manifest** — single-binary + bring-your-own-Postgres for
   local development and reference single-node deployments.

Per-consumer migration guides (e.g., `MIGRATION_RX.md`) live alongside
the spec but are not part of the v0 platform deliverable surface.

### 8.1 Helm chart

`values.yaml` exposes a high-level idiomatic shape; the chart's
templates render those values into a `config.yaml` and mount it as a
ConfigMap. The Deployment carries a `checksum/config` annotation
computed from the rendered ConfigMap so any values change rolls the
pods.

Sketch of the values surface:

```yaml
image:
  repository: ghcr.io/xrl/knievel
  tag: ""               # defaults to chart appVersion
  pullPolicy: IfNotPresent

replicaCount: 2

resources:
  requests: { cpu: 100m, memory: 256Mi }
  limits:   { cpu: 2,    memory: 1Gi }

database:
  # Aurora cluster writer endpoint.
  host: ""              # required
  port: 5432
  name: ""              # required
  schema: knievel
  sslMode: require
  existingSecret: ""    # holds username + password
  userKey: username
  passwordKey: password
  maxConnections: 8

events:
  retentionDays: 30
  flushIntervalMs: 1000
  flushBatchSize: 5000

hmac:
  existingSecret: ""    # default per-project HMAC key bootstrap
  key: hmac-default

sentry:
  enabled: true
  existingSecret: ""    # holds DSN
  dsnKey: dsn
  environment: ""       # defaults to .Release.Namespace
  tracesSampleRate: 0.0 # OTel handles tracing; Sentry for errors only
  release: ""           # defaults to image.tag

otel:
  enabled: true
  endpoint: ""          # OTLP gRPC, e.g. http://otel-collector:4317
  serviceName: knievel
  resourceAttributes: {} # service.namespace, deployment.environment, etc.

logging:
  level: info
  format: json
  decisionsSampleRate: 0.01

api:
  publicBaseUrl: ""     # used in minted impression/click URLs
  bindAddr: 0.0.0.0:8080

ingress:
  enabled: false
  className: nginx
  annotations: {}
  hosts: []
  tls: []

service:
  type: ClusterIP
  port: 80

serviceMonitor:
  enabled: true
  interval: 30s

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 65532
  fsGroup: 65532

securityContext:
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities: { drop: [ALL] }

nodeSelector: {}
tolerations: []
affinity: {}
```

Templates render values into a single mounted file at
`/etc/knievel/config.yaml`. Secrets (DSNs, DB passwords) are resolved
via `existingSecret` references and projected as env vars; the
`config.yaml` references them via `${VAR}` interpolation rather than
embedding their plaintext.

Key chart conventions:

- `checksum/config` pod annotation = `sha256sum` of the rendered
  ConfigMap. Helm chart upgrade with new values triggers a rollout.
- Resource names follow `{{ include "knievel.fullname" . }}`; standard
  Helm labels (`app.kubernetes.io/...`) on every resource.
- Optional `ServiceMonitor` for Prometheus Operator users.
- `helm template` output validated in CI with `helm lint` and
  `kubeconform`.

## 9. Performance Targets

Single node, 4 vCPU / 8 GB RAM, 100 k active flights:

- p50 decision latency ≤ **1 ms** (1 placement, no overrides).
- p99 decision latency ≤ **10 ms** (4 placements).
- Sustained throughput ≥ **20 000 decisions/sec** before saturating one
  core.
- Cold-start to first decision served ≤ **2 s**.
- Event flusher keeps end-to-end ingest lag ≤ **3 s** at peak.

Starting numbers; we measure and adjust.

## 10. Operational

### 10.1 Configuration

Layered, in order of precedence (later wins):

1. Built-in defaults (compiled in).
2. `config.yaml` (path from `KNIEVEL_CONFIG`, default
   `/etc/knievel/config.yaml`).
3. Individual env-var overrides (`KNIEVEL_*`).

The `config.yaml` is preprocessed for `${VAR}` and `${VAR:default}`
interpolation before parse, so secrets injected as env vars (DB
password, Sentry DSN, HMAC key) can be referenced inline without
templating tools. `${VAR}` with an unset `VAR` and no default is a
hard error at startup.

`config.yaml` schema (illustrative; full schema lives in code +
`config.example.yaml`):

```yaml
api:
  bind_addr: 0.0.0.0:8080
  public_base_url: https://ads.example.com

database:
  url: postgres://knievel_app:${DB_PASSWORD}@${DB_HOST}/${DB_NAME}?sslmode=require
  schema: knievel
  max_connections: 8
  flusher_connections: 2
  auto_migrate: false

events:
  retention_days: 30
  flush_interval_ms: 1000
  flush_batch_size: 5000

hmac:
  default_secret: ${KNIEVEL_HMAC_DEFAULT_SECRET}

logging:
  level: info
  format: json
  decisions_sample_rate: 0.01

tracing:
  otel:
    enabled: true
    endpoint: ${OTEL_EXPORTER_OTLP_ENDPOINT}
    service_name: knievel
    resource_attributes:
      deployment.environment: ${KNIEVEL_ENV:production}

errors:
  sentry:
    enabled: true
    dsn: ${SENTRY_DSN:}
    environment: ${KNIEVEL_ENV:production}
    release: ${KNIEVEL_RELEASE:}
    sample_rate: 1.0
```

Loading is done with `figment` layering the three sources. Empty
`sentry.dsn` is permitted and disables the integration — keeps dev
runs from needing a Sentry project.

### 10.2 Logging

Structured JSON via `tracing` + `tracing-subscriber::fmt::json()`.
Every line includes `timestamp`, `level`, `target`, `message`,
`request_id`, and (when in a span) `trace_id` + `span_id` from the
OTel context.

Per-context fields attached at request entry:

- `org_id`, `project_id`, `route`, `method`, `status`, `duration_ms`
- For decision requests: `placement_count`, `decision_outcome` (hit /
  miss / blocked / forced)
- For management writes: `resource`, `external_id`, `idempotent_replay`

Decision endpoint logs are sampled at `decisions_sample_rate` (default
1%); errors and 4xx/5xx always log fully, regardless of sample.

### 10.3 Distributed tracing (OpenTelemetry)

`opentelemetry` + `opentelemetry-otlp` (gRPC) + `opentelemetry-sdk`
+ `tracing-opentelemetry` bridging from `tracing`. Spans:

- One per HTTP request, with HTTP semantic conventions.
- One per snapshot refresh cycle.
- One per event flush batch (with `batch_size` and `partition_count`
  attributes).
- One per outbound DB call (via `sqlx` instrumentation).

Trace context propagated via standard W3C `traceparent` header.
Exported to whatever OTLP endpoint the operator configures (Tempo,
Honeycomb, Datadog Agent, Grafana Cloud, etc.). Sampling is
tail-based at the collector; knievel always emits.

### 10.4 Error reporting (Sentry)

`sentry` Rust SDK + `sentry-tower` HTTP middleware (per-request hub
with auto-attached scope) + `sentry-tracing` (forwards `tracing`
events as breadcrumbs and elevates `ERROR` to Sentry exceptions).
Panic capture is enabled at boot.

What gets reported:

- All unhandled errors that surface as `5xx` responses.
- Panics (with full backtrace).
- Non-panic `tracing::error!()` calls — including event-flusher
  failures, snapshot-load failures, partition-missing errors.
- Migration failures at startup.

Per-request scope automatically attaches:

- `request_id`, `org_id`, `project_id`, `route`, `method`
- Sanitized request headers (no `Authorization`)
- Token name (not the secret) when present

Sentry DSN is optional in config; missing DSN disables reporting
entirely (useful for local dev). Sentry is for errors only;
performance/tracing live in OTel.

### 10.5 Metrics

Prometheus exposition at `/metrics`. Default labels: `project_id`,
`org_id` (cardinality bounded — one row per active project).

Required series:

- `knievel_decision_requests_total{project_id, outcome}`
- `knievel_decision_duration_seconds{project_id}` (histogram)
- `knievel_event_flush_batch_size{project_id}` (histogram)
- `knievel_event_channel_depth` (gauge, single global)
- `knievel_event_channel_dropped_total{reason}` (counter)
- `knievel_snapshot_age_seconds` (gauge, one per loader)
- `knievel_partman_maintenance_runs_total{result}` (counter)
- standard `process_*`, `tokio_*`, `sqlx_pool_*` series.

### 10.6 Health and readiness

- `/healthz` — liveness. `200` if the process is running.
- `/readyz` — readiness. `200` only if (a) snapshot has loaded once,
  (b) DB writer is reachable, (c) event flusher hasn't deadlocked,
  (d) partman maintenance has run within the last 24 h.

### 10.7 Graceful shutdown

On SIGTERM:

1. Stop accepting new connections (HTTP `Connection: close`).
2. Drain in-flight requests up to `shutdown_drain_timeout` (default
   30 s).
3. Flush the event channel — final `COPY` of all buffered events.
4. Close DB connections, OTel exporter, Sentry transport (with
   their own bounded flush deadlines).
5. Exit.

Total budget bounded by `shutdown_total_timeout` (default 60 s).

### 10.8 Database migrations

`sqlx-cli`-style: plain SQL files in `migrations/`, embedded via
`sqlx::migrate!()`. Run on startup if `database.auto_migrate: true`,
or via `knievel-cli migrate`. All migrations target the configured
schema and are idempotent at the migration-runner level (`_sqlx_migrations`).

## 11. Roadmap (post-v0)

Order is rough; each item is independently shippable.

1. **Frequency capping** — Redis joins the stack.
2. **Custom-property targeting** — flight predicates over arbitrary
   key/value pairs supplied in the decision request.
3. **Geo / IP targeting** — MaxMind DB on the snapshot side.
4. **Day-parting** — per-flight schedule.
5. **eCPM auctions** — second-price clearing within auction priorities.
6. **Reporting API** — query the rollup table; queue/poll model for
   heavy reports.
7. **Browser-direct mode** — CORS, anonymous decision endpoint, bot
   filtering, rate limits.
8. **UserDB-equivalent** — opaque user keys, interests, opt-out, GDPR
   forget. Designed fresh.
9. **Webhooks** — flight exhausted, sync complete, etc.
10. **Site Group scoping** — Project members/tokens scoped to a subset
    of Sites for sub-tenant admin isolation.
11. **Cross-project broadcast upsert** — for ads that span many Projects
    in an Org.
12. **Decision Explainer** — per-candidate reason codes for debugging.
13. **Custom event types** beyond impression/click (likes, shares, video
    quartiles).
14. **Web admin UI**.
15. **SSO / OIDC** for the admin UI.
16. **Write endpoints for Channel / Priority / AdType**.

## 12. Open Questions

- **Snapshot refresh strategy** — pure `LISTEN/NOTIFY` is lossy under
  load; a notify-then-version-poll backstop is probably worth the
  complexity.
- **Image hosting backend** — operator-configurable (S3-compatible,
  local disk, Postgres bytea), but pick a default. Lean S3 for the
  reference deployment.
- **CreativeTemplate schema language** — JSON Schema is the obvious
  answer; verify `poem-openapi` expresses the cross-reference cleanly.
- **Cross-project ads** — duplicate-on-write vs. broadcast endpoint vs.
  an "ad library" abstraction. Defer until a real use case appears.
- **User/auth backend for the admin UI** — local accounts vs. SSO-only.
  Lean SSO-only when the UI ships.

## References

- [Kevel Decision API reference](https://dev.kevel.com/reference/request)
- [Kevel Management API tutorial](https://dev.kevel.com/docs/management-api-tutorial)
- [Understanding Kevel](https://dev.kevel.com/docs/understanding-kevel)
- [`poem-openapi`](https://docs.rs/poem-openapi/)
- [`sqlx`](https://github.com/launchbadge/sqlx)
- [`pg_partman`](https://github.com/pgpartman/pg_partman)
- [`figment`](https://docs.rs/figment/) (layered configuration)
- [`tracing`](https://docs.rs/tracing/) and
  [`tracing-subscriber`](https://docs.rs/tracing-subscriber/)
- [`opentelemetry-rust`](https://github.com/open-telemetry/opentelemetry-rust)
- [`tracing-opentelemetry`](https://docs.rs/tracing-opentelemetry/)
- [Sentry Rust SDK](https://docs.rs/sentry/)
- [OpenAPI Generator](https://openapi-generator.tech)
- [Aurora PostgreSQL extensions](https://docs.aws.amazon.com/AmazonRDS/latest/AuroraPostgreSQLReleaseNotes/AuroraPostgreSQL.Extensions.html)
