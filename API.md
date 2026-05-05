# Knievel API

Implementation reference for the v0 HTTP surface. Companion to
`REQUIREMENTS.md`. The OpenAPI spec generated from the Rust binary is the
source of truth; this document is the human map.

JSON only. UTF-8 only.

## Path Structure

- `/v1/orgs/{orgId}/...` — org-level operations (project provisioning,
  tokens, members). Org token only.
- `/v1/projects/{projectId}/...` — project-scoped resources (decisions,
  CRUD).
- `/e/...` — public event tracking (HMAC-signed in URL, no auth).
- `/openapi.json`, `/healthz`, `/readyz`, `/metrics`, `/version` — system.

`{orgId}` and `{projectId}` accept either the server-assigned ID
(`org_AbCd...`, `pj_EfGh...`) or the URL-safe `externalId`.

## Conventions

### Authentication

- **Management + Decision endpoints:** `Authorization: Bearer <token>`.
  - **Org tokens** can address any Project in their Org via the
    `/v1/projects/{projectId}/...` paths.
  - **Project tokens** can only address their own Project.
  - Wrong org or project for the token → `403`.
- **Event endpoints (`/e/...`)**: unauthenticated. Browsers hit them
  directly. Authorization is the HMAC signature in the URL.
- **System endpoints**: unauthenticated by default; operator can put them
  behind a reverse proxy.

### Headers

| Header | Used on | Purpose |
|---|---|---|
| `Authorization` | management, decision | `Bearer <token>` |
| `Idempotency-Key` | all `POST`/`PATCH` | Replay-safe within 24 h |
| `Content-Type` | writes | `application/json` (or `multipart/form-data` for image upload) |
| `If-Match` | `PATCH` (optional) | Optimistic concurrency on `etag` |
| `Accept` | any | `application/json` |

### Pagination

Cursor-based:

```
GET /v1/projects/{projectId}/<resource>?limit=100&cursor=<opaque>
```

Response envelope:

```json
{
  "items": [ ... ],
  "nextCursor": "eyJ..." | null
}
```

`limit` defaults to 50, max 500. No `totalRecords`.

### Filters

Every list endpoint accepts `externalId=` plus resource-specific filters.
Filters AND together. Examples:

```
GET /v1/projects/pj_AbCd/ads?flightId=42&isActive=true
GET /v1/projects/pj_AbCd/ads?externalId=ad-2024-spring-1
```

### Idempotency

`POST` and `PATCH` accept `Idempotency-Key`. Responses are stored 24 h
keyed on `(project, key, route, body-hash)`. Replays return the original
response with `Idempotent-Replay: true` set.

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
| `403` | Wrong org/project or insufficient role |
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
`X-Request-Id` response header and server logs.

### Common entity fields

| Field | Type | Notes |
|---|---|---|
| `id` | string | Server-assigned, project-scoped (or org-scoped for orgs/projects). |
| `externalId` | string \| null | Caller-assigned, unique within `(project, resource)`. |
| `etag` | string | Pass to `If-Match` on `PATCH`. |
| `createdAt` | RFC 3339 | |
| `updatedAt` | RFC 3339 | |
| `isActive` | bool | Soft-delete via `isActive: false`; v0 has no hard delete. |

---

## 1. Decision API

### `POST /v1/projects/{projectId}/decisions`

The hot path. Returns ad selections for one or more placements.

**Request:**

```json
{
  "context": {
    "url":       "https://example.com/article/42",
    "referrer":  "https://www.google.com/...",
    "userAgent": "Mozilla/5.0 ..."
  },
  "placements": [
    {
      "id":            "header",
      "siteId":        12,
      "siteUrl":       null,
      "siteExternalId": null,
      "zoneIds":       [34, 35],
      "adTypes":       [16],
      "count":         1,
      "force": {
        "adId":        null,
        "campaignId":  null,
        "flightId":    null,
        "creativeId":  null
      }
    }
  ],
  "block": {
    "creativeIds":   [],
    "advertiserIds": [],
    "campaignIds":   []
  }
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `context.url` | string | no | Page URL serving the ad. Stored on event rows. |
| `context.referrer` | string | no | Referrer URL. |
| `context.userAgent` | string | no | UA string; hashed before storage. |
| `placements[]` | array | yes | 1–32 placements per request. |
| `placements[].id` | string | yes | Echoed as the response key. |
| `placements[].siteId` \| `siteUrl` \| `siteExternalId` | — | yes (one) | Identifies the site. URL match consults `Site.url` and `Site.aliases`. |
| `placements[].zoneIds` | int64[] | no | Restrict to specific zones. |
| `placements[].adTypes` | int[] | yes | Non-empty. |
| `placements[].count` | int | no | Default 1, max 10. |
| `placements[].force.*` | int64 \| null | no | Debug overrides; bypass eligibility. Not for production. |
| `block.creativeIds` | int64[] | no | Caller-derived suppression list. |
| `block.advertiserIds` | int64[] | no | |
| `block.campaignIds` | int64[] | no | |

`context` is informational only — it is **never** used for tenant
routing. The project ID in the path is the sole authoritative tenant
signal.

**Response (200):**

```json
{
  "decisions": {
    "header": [
      {
        "adId":         9001,
        "creativeId":   4242,
        "flightId":     333,
        "campaignId":   444,
        "advertiserId": 555,
        "priorityId":   1,
        "siteId":       12,
        "externalId":   "ad-2024-spring-1",
        "clickUrl":     "https://ads.example.com/e/c/AbCd...",
        "impressionUrl":"https://ads.example.com/e/i/EfGh...",
        "creative": {
          "type":     "native",
          "template": "sponsored_card_v1",
          "values": {
            "title":   "...",
            "body":    "...",
            "imageUrl":"...",
            "ctaText": "Learn more"
          }
        }
      }
    ]
  }
}
```

- `decisions[<id>]` is **always an array**, even when `count == 1`.
  Empty array = no eligible ad.
- `siteId` is the resolved site (useful when caller passed `siteUrl` or
  `siteExternalId`).
- `creative` is a `oneOf`:
  - `{"type":"image","imageUrl":..,"width":..,"height":..,"alt":..,"clickThroughUrl":..}`
  - `{"type":"html","body":..,"clickThroughUrl":..}`
  - `{"type":"native","template":..,"values":{...},"clickThroughUrl":..}`
- All URL fields are absolute.

---

## 2. Org Operations

All endpoints in this section require an **Org token** (Org Owner / Org
Admin).

### 2.1 Projects

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/orgs/{orgId}/projects` | List. Filter: `externalId`, `isActive`. |
| `POST` | `/v1/orgs/{orgId}/projects` | Create. Idempotent on `externalId`. |
| `POST` | `/v1/orgs/{orgId}/projects:batchUpsert` | Bulk by `externalId`. |
| `GET` | `/v1/orgs/{orgId}/projects/{projectId}` | Read. |
| `PATCH` | `/v1/orgs/{orgId}/projects/{projectId}` | Update name / isActive / hmacSecret. |

Body:

```json
{
  "externalId": "tenant_acme",
  "name":       "Acme Marketplace"
}
```

### 2.2 Tokens

| Verb | Path | Purpose |
|---|---|---|
| `POST` | `/v1/orgs/{orgId}/tokens` | Mint a token. Returns the secret exactly once. |
| `GET` | `/v1/orgs/{orgId}/tokens` | List token metadata (no secrets). |
| `DELETE` | `/v1/orgs/{orgId}/tokens/{tokenId}` | Revoke. |

Create body:

```json
{
  "name":        "prod sync",
  "scope":       "org",
  "projectId":   null,
  "role":        "editor",
  "expiresAt":   null,
  "ipAllowlist": []
}
```

| Field | Required | Notes |
|---|---|---|
| `scope` | yes | `org` or `project`. |
| `projectId` | if `scope=project` | |
| `role` | yes | `org-owner`, `org-admin`, `admin`, `editor`, `reader`. Org tokens take an org-level role; project tokens take a project-level role. |
| `expiresAt` | no | RFC 3339. Null = never. |
| `ipAllowlist` | no | CIDR list. Empty = no restriction. |

Create response (`201`):

```json
{
  "id":        "tok_AbCd...",
  "secret":    "kvl_prod_org_AbCd_8f2a...",
  "name":      "prod sync",
  "scope":     "org",
  "role":      "editor",
  "createdAt": "..."
}
```

`secret` is returned **only once**. Lost secrets cannot be recovered;
revoke and reissue.

### 2.3 Members

User management is mostly stub for v0 (no admin UI), but the endpoints
exist so the data model and auth checks are in place.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/orgs/{orgId}/members` | List org members. |
| `POST` | `/v1/orgs/{orgId}/members` | Invite a user. |
| `PATCH` | `/v1/orgs/{orgId}/members/{userId}` | Change org role or per-project roles. |
| `DELETE` | `/v1/orgs/{orgId}/members/{userId}` | Remove. |

---

## 3. Project Resources

All endpoints in this section accept either an **Org token** (with
sufficient role) or a **Project token** scoped to `{projectId}`.

### 3.1 Advertisers

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/advertisers` | List. Filter: `externalId`, `isActive`. |
| `POST` | `/v1/projects/{projectId}/advertisers` | Create. |
| `POST` | `/v1/projects/{projectId}/advertisers:batchUpsert` | Bulk by `externalId`. |
| `GET` | `/v1/projects/{projectId}/advertisers/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/advertisers/{id}` | Update. |

Body:

```json
{
  "externalId": "advertiser-acme",
  "name":       "Acme Corp",
  "isActive":   true
}
```

### 3.2 Campaigns

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/campaigns` | List. Filter: `advertiserId`, `externalId`, `isActive`. |
| `POST` | `/v1/projects/{projectId}/campaigns` | Create. |
| `POST` | `/v1/projects/{projectId}/campaigns:batchUpsert` | Bulk. |
| `GET` | `/v1/projects/{projectId}/campaigns/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/campaigns/{id}` | Update. |

Body:

```json
{
  "externalId":   "campaign-spring-2026",
  "advertiserId": 555,
  "name":         "Spring Promo",
  "isActive":     true
}
```

### 3.3 Flights

The unit of delivery. Carries dates, priority, and inventory targeting.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/flights` | List. Filter: `campaignId`, `priorityId`, `externalId`, `isActive`, `activeAt=<RFC3339>`. |
| `POST` | `/v1/projects/{projectId}/flights` | Create. |
| `POST` | `/v1/projects/{projectId}/flights:batchUpsert` | Bulk. |
| `GET` | `/v1/projects/{projectId}/flights/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/flights/{id}` | Update. |

Body:

```json
{
  "externalId":  "flight-spring-2026-cardio",
  "campaignId":  444,
  "name":        "Spring Promo — Cardiology",
  "priorityId":  1,
  "startDate":   "2026-05-01T00:00:00Z",
  "endDate":     "2026-06-01T00:00:00Z",
  "siteIds":     [12],
  "zoneIds":     [34],
  "adTypes":     [16],
  "isActive":    true
}
```

`siteIds` / `zoneIds` empty means "any site/zone in the project."
`adTypes` is required and non-empty.

### 3.4 Ads

The flight↔creative binding plus a delivery weight.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/ads` | List. Filter: `flightId`, `creativeId`, `externalId`, `isActive`. |
| `POST` | `/v1/projects/{projectId}/ads` | Create. |
| `POST` | `/v1/projects/{projectId}/ads:batchUpsert` | Bulk. The hot management path for sync jobs. |
| `GET` | `/v1/projects/{projectId}/ads/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/ads/{id}` | Update. |

Body:

```json
{
  "externalId": "ad-2024-spring-1",
  "flightId":   333,
  "creativeId": 4242,
  "weight":     100,
  "isActive":   true
}
```

### 3.5 Creatives

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/creatives` | List. Filter: `advertiserId`, `type`, `externalId`, `isActive`. |
| `POST` | `/v1/projects/{projectId}/creatives` | Create. |
| `GET` | `/v1/projects/{projectId}/creatives/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/creatives/{id}` | Update. |
| `POST` | `/v1/projects/{projectId}/creatives/{id}/image` | Upload image (multipart). Returns `{ "imageUrl": "..." }`. |

Body (`oneOf` on `type`):

```json
{
  "externalId":      "creative-banner-728x90",
  "advertiserId":    555,
  "name":            "Spring banner — 728x90",
  "type":            "image",
  "imageUrl":        "https://cdn.example.com/banner.jpg",
  "width":           728,
  "height":          90,
  "alt":             "Acme Spring Sale",
  "clickThroughUrl": "https://acme.example.com/sale"
}
```

```json
{
  "type":            "html",
  "body":            "<div>...</div>",
  "clickThroughUrl": "https://..."
}
```

```json
{
  "type":            "native",
  "templateId":      7,
  "values": {
    "title":    "...",
    "body":     "...",
    "imageUrl": "...",
    "ctaText":  "Learn more"
  },
  "clickThroughUrl": "https://..."
}
```

`values` for `native` creatives is validated against the referenced
template's JSON Schema at write time; `422` on schema violation.

### 3.6 Creative Templates

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/creative-templates` | List. Filter: `name`, `externalId`. |
| `POST` | `/v1/projects/{projectId}/creative-templates` | Create. |
| `GET` | `/v1/projects/{projectId}/creative-templates/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/creative-templates/{id}` | Update; bumps `version`. |

Body:

```json
{
  "externalId": "template-sponsored-card-v1",
  "name":       "sponsored_card_v1",
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

### 3.7 Sites

Promoted from read-only — sites are per-project inventory and need to
be created/updated via API.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/sites` | List. Filter: `channelId`, `externalId`, `url`. |
| `POST` | `/v1/projects/{projectId}/sites` | Create. |
| `POST` | `/v1/projects/{projectId}/sites:batchUpsert` | Bulk by `externalId`. |
| `POST` | `/v1/projects/{projectId}/sites:upsertByUrl` | Upsert keyed by URL — natural-key endpoint for URL-driven flows. |
| `GET` | `/v1/projects/{projectId}/sites/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/sites/{id}` | Update. |

Body:

```json
{
  "externalId": "site-main",
  "channelId":  null,
  "name":       "Main Property",
  "url":        "https://example.com",
  "aliases":    ["https://www.example.com", "https://m.example.com"]
}
```

`url` and entries in `aliases` are unique within the project (across
both fields together).

`:upsertByUrl` body:

```json
{ "url": "https://example.com", "name": "Main Property" }
```

Returns the existing or newly-created site.

### 3.8 Zones

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/zones` | List. Filter: `siteId`, `externalId`. |
| `POST` | `/v1/projects/{projectId}/zones` | Create. |
| `POST` | `/v1/projects/{projectId}/zones:batchUpsert` | Bulk. |
| `GET` | `/v1/projects/{projectId}/zones/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/zones/{id}` | Update. |

Body:

```json
{
  "externalId": "zone-header",
  "siteId":     12,
  "name":       "Header"
}
```

### 3.9 Read-only inventory (Channel, Priority, AdType)

Network-level taxonomy. Read-only via API in v0; managed by operator
via CLI/SQL or seeded at project creation.

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/channels` | List. |
| `GET` | `/v1/projects/{projectId}/channels/{id}` | Read. |
| `GET` | `/v1/projects/{projectId}/priorities` | List. Ordered by tier. |
| `GET` | `/v1/projects/{projectId}/priorities/{id}` | Read. |
| `GET` | `/v1/projects/{projectId}/ad-types` | List. |
| `GET` | `/v1/projects/{projectId}/ad-types/{id}` | Read. |

Write endpoints for these resources are post-v0 (see roadmap).

---

## 4. Event Tracking

Unauthenticated; HMAC-signed in the URL. Browsers hit these directly
from the rendered ad.

### `GET /e/i/{signed}`

Impression ping.

- Default: `204 No Content`.
- With `?fmt=gif`: `200 OK`, `image/gif`, 43-byte 1×1 transparent GIF.
- Tampered or expired signature: `204` (silent), counter incremented.
- Replay within TTL: counted (deduped via `dedup_key`).

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
project_id | ad_id | creative_id | placement_id_hash | issued_at | nonce
```

URL-safe base64. Per-project secret. TTL is configurable per project
(default 24 h).

---

## 5. System

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/openapi.json` | OpenAPI 3.1 spec, generated from the binary. |
| `GET` | `/healthz` | Liveness. `200` if process is up. |
| `GET` | `/readyz` | Readiness. `200` only if snapshot loaded, DB reachable, flusher healthy. |
| `GET` | `/metrics` | Prometheus exposition. |
| `GET` | `/version` | Build metadata: git sha, build time, schema version. |

---

## 6. Resource Map (cheat sheet)

```
Org-level (Org token)
  POST   /v1/orgs/{orgId}/projects                    — provision a project
  GET    /v1/orgs/{orgId}/projects
  POST   /v1/orgs/{orgId}/projects:batchUpsert
  GET    /v1/orgs/{orgId}/projects/{projectId}
  PATCH  /v1/orgs/{orgId}/projects/{projectId}
  POST   /v1/orgs/{orgId}/tokens                      — mint API token
  GET    /v1/orgs/{orgId}/tokens
  DELETE /v1/orgs/{orgId}/tokens/{tokenId}
  GET/POST/PATCH/DELETE /v1/orgs/{orgId}/members

Decision (Org or Project token)
  POST   /v1/projects/{projectId}/decisions

Demand-side (full CRUD + batchUpsert)
  /v1/projects/{projectId}/advertisers
  /v1/projects/{projectId}/campaigns
  /v1/projects/{projectId}/flights
  /v1/projects/{projectId}/ads
  /v1/projects/{projectId}/creatives             (+ /image upload)
  /v1/projects/{projectId}/creative-templates

Inventory (full CRUD; sites also support :upsertByUrl)
  /v1/projects/{projectId}/sites
  /v1/projects/{projectId}/zones

Read-only inventory taxonomy
  /v1/projects/{projectId}/channels
  /v1/projects/{projectId}/priorities
  /v1/projects/{projectId}/ad-types

Events (public, HMAC-signed)
  GET    /e/i/{signed}
  GET    /e/c/{signed}

System
  GET    /openapi.json
  GET    /healthz
  GET    /readyz
  GET    /metrics
  GET    /version
```

---

## 7. Out of Scope (v0)

The following endpoints are explicitly **not** in v0 — they map onto
the roadmap items in `REQUIREMENTS.md` §11. Calls return `404` with
`code: "not_implemented"`.

- `POST /v1/projects/{projectId}/users/*` — UserDB.
- `POST /v1/projects/{projectId}/decisions:explain` — Decision Explainer.
- Frequency-cap configuration on flights.
- Geo / IP / day-parting / keyword / custom-property targeting fields.
- `POST /v1/projects/{projectId}/reports/*` — Reporting API.
- Webhook subscription endpoints.
- Custom event types beyond impression/click (`/e/x/{signed}`).
- Write endpoints for channels / priorities / ad-types.
- Site Group endpoints.
- Cross-project broadcast upsert.
