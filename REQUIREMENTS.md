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
- **Datastore:** Postgres for both configuration (source of truth) and
  events (range-partitioned). No Redis in v0.
- **Hot path:** the configuration snapshot lives in process memory, keyed
  by `(project_id, resource)`, refreshed on change notification. Decision
  requests touch RAM only.
- **Event path:** decision/impression/click events are buffered in an
  in-process channel and `COPY`'d to Postgres in batches every 1–2 s.

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

### 7.1 Configuration

Postgres. All entities carry `(org_id, project_id)` columns; isolation
enforced at the query layer and via Postgres row-level security policies
(defense in depth).

Source of truth. Mutated only via the Management API. Snapshot loader
subscribes via `LISTEN/NOTIFY` to a `config_changed` channel; pulls
diffs and atomically swaps the in-memory snapshot. Cold-start hydration
is one query per table.

The in-memory snapshot is keyed by `(project_id, resource)` so a single
process can serve thousands of small Projects efficiently.

### 7.2 Events

Two tables:

- **`events_raw`** — append-only, range-partitioned by day, managed with
  `pg_partman`. Columns: `ts`, `org_id`, `project_id`, `kind`
  (`decision` | `impression` | `click`), `placement_id`, `site_id`,
  `zone_id`, `ad_id`, `creative_id`, `flight_id`, `campaign_id`,
  `advertiser_id`, `url`, `referrer_host`, `user_agent_hash`,
  `signature_nonce`, `dedup_key`. Retention 30–90 days; old partitions
  detached and dropped.
- **`events_rollup`** — hourly aggregates by `(project_id, site_id,
  zone_id, flight_id, ad_id, creative_id, kind)`. Computed by a periodic
  job before raw partitions age out. Indefinite retention.

### 7.3 Write path

Per-request DB I/O is forbidden on the hot path. Events go to a bounded
`tokio::sync::mpsc` channel. A flusher task drains every 1–2 s (or 5 k
events, whichever first) and `COPY`s into the current `events_raw`
partition. Channel saturation surfaces as `503` on the decision endpoint
rather than silent loss.

### 7.4 What's deferred

- **Redis** — only needed when frequency capping or per-user pacing
  ships.
- **TimescaleDB / ClickHouse** — escape hatches if/when partitioned
  Postgres stops keeping up.

## 8. Deliverables

1. **`knievel`** Rust binary — server, snapshot loader, event flusher,
   migrations.
2. **`openapi.yaml`** — generated from the binary by `cargo xtask
   openapi`, committed to the repo, served at `/openapi.json`.
3. **Generated client libraries** — at minimum a Ruby gem
   (`knievel-ruby`) generated via `openapi-generator-cli` in CI,
   published on tag. Other languages on demand.
4. **`knievel-cli`** — admin CLI for project provisioning, token
   rotation, snapshot inspection, migration replay. Shares the OpenAPI
   client.
5. **Compose / Helm manifests** — knievel + Postgres for local dev and
   single-node deployment.

Per-consumer migration guides (e.g., `MIGRATION_RX.md`) live alongside
the spec but are not part of the v0 platform deliverable surface.

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

- Config via env vars + optional TOML (`KNIEVEL_DATABASE_URL`,
  `KNIEVEL_LISTEN_ADDR`, `KNIEVEL_HMAC_DEFAULT_SECRET`,
  `KNIEVEL_PUBLIC_BASE_URL`).
- Structured JSON logs via `tracing`. Decision-endpoint sampling at 1%
  by default; full sample on errors.
- Prometheus `/metrics`. Counters by `(project, decision_outcome)` and
  `(project, kind)` for events.
- Health: `/healthz` (liveness), `/readyz` (snapshot loaded, DB
  reachable, flusher healthy).
- Graceful shutdown: stop accepting requests, drain in-flight, flush
  event channel, exit. Bounded by a configurable deadline.
- Migrations via `sqlx-cli` or `refinery`; run on startup behind a flag.

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
- [`pg_partman`](https://github.com/pgpartman/pg_partman)
- [OpenAPI Generator](https://openapi-generator.tech)
