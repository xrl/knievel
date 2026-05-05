# Knievel Requirements

**Tagline:** Fearlessly fast ad delivery that steals the show.

Knievel is a Rust ad server inspired by [Kevel](https://dev.kevel.com)'s
domain model, but with its own clean OpenAPI-defined wire format. The first
consumer is the RX app, which today calls Kevel directly; knievel will
replace that integration via a generated Ruby client gem at the existing
call sites.

This document is the working spec. It will be wrong in places — we iterate.

## 1. Goals

1. Replace RX's Kevel usage end-to-end: same call sites, same render path,
   no behavior change visible to providers or end users.
2. OpenAPI-first: the spec is the contract. The Rust server and the Ruby
   client gem are both derived from it.
3. Sub-millisecond p50 decision latency on a single node for typical RX
   placements.
4. Single statically-linked Rust binary + Postgres. No second datastore in
   v0.
5. A foundation that can grow into the broader Kevel feature surface
   without rewrites.

## 2. Non-Goals (v0)

- Kevel wire compatibility. We are not mimicking `divName`, `e-{networkId}.adzerk.net`,
  the `text/plain` JSON CORS hack, or Kevel's URL conventions. Drop-in
  compatibility is a Ruby gem swap, not a wire-format swap.
- Browser-direct ad calls. RX proxies through its app server today; that
  is the intended deployment shape (see §4).
- UserDB / per-user behavioral targeting. RX does not use it.
- Geo, IP, lat/long, radius, day-parting, keyword, custom-property
  targeting.
- Frequency capping, pacing simulation, eCPM auctions, second-price
  clearing, pricing data, relevancy scores.
- Reporting API surface. RX tracks its own analytics from impression and
  click pings.
- Multi-network in a single binary.
- Web admin UI. CLI + API only.
- Header bidding, OpenRTB, DSP/SSP integration.

These are explicitly future work, not "never." See §11.

## 3. Architecture

```
┌──────────────┐    POST /v1/decisions    ┌──────────────┐
│   RX app     │ ───────────────────────▶ │   knievel    │
│  (Ruby gem)  │ ◀─────────────────────── │  (Rust/poem) │
└──────────────┘                          └──────┬───────┘
       ▲                                         │
       │                                  in-mem │ snapshot
       │                                         ▼
       │                                  ┌──────────────┐
       └─ impression/click pings ────────▶│   Postgres   │
          GET /e/i/<sig>                  │   (config +  │
          GET /e/c/<sig>                  │  partitioned │
                                          │   events)    │
                                          └──────────────┘
```

- **Web framework:** [`poem`](https://github.com/poem-web/poem) +
  [`poem-openapi`](https://docs.rs/poem-openapi/). Handlers and request/
  response types are annotated; the OpenAPI spec is generated from the
  binary and exposed at `GET /openapi.json`.
- **Datastore:** Postgres for both configuration (source of truth) and
  events (partitioned). No Redis in v0.
- **Hot path:** the configuration snapshot lives in process memory,
  refreshed on change notification. Decision requests touch RAM only.
- **Event path:** decision/impression/click events are buffered in an
  in-process channel and `COPY`'d to Postgres in batches every 1–2 s.

## 4. Integration Shape

Knievel is designed for **server-to-server** calls from a trusted upstream
(the app server). Browser-direct mode is a future addition.

This matches industry direction for first-party ad serving: ad-blocker
resistance, server-side enrichment, simpler auth, easier backend swaps.

Concretely:

- The decision endpoint takes a Bearer token. One trusted credential per
  network; no per-browser bot filtering, no CORS preflight, no signed
  client requests.
- Plain `application/json`. No `text/plain` workaround.
- CORS is off by default. Browser-direct mode (CORS, anonymous decision,
  rate-limited, bot-signal aware) is a v1+ feature.
- Impression and click pings are GETs that browsers hit directly; their
  signatures are HMAC-minted at decision time.

## 5. Domain Model

Borrowed from Kevel because the hierarchy is sound:

```
Network
  └── Advertiser
        └── Campaign
              └── Flight              (date-bounded delivery rules)
                    └── Ad            (flight-to-creative binding + weight)
                          └── Creative

Network
  ├── Channel ── Site ── Zone
  ├── Priority                         (waterfall tier)
  └── AdType                           (format/size identifier)
```

Every entity carries an `externalId` (string, unique within network) so
RX's syncer can drive knievel from its own `KevelAd` records without
maintaining an ID mapping table.

## 6. API Surface (v0)

OpenAPI 3.1, served at `/openapi.json`. Bearer auth. Cursor pagination.
`Idempotency-Key` header documented as part of the spec.

### 6.1 Decision API

`POST /v1/decisions`

```json
{
  "placements": [
    {
      "id": "header",
      "siteId": 12,
      "zoneIds": [34],
      "adTypes": [16],
      "count": 1,
      "force": { "adId": null, "campaignId": null, "flightId": null }
    }
  ]
}
```

Response:

```json
{
  "decisions": {
    "header": [
      {
        "adId": 9001,
        "creativeId": 4242,
        "flightId": 333,
        "campaignId": 444,
        "advertiserId": 555,
        "priorityId": 1,
        "externalId": "kevel_ad:7788",
        "clickUrl": "https://ads.example.com/e/c/AbCd...",
        "impressionUrl": "https://ads.example.com/e/i/EfGh...",
        "creative": {
          "type": "native",
          "template": "sponsored_card_v1",
          "values": {
            "title": "...",
            "body": "...",
            "imageUrl": "...",
            "ctaText": "..."
          }
        }
      }
    ]
  }
}
```

Notes:
- `decisions[id]` is **always an array**, even when `count == 1`. Empty
  array means no eligible ad. (Kevel's "object-or-array" quirk is gone.)
- `creative` is a `oneOf` — `image`, `html`, `native` — typed, not a hash
  blob.
- `externalId` is echoed so the caller can correlate without a lookup.

Selection algorithm:
1. Filter to flights active at request time (date window).
2. Filter to ads matching `siteId`/`zoneIds`/`adTypes`.
3. Apply `force.*` overrides (debug / testing).
4. Group by priority tier; highest non-empty tier wins.
5. Within tier: weighted random by ad weight.
6. Mint HMAC-signed click and impression URLs.

### 6.2 Management API

REST, JSON, Bearer auth, cursor pagination.

| Resource | Endpoints |
|---|---|
| Advertiser | `POST/GET/PATCH /v1/advertisers`, `GET /v1/advertisers/{id}`, `POST /v1/advertisers:batchUpsert` |
| Campaign | `POST/GET/PATCH /v1/campaigns`, `GET /v1/campaigns/{id}` |
| Flight | `POST/GET/PATCH /v1/flights`, `GET /v1/flights/{id}` |
| Ad | `POST/GET/PATCH /v1/ads`, `GET /v1/ads/{id}`, `POST /v1/ads:batchUpsert` |
| Creative | `POST/GET/PATCH /v1/creatives`, `GET /v1/creatives/{id}`, `POST /v1/creatives/{id}/image` (multipart) |
| CreativeTemplate | `POST/GET/PATCH /v1/creative-templates`, `GET /v1/creative-templates/{id}` |
| Site | `GET /v1/sites`, `GET /v1/sites/{id}` |
| Zone | `GET /v1/zones`, `GET /v1/zones/{id}` |
| Channel | `GET /v1/channels`, `GET /v1/channels/{id}` |
| Priority | `GET /v1/priorities` |
| AdType | `GET /v1/ad-types` |

All write endpoints accept an optional `Idempotency-Key` header; replays
within 24 h return the original response. Bulk upserts are atomic per
batch and keyed on `externalId`.

Lookups by external ID: `GET /v1/ads?externalId=kevel_ad:7788` returns the
canonical record. RX's syncer uses this rather than maintaining its own
mapping table.

### 6.3 Event Tracking

- `GET /e/i/{signed}` — impression. Returns `204 No Content` (or 1×1 GIF
  if `?fmt=gif`).
- `GET /e/c/{signed}` — click. Returns `302` to the creative's click-through
  URL.

Signed payloads are HMAC-SHA256 over `(network, ad, creative, placement,
ts, nonce)` with a per-network secret. Replays past a configurable TTL
(default 24 h) are accepted but flagged. Tampered signatures drop with a
counter increment.

### 6.4 Creative Templates (native ads)

RX's `CreativeTemplate` + dynamic JSONB values pattern, modeled as typed
OpenAPI variants:

- A `CreativeTemplate` defines a name and a JSON Schema for `values`.
- A `Creative` of type `native` references a template and supplies `values`
  conforming to that schema.
- The decision response returns the template name and validated `values`,
  not a rendered string. Rendering stays client-side.

## 7. Storage

### 7.1 Configuration

Postgres. One schema per network (or a `network_id` column — TBD).
Mutated only via the Management API. Acts as source of truth.

Snapshot loader subscribes via `LISTEN/NOTIFY` to a `config_changed`
channel. On notify, it pulls the diff and atomically swaps the in-memory
snapshot. Cold-start hydration is a single query per table.

### 7.2 Events

Two tables:

- **`events_raw`** — append-only, range-partitioned by day, managed with
  `pg_partman`. Columns: `ts`, `network_id`, `kind` (`decision` |
  `impression` | `click`), `placement_id`, `ad_id`, `creative_id`,
  `flight_id`, `campaign_id`, `advertiser_id`, `signature_nonce`,
  `dedup_key`. Retention 30–90 days; old partitions detached and dropped.
- **`events_rollup`** — hourly aggregates by `(network_id, site_id,
  zone_id, flight_id, ad_id, creative_id, kind)`. Computed by a periodic
  job before raw partitions age out. Indefinite retention.

### 7.3 Write path

Per-request DB I/O is forbidden on the hot path. Events go to a bounded
`tokio::sync::mpsc` channel. A flusher task drains every 1–2 s (or 5k
events, whichever first) and `COPY`s into the current `events_raw`
partition. Channel saturation surfaces as `503` on the decision endpoint
rather than silent loss.

### 7.4 What's deferred

- **Redis** — only needed for frequency capping and per-user pacing
  counters, neither of which v0 supports. Add when needed.
- **TimescaleDB / ClickHouse** — escape hatches if/when partitioned
  Postgres stops keeping up. RX's volume does not warrant either now.

## 8. Deliverables

1. **`knievel`** Rust binary — server, snapshot loader, event flusher,
   migrations.
2. **`openapi.yaml`** — generated from the binary by `cargo xtask
   openapi`, committed to the repo, served at `/openapi.json`.
3. **`knievel-ruby`** gem — generated from the spec via
   `openapi-generator-cli` in CI, published on tag.
4. **Migration guide** — per-call-site mapping from RX's `Kevel::*`
   classes to `Knievel::*`. One PR per call site.
5. **Compose / Helm manifests** — knievel + Postgres for local dev and
   single-node deployment.

## 9. Performance Targets

Single node, 4 vCPU / 8 GB RAM, 100k active flights:

- p50 decision latency ≤ **1 ms** (1 placement, no force overrides).
- p99 decision latency ≤ **10 ms** (4 placements).
- Sustained throughput ≥ **20 000 decisions/sec** before saturating one
  core.
- Cold-start to first decision served ≤ **2 s**.
- Event flusher keeps end-to-end ingest lag ≤ **3 s** at peak.

These are starting numbers; we measure and adjust.

## 10. Operational

- Config via env vars + optional TOML (`KNIEVEL_DATABASE_URL`,
  `KNIEVEL_LISTEN_ADDR`, `KNIEVEL_HMAC_SECRET`, `KNIEVEL_API_KEYS`).
- Structured JSON logs via `tracing`. Decision-endpoint sampling at 1%
  by default; full sample on errors.
- Prometheus `/metrics`. Counters by `(network, site, zone, decision_outcome)`
  and `(network, kind)` for events.
- Health: `/healthz` (liveness), `/readyz` (snapshot loaded, DB
  reachable, flusher healthy).
- Graceful shutdown: stop accepting requests, drain in-flight, flush
  event channel, exit. Bounded by a configurable deadline.
- Migrations via `sqlx-cli` or `refinery`; run on startup behind a flag.

## 11. Roadmap (post-v0)

Order is rough. Each item is independently shippable.

1. **Frequency capping** — Redis joins the stack.
2. **Custom-property targeting** — flight predicates over arbitrary key/
   value pairs supplied in the decision request.
3. **Geo / IP targeting** — MaxMind DB on the snapshot side.
4. **Day-parting** — per-flight schedule.
5. **eCPM auctions** — second-price clearing within auction priorities.
6. **Reporting API** — query the rollup table; queue/poll model for
   heavy reports.
7. **Browser-direct mode** — CORS, anonymous decision endpoint, bot
   filtering.
8. **UserDB-equivalent** — opaque user keys, interests, opt-out, GDPR
   forget. Designed fresh, not copied.
9. **Webhooks** — flight exhausted, sync complete, etc.
10. **Multi-network single-binary** — currently one process per network.
11. **Decision Explainer** — return per-candidate reason codes for
    debugging.
12. **Custom events** — likes, shares, video quartiles beyond
    impression/click.

## 12. Open Questions

- Network isolation: schema-per-network vs `network_id` column.
  Schema-per-network gives cleaner blast radius and easier per-tenant
  backups; column is simpler ops. Lean toward column for v0, schema for
  multi-tenant later.
- Snapshot refresh: pure `LISTEN/NOTIFY` vs notify-then-poll-with-version.
  Notify alone is lossy under load; version polling as a backstop is
  probably worth the complexity.
- Image hosting for creatives: store in Postgres bytea, on local disk, in
  S3? Defer to operator (configurable backend) but pick a default.
- `CreativeTemplate` schema language: JSON Schema is the obvious answer;
  confirm `poem-openapi` can express the cross-reference cleanly.
- Migration cadence with RX: big-bang gem swap vs feature-flagged
  per-placement rollout. Probably the latter, but RX team owns that call.

## References

- [Kevel Decision API reference](https://dev.kevel.com/reference/request)
- [Kevel Management API tutorial](https://dev.kevel.com/docs/management-api-tutorial)
- [Understanding Kevel](https://dev.kevel.com/docs/understanding-kevel)
- [`poem-openapi`](https://docs.rs/poem-openapi/)
- [`pg_partman`](https://github.com/pgpartman/pg_partman)
- [OpenAPI Generator (Ruby)](https://openapi-generator.tech/docs/generators/ruby)
