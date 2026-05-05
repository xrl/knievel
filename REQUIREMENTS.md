# Knievel Requirements

**Tagline:** Fearlessly fast ad delivery that steals the show.

Knievel is a Rust implementation of an ad server modeled on the public surface
of the [Kevel](https://dev.kevel.com) platform (formerly Adzerk). The goal is
wire-level compatibility with Kevel's three primary APIs so existing Kevel
SDKs and integrations can be pointed at a Knievel instance with minimal change.

This document is the starting point. It will be wrong in places — that's fine,
we iterate.

## 1. Goals

1. Serve ad decisions over HTTP with sub-millisecond p50 latency on a single
   node for typical placement counts (1–4 placements per request).
2. Expose Kevel-compatible Decision, Management, and UserDB APIs.
3. Be embeddable: a single statically-linked Rust binary plus a small
   datastore. No JVM, no Node runtime, no per-request allocation storms.
4. Be honest about what's not implemented yet — return clear errors rather
   than silently degrading.

## 2. Non-Goals (for v0)

- Full RTB / OpenRTB exchange integration.
- Header bidding orchestration.
- A hosted multi-tenant SaaS control plane (single-network deployments first).
- A web admin UI. CLI + API only until the data model stabilizes.
- Forecasting, pacing simulation, attribution modeling.

## 3. Domain Model

Mirroring Kevel's hierarchy:

```
Network
  └── Advertiser
        └── Campaign
              └── Flight              (time- and budget-bounded)
                    └── Ad
                          └── Creative

Network
  ├── Channel
  │     └── Site
  │           └── Zone
  ├── Priority                         (waterfall tier)
  └── AdType                           (size / format spec)
```

- **Network**: top-level tenant. All other entities scope under one network.
- **Advertiser**: brand owner of campaigns.
- **Campaign**: groups flights for an advertiser.
- **Flight**: time- and budget-bounded delivery rules; carries targeting,
  pricing model (CPM / CPC / CPA / flat), goal, and priority.
- **Ad**: a flight's binding to one or more creatives plus per-ad weight.
- **Creative**: the actual payload (image URL, HTML, native key/value blob,
  video VAST URL, etc.).
- **Site / Zone**: inventory taxonomy a placement targets.
- **Channel**: a grouping of sites (optional).
- **Priority**: ordered waterfall tier (e.g. Sponsorship > Standard >
  House > Backfill). Higher priorities short-circuit lower ones.
- **AdType**: integer ID identifying a creative format/size (Kevel uses
  small integers like `5`, `16`, etc.).

## 4. Decision API (ad serving)

The hot path. Compatibility target:

- **Endpoint:** `POST /api/v2`
  (Kevel form: `https://e-{networkId}.adzerk.net/api/v2`. Knievel will accept
  the network ID either via subdomain, path prefix `/{networkId}/api/v2`, or
  explicit field — config-selectable.)
- **Content-Type:** `application/json`. Also accept `text/plain` bodies that
  parse as JSON, since Kevel does (CORS preflight avoidance).
- **Auth:** none on the public decision endpoint. Bot filtering via
  `enableBotFiltering` flag.

### 4.1 Request body

Top-level fields:

| Field | Type | Required | Notes |
|---|---|---|---|
| `placements` | `Placement[]` | yes | One slot per element. |
| `user` | `{ key: string }` | no | UserDB key for targeting. |
| `keywords` | `string[]` | no | Keyword targeting. |
| `url` | `string` | no | Page URL (for site/keyword targeting). |
| `referrer` | `string` | no | |
| `ip` | `string` | no | Required for geo-targeting. |
| `time` | `string` | no | ISO-ish day/time-parting override. |
| `includePricingData` | `bool` | no | Default `false`. |
| `includeRelevancyData` | `bool` | no | Return relevancy scores. |
| `enableBotFiltering` | `bool` | no | Set `true` for client-side requests. |
| `enableUserDBIP` | `bool` | no | Use UserDB-stored IP over request IP. |
| `consent` | `{ gdpr: bool, ... }` | no | Consent flags. |
| `deviceID` | `string` | no | IFA / IDFA. |
| `parallel` | `bool` | no | Process placements in parallel (no cross-placement deduping). |
| `intendedLatitude` | `f64` | no | `[-90, 90]`. |
| `intendedLongitude` | `f64` | no | `[-180, 180]`. |
| `radius` | `f64` | no | `[0.01, 100]` (km). |
| `searchTerm` | `string` | no | |
| `block` | `{ advertisers?: int[], campaigns?: int[], creatives?: int[] }` | no | |
| `blockedCreatives` | `int[]` | no | Legacy alias. |
| `notrack` | `bool` | no | Deprecated; honored for compatibility. |

### 4.2 Placement

| Field | Type | Required | Notes |
|---|---|---|---|
| `divName` | `string` | yes | Echoed as the key in `decisions`. |
| `networkId` | `int` | yes | |
| `siteId` | `int` | yes (or `zoneIds`) | |
| `adTypes` | `int[]` | yes | At least one. |
| `zoneIds` | `int[]` | no | Restrict to specific zones. |
| `campaignId` | `int` | no | Force a specific campaign. |
| `flightId` | `int` | no | Force a specific flight. |
| `adId` | `int` | no | Force a specific ad. |
| `creativeId` | `int` | no | Force a specific creative. |
| `eventIds` | `int[]` | no | Custom events to mint URLs for. |
| `properties` | `object` | no | Custom-targeting key/value pairs. |
| `keywords` | `string[]` | no | Per-placement keyword override. |
| `count` | `int` | no | Return up to N decisions for this slot. Default 1. |
| `ecpmPartition` | `string` | no | Partition for eCPM optimization. |
| `overrideKey` | `string` | no | Forces a specific decision branch (testing). |

### 4.3 Response body

```json
{
  "user": { "key": "..." },
  "decisions": {
    "<divName>": {
      "adId": 0,
      "creativeId": 0,
      "flightId": 0,
      "campaignId": 0,
      "advertiserId": 0,
      "priorityId": 0,
      "clickUrl": "...",
      "impressionUrl": "...",
      "contents": [{ "type": "html", "data": { "imageUrl": "...", "title": "..." }, "body": "..." }],
      "events": [{ "id": 1, "url": "..." }],
      "pricing": { "price": 0.0, "clearPrice": 0.0 },
      "relevancy": { "score": 0.0 },
      "matchedPoints": 0
    }
  }
}
```

`decisions[divName]` is `null` when no ad is selected. When a placement's
`count` > 1, the value is an array of decisions instead of a single object.
This is a known Kevel quirk we replicate.

### 4.4 Selection algorithm

For each placement, in order:

1. Filter ads to those whose flight is active (date window, budget remaining,
   day-parting match).
2. Filter to ads matching `adTypes`, `siteId`/`zoneIds`, geo, device,
   keyword, custom-property, and frequency-cap rules.
3. Apply `block` lists and `blockedCreatives`.
4. Apply UserDB-driven targeting (interests, custom properties).
5. Group remaining ads by priority tier; the highest tier with any eligible
   ad wins. Within a tier:
   - **Auction priorities**: weighted by eCPM (with floor and second-price
     clearing if configured).
   - **Non-auction priorities**: weighted random by ad weight, with goal
     pacing influencing weights.
6. Mint signed click, impression, and event URLs scoped to
   `(networkId, adId, creativeId, divName, userKey?, ts, nonce)`.

Determinism: requests carrying the same `overrideKey` MUST return the same
decision (or null) within a flight's eligibility window.

## 5. Event Tracking

Three event endpoints, all GET, all returning a 1×1 GIF or a 302:

- `GET /i.gif?...` — impression.
- `GET /r?...` — click; 302 to creative `clickUrl` after recording.
- `GET /e.gif?...` — custom events (likes, shares, video quartiles, etc.).

URL parameters are HMAC-signed at decision time. Replays past a configurable
TTL (default 24h) are accepted but flagged. Tampered signatures are dropped
silently with a counter increment.

Counters feed both real-time pacing/frequency-capping and the report store.

## 6. Management API

- **Base URL:** `/v1` (Kevel form: `https://api.kevel.co/v1/`).
- **Auth:** `X-Adzerk-ApiKey: <key>` header. Keys scoped to a network.
- **Style:** REST, JSON, standard CRUD.

Endpoints (initial set):

| Resource | Operations |
|---|---|
| Advertiser | `POST/GET/PUT /advertiser`, `GET /advertiser/{id}`, list with paging |
| Campaign | `POST/GET/PUT /campaign`, `GET /campaign/{id}` |
| Flight | `POST/GET/PUT /flight`, `GET /flight/{id}`, `GET /flight/{id}/creative` |
| Ad (Flight Creative Map) | `POST/GET/PUT /flight/{flightId}/creative` |
| Creative | `POST/GET/PUT /creative`, `POST /creative/{id}/upload` (multipart) |
| Channel | `GET /channel`, `GET /channel/{id}` |
| Site | `GET /site`, `GET /site/{id}` |
| Zone | `GET /zone`, `GET /zone/{id}` |
| Priority | `GET /priority` |
| AdType | `GET /adtypes` |
| Report | `POST /report/queue`, `GET /report/queue/{id}` |

Pagination uses `?page=` + `?pageSize=` with a `totalRecords` envelope, again
to match Kevel's wire format.

Idempotency: `POST` accepts an optional `Idempotency-Key` header; replays
within 24h return the original response.

## 7. UserDB API

- **Base URL:** `/udb/{networkId}` (Kevel uses `e-{networkId}.adzerk.net/udb/{networkId}`).
- **Auth:** none for read of own key; mutating endpoints accept the user's
  own UserKey or a server-side API key.

Endpoints:

| Method | Path | Purpose |
|---|---|---|
| GET | `/read?userKey=` | Return the user record. |
| POST | `/custom?userKey=` | Set custom properties (merge). |
| POST | `/interest/i.gif?userKey=&interest=` | Add an interest. |
| POST | `/optout/i.gif?userKey=` | Opt out of targeting. |
| POST | `/optout/forget/i.gif?userKey=` | GDPR forget. |
| POST | `/ip?userKey=&ip=` | Set IP for the user record. |
| POST | `/gdpr-consent` | Record consent payload. |
| POST | `/cookies/retarget/{advId}/{segId}?userKey=` | Add to retargeting segment. |

User keys are opaque, 128-bit-equivalent strings. Knievel mints them on first
decision request that omits `user.key`, returning the new key in the response.

## 8. Storage

Two stores, separated by access pattern:

- **Configuration store** (campaigns, flights, ads, creatives, sites, etc.):
  Postgres. Source of truth. Mutated only via the Management API.
- **Decision cache**: an in-memory snapshot loaded at boot and refreshed on
  change notifications (LISTEN/NOTIFY or a poll loop). All hot-path reads
  hit RAM; the database is never on the request path for `POST /api/v2`.
- **UserDB**: Redis (or compatible). Keyed by user key, TTL-bounded.
- **Event log**: append-only stream (Kafka-compatible or local file +
  rotation). Consumer aggregates into the report store.
- **Report store**: ClickHouse or Postgres with rollups, depending on volume.

Single-node mode collapses Postgres + Redis + local file log into one
sqlite-backed binary for testing and small deployments.

## 9. Performance Targets

For a single Knievel node, 4-core / 8 GB:

- p50 decision latency ≤ **1 ms** for 1 placement, no UserDB lookup.
- p99 decision latency ≤ **10 ms** for 4 placements + UserDB lookup.
- Sustained throughput ≥ **20 000 decisions/sec** before saturating one core.
- Cold-start to first decision served ≤ **2 s** with 100k active flights.

These are starting numbers; we'll measure and adjust.

## 10. Operational

- Configuration via env vars + optional TOML file.
- Structured JSON logs (tracing crate). One log line per request at INFO is
  too noisy at target throughput — sample at 1% by default, full sample for
  errors.
- Metrics: Prometheus `/metrics` endpoint. Counters per
  `(network, site, zone, decision_outcome)`.
- Health: `/healthz` (liveness) and `/readyz` (snapshot loaded, DB reachable).
- Graceful shutdown: drain in-flight decisions, flush event log, exit.

## 11. Compatibility Caveats

Things we intentionally do **not** match Kevel on yet, and will document
clearly in error responses:

- No `*.adzerk.net` domain spoofing — operators bring their own domain.
- No legacy XML/JSONP response formats.
- No Adzerk v1 API.
- Reporting schema is a subset; advanced report types return 501.

## 12. Open Questions

- Pricing model edge cases: do we replicate Kevel's exact eCPM tie-break, or
  document our own? Need to read the Decision Explainer output more closely.
- Frequency capping granularity: per-flight, per-campaign, per-advertiser —
  all three, or pick one for v0?
- Native ad templating: Kevel ships a Mustache-like templater server-side.
  Do we, or do we leave templating to the client SDK?
- Multi-network in a single binary, or one network per process?

## References

- [Kevel Decision API reference](https://dev.kevel.com/reference/request)
- [Kevel Management API tutorial](https://dev.kevel.com/docs/management-api-tutorial)
- [Decision API quickstart](https://dev.kevel.com/docs/native-ads-api-quickstart)
- [Understanding Kevel](https://dev.kevel.com/docs/understanding-kevel)
- [Decision Explainer](https://dev.kevel.com/docs/decision-explainer)
