# Knievel API

Implementation reference for the v0 HTTP surface. Companion to
`REQUIREMENTS.md`. The OpenAPI spec generated from the Rust binary is
the source of truth; this document is the human map.

All paths are prefixed `/v1` unless noted. JSON only. UTF-8 only.

## Conventions

### Authentication

- **Management + Decision endpoints:** `Authorization: Bearer <token>`.
  Tokens are scoped to a single network. Missing or invalid → `401`;
  valid but wrong network → `403`.
- **Event endpoints (`/e/...`):** unauthenticated. Browsers hit them
  directly. Authorization is the HMAC signature in the URL.
- **System endpoints (`/healthz`, `/readyz`, `/metrics`,
  `/openapi.json`):** unauthenticated by default; operator can put them
  behind a reverse proxy.

### Headers

| Header | Used on | Purpose |
|---|---|---|
| `Authorization` | management, decision | `Bearer <token>` |
| `Idempotency-Key` | all `POST`/`PATCH` | Replay-safe within 24 h |
| `Content-Type` | all writes | `application/json` (or `multipart/form-data` for image upload) |
| `If-Match` | `PATCH` (optional) | Optimistic concurrency on `etag` |
| `Accept` | any | `application/json` |

### Pagination

List endpoints use opaque cursors:

```
GET /v1/<resource>?limit=100&cursor=<opaque>
```

Response envelope:

```json
{
  "items": [ ... ],
  "nextCursor": "eyJ..." | null
}
```

`limit` defaults to 50, max 500. No `totalRecords` — counting is
expensive and rarely useful at this layer.

### Filters

List endpoints accept resource-specific filter query params plus a
universal `externalId=` filter. Filters AND together. Examples:

```
GET /v1/ads?flightId=42&isActive=true
GET /v1/ads?externalId=kevel_ad:7788
```

### Idempotency

Any `POST` or `PATCH` accepting `Idempotency-Key` stores the response
keyed on `(network, key, route, body-hash)` for 24 h. Replays return the
original response with `Idempotent-Replay: true` set.

### Status codes

| Code | Meaning |
|---|---|
| `200` | OK (read, update, idempotent replay) |
| `201` | Created |
| `202` | Accepted (async work queued) |
| `204` | No content (impression ping, delete) |
| `302` | Redirect (click ping) |
| `400` | Malformed request |
| `401` | Missing / invalid token |
| `403` | Wrong network or insufficient scope |
| `404` | Not found |
| `409` | Conflict (`externalId` collision, `If-Match` mismatch) |
| `422` | Semantically invalid (e.g. campaign references missing advertiser) |
| `429` | Rate limited |
| `500` | Server error |
| `503` | Event channel saturated; retry with backoff |

### Error body

```json
{
  "error": {
    "code": "validation_failed",
    "message": "siteId is required",
    "field": "placements[0].siteId",
    "requestId": "01JABCDEF..."
  }
}
```

`code` is a stable machine-readable string. `requestId` matches the
value in `X-Request-Id` response header and in server logs.

### Common entity fields

Every persistable entity has:

| Field | Type | Notes |
|---|---|---|
| `id` | int64 | Network-scoped, server-assigned. |
| `externalId` | string \| null | Unique within `(network, resource)`; caller-assigned. |
| `etag` | string | Opaque; pass to `If-Match` on `PATCH`. |
| `createdAt` | RFC 3339 | |
| `updatedAt` | RFC 3339 | |
| `isActive` | bool | Soft-delete via `isActive: false`; v0 has no hard delete. |

---

## 1. Decision API

### `POST /v1/decisions`

The hot path. Returns ad selections for one or more placements.

**Request:**

```json
{
  "placements": [
    {
      "id": "header",
      "siteId": 12,
      "zoneIds": [34, 35],
      "adTypes": [16],
      "count": 1,
      "force": {
        "adId": null,
        "campaignId": null,
        "flightId": null,
        "creativeId": null
      }
    }
  ]
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `placements[]` | array | yes | 1–32 placements per request. |
| `placements[].id` | string | yes | Echoed as the response key. Caller-defined. |
| `placements[].siteId` | int64 | yes | |
| `placements[].zoneIds` | int64[] | no | Restrict to specific zones. |
| `placements[].adTypes` | int[] | yes | Non-empty. |
| `placements[].count` | int | no | Default 1, max 10. |
| `placements[].force.*` | int64 \| null | no | Debug overrides; bypass eligibility filters. Not for production traffic. |

**Response (200):**

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

- `decisions[<id>]` is **always an array**. Empty array = no eligible ad.
- `creative` is a `oneOf`:
  - `{"type":"image","imageUrl":..,"width":..,"height":..,"alt":..,"clickThroughUrl":..}`
  - `{"type":"html","body":..,"clickThroughUrl":..}`
  - `{"type":"native","template":..,"values":{...}}`
- All URL fields are absolute.

---

## 2. Management API

### 2.1 Advertisers

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/advertisers` | List. Filter: `externalId`, `isActive`. |
| `POST` | `/v1/advertisers` | Create. |
| `POST` | `/v1/advertisers:batchUpsert` | Atomic upsert by `externalId`. |
| `GET` | `/v1/advertisers/{id}` | Read. |
| `PATCH` | `/v1/advertisers/{id}` | Partial update. |

Body:

```json
{
  "externalId": "rx:provider:42",
  "name": "Acme Corp",
  "isActive": true
}
```

### 2.2 Campaigns

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/campaigns` | List. Filter: `advertiserId`, `externalId`, `isActive`. |
| `POST` | `/v1/campaigns` | Create. |
| `POST` | `/v1/campaigns:batchUpsert` | Atomic upsert by `externalId`. |
| `GET` | `/v1/campaigns/{id}` | Read. |
| `PATCH` | `/v1/campaigns/{id}` | Partial update. |

Body:

```json
{
  "externalId": "rx:campaign:99",
  "advertiserId": 555,
  "name": "Spring Promo",
  "isActive": true
}
```

### 2.3 Flights

The unit of delivery. Carries dates, priority, and inventory targeting.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/flights` | List. Filter: `campaignId`, `priorityId`, `externalId`, `isActive`, `activeAt=<RFC3339>`. |
| `POST` | `/v1/flights` | Create. |
| `POST` | `/v1/flights:batchUpsert` | Atomic upsert by `externalId`. |
| `GET` | `/v1/flights/{id}` | Read. |
| `PATCH` | `/v1/flights/{id}` | Partial update. |

Body:

```json
{
  "externalId": "rx:flight:12345",
  "campaignId": 444,
  "name": "Spring Promo — Cardiology",
  "priorityId": 1,
  "startDate": "2026-05-01T00:00:00Z",
  "endDate":   "2026-06-01T00:00:00Z",
  "siteIds":   [12],
  "zoneIds":   [34],
  "adTypes":   [16],
  "isActive":  true
}
```

`siteIds` / `zoneIds` empty means "any site/zone in the network."
`adTypes` is required and non-empty.

### 2.4 Ads

The flight↔creative binding plus a delivery weight.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/ads` | List. Filter: `flightId`, `creativeId`, `externalId`, `isActive`. |
| `POST` | `/v1/ads` | Create. |
| `POST` | `/v1/ads:batchUpsert` | Atomic upsert by `externalId`. The endpoint RX's sync job hits most. |
| `GET` | `/v1/ads/{id}` | Read. |
| `PATCH` | `/v1/ads/{id}` | Partial update. |

Body:

```json
{
  "externalId": "rx:ad:7788",
  "flightId": 333,
  "creativeId": 4242,
  "weight": 100,
  "isActive": true
}
```

### 2.5 Creatives

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/creatives` | List. Filter: `advertiserId`, `type`, `externalId`, `isActive`. |
| `POST` | `/v1/creatives` | Create. |
| `GET` | `/v1/creatives/{id}` | Read. |
| `PATCH` | `/v1/creatives/{id}` | Partial update. |
| `POST` | `/v1/creatives/{id}/image` | Upload an image asset (multipart). Returns `{ "imageUrl": "..." }`. |

Body (`oneOf` on `type`):

```json
{
  "externalId": "rx:creative:image:42",
  "advertiserId": 555,
  "name": "Spring banner — 728x90",
  "type": "image",
  "imageUrl": "https://cdn.example.com/banner.jpg",
  "width": 728,
  "height": 90,
  "alt": "Acme Spring Sale",
  "clickThroughUrl": "https://acme.com/sale"
}
```

```json
{
  "type": "html",
  "body": "<div>...</div>",
  "clickThroughUrl": "https://..."
}
```

```json
{
  "type": "native",
  "templateId": 7,
  "values": {
    "title": "...",
    "body": "...",
    "imageUrl": "...",
    "ctaText": "Learn more"
  },
  "clickThroughUrl": "https://..."
}
```

`values` for `native` creatives is validated against the referenced
template's JSON Schema at write time; `422` on schema violation.

### 2.6 Creative Templates

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/creative-templates` | List. Filter: `name`, `externalId`. |
| `POST` | `/v1/creative-templates` | Create. |
| `GET` | `/v1/creative-templates/{id}` | Read. |
| `PATCH` | `/v1/creative-templates/{id}` | Partial update; bumps `version`. |

Body:

```json
{
  "externalId": "rx:template:sponsored_card_v1",
  "name": "sponsored_card_v1",
  "schema": {
    "type": "object",
    "required": ["title", "body", "ctaText"],
    "properties": {
      "title":    { "type": "string", "maxLength": 80 },
      "body":     { "type": "string", "maxLength": 240 },
      "imageUrl": { "type": "string", "format": "uri" },
      "ctaText":  { "type": "string", "maxLength": 24 }
    },
    "additionalProperties": false
  }
}
```

Mutating `schema` does **not** retroactively re-validate existing
creatives; it applies to subsequent writes only. Use a new template name
for breaking changes.

### 2.7 Inventory (read-only in v0)

These are network configuration, managed via CLI / SQL in v0. Read-only
HTTP surface so callers can resolve names → IDs.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sites` | List. Filter: `channelId`, `externalId`. |
| `GET` | `/v1/sites/{id}` | Read. |
| `GET` | `/v1/zones` | List. Filter: `siteId`, `externalId`. |
| `GET` | `/v1/zones/{id}` | Read. |
| `GET` | `/v1/channels` | List. |
| `GET` | `/v1/channels/{id}` | Read. |
| `GET` | `/v1/priorities` | List. Ordered by tier. |
| `GET` | `/v1/priorities/{id}` | Read. |
| `GET` | `/v1/ad-types` | List. |
| `GET` | `/v1/ad-types/{id}` | Read. |

Write endpoints for these resources are post-v0 (see roadmap).

---

## 3. Event Tracking

Unauthenticated; HMAC-signed in the URL. Browsers hit these directly
from the rendered ad.

### `GET /e/i/{signed}`

Impression ping.

- Default response: `204 No Content`.
- With `?fmt=gif`: `200 OK`, `image/gif`, 43-byte 1×1 transparent GIF.
- Tampered or expired signature: `204` (silent), counter incremented.
- Replay within TTL: counted (deduped via `dedup_key` on the row).

### `GET /e/c/{signed}`

Click ping.

- Records the click, `302` redirect to the creative's
  `clickThroughUrl`.
- Tampered or expired signature: `400`.
- Optional `?u=<url>` overrides the redirect target only if signed into
  the payload (prevents open-redirect abuse).

### Signature payload

HMAC-SHA256 over a compact binary record:

```
network_id | ad_id | creative_id | placement_id_hash | issued_at | nonce
```

URL-safe base64. Per-network secret. TTL is configurable per network
(default 24 h).

---

## 4. System

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/openapi.json` | OpenAPI 3.1 spec, generated from the binary. |
| `GET` | `/healthz` | Liveness. `200` if the process is up. |
| `GET` | `/readyz` | Readiness. `200` only if snapshot loaded, DB reachable, flusher healthy. |
| `GET` | `/metrics` | Prometheus exposition. |
| `GET` | `/version` | Build metadata: git sha, build time, schema version. |

---

## 5. Resource Map (cheat sheet)

```
Decision
  POST   /v1/decisions

Advertiser           — full CRUD + batchUpsert
Campaign             — full CRUD + batchUpsert
Flight               — full CRUD + batchUpsert
Ad                   — full CRUD + batchUpsert       (the hot management path)
Creative             — full CRUD + image upload
CreativeTemplate     — full CRUD

Site, Zone, Channel, Priority, AdType
                     — read-only (write deferred)

Events
  GET    /e/i/{signed}
  GET    /e/c/{signed}

System
  GET    /openapi.json
  GET    /healthz
  GET    /readyz
  GET    /metrics
  GET    /version
```

## 6. Out of Scope (v0)

The following endpoints are explicitly **not** in v0 — they map onto
the roadmap items in `REQUIREMENTS.md` §11. Calls return `404` with
`code: "not_implemented"`.

- `POST /v1/users/*` and `GET /v1/users/*` — UserDB.
- `POST /v1/decisions:explain` — Decision Explainer.
- Frequency-cap configuration on flights.
- Geo / IP / day-parting / keyword / custom-property targeting fields.
- `POST /v1/reports/*` — Reporting API.
- Webhook subscription endpoints.
- Custom event types beyond impression/click (`/e/x/{signed}`).
- Write endpoints for sites/zones/channels/priorities/ad-types.
