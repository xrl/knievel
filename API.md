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

- **Management + Decision endpoints:** `Authorization: Bearer <credential>`.
  Two coexisting credential types are accepted (per-deployment config
  picks which are enabled):
  - **Opaque tokens** — prefix `kvl_<env>_<scope>_<random>`. DB
    lookup. Org-scoped tokens may address any Project in their Org;
    Project-scoped tokens are limited to their own Project.
  - **JWTs** — standard three-segment JWT. Validated statelessly
    against the issuer's JWKS (Keycloak or any OIDC provider).
    Authorization context is read from a `knievel` claim (`scope`,
    `org_id`, `project_id`, `role`). See `AUTH.md`.
  - Wrong org or project for the credential → `403`.
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

```json,ignore
{
  "items": [ ... ],
  "nextCursor": "eyJ..." | null
}
```

`limit` defaults to 50, max 500 (`limit=0` and `limit > 500` are
rejected with `400 invalid_limit`). No `totalRecords` — counting
is expensive and rarely useful at this layer.

Cursors are opaque (`base64url(JSON{kind, last_id})` internally,
but consumers should treat them as black boxes). The server
rejects a cursor whose `kind` doesn't match the endpoint that
received it (`400 invalid_cursor`) — this catches the
"`listAdvertisers` cursor pasted into `listCampaigns`" footgun
without requiring HMAC-signed cursors.

The cursor only carries the resume key — **changing filters
between pages is the caller's responsibility**. A cursor minted
under one filter set, replayed against a different filter set on
the same endpoint, may skip rows or return duplicates relative
to a fresh-walk-with-the-new-filter. Consumers walking with
filters should keep the filter set stable across the walk.

**Non-paginated list endpoints (v0):** taxonomy
(`listChannels`, `listPriorities`, `listAdTypes`) return the
full set in semantic order (priorities by `tier`); the response
shape still carries `nextCursor` (always `null`) so wrappers
degenerate to a single-page walk. `listAdLibraryItems` and
`listTokens` are also un-cursored in v0 because their primary
keys are TEXT (`(created_at, id)` cursor lands in Phase 6.5).

**Future:** generated client wrappers (e.g. the Ruby gem in
`REQUIREMENTS.md` §8 item 3) eventually key off
`x-knievel-paginated: true` /
`x-knievel-paginated-items: items` /
`x-knievel-paginated-cursor: nextCursor` vendor extensions on
each paginated operation, so a second-language binding or a
doc-site generator can discover paginated endpoints from the
spec alone. **Not shipped today** — poem-openapi 5 lacks an
operation-level extension API, and we're upstreaming it first
rather than carrying a post-processor in `cargo xtask openapi`.
Tracked in PHASES.md § 6.6. Until then, the Ruby gem's
hand-written wrapper hardcodes its paginated set.

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

### Write contract

Every mutation — single `POST` / `PATCH` and any `:batchUpsert` —
runs in **exactly one Postgres transaction**. There are three rules:

1. **All-or-nothing per call.** If any row in a `:batchUpsert` (or
   any cross-entity FK in a single-row write) fails validation, the
   entire transaction rolls back. Partial state never leaks into the
   snapshot. There are no "best-effort" semantics on any wire
   endpoint; the gem-side `upsertWithFlightAndCreative` helper is
   self-healing across multiple wire calls, but each individual wire
   call is atomic.
2. **Cross-entity FKs validated inside the transaction.** Creating
   a flight that references a campaign, an ad referencing a
   creative, a creative referencing a CreativeTemplate — every
   reference is verified against the same transaction's view, so a
   campaign created earlier in the same `:batchUpsert` is visible to
   a flight defined later in the array. This means callers can ship
   a logically coherent unit (advertiser → campaign → flight → ad
   → creative) in one batch.
3. **Per-row diagnostics.** When a batch fails, the error body lists
   each offending row with deterministic structure:

```json
{
  "error": {
    "code": "batch_partial_failure",
    "message": "1 of 12 rows failed validation",
    "requestId": "01JABCDEF...",
    "details": [
      {
        "index":   3,
        "field":   "campaignId",
        "code":    "fk_not_found",
        "message": "campaignId 12345 does not exist in this project"
      }
    ]
  }
}
```

`details[].index` is the position in the request array.
`details[].code` is one of: `fk_not_found`, `external_id_conflict`,
`validation_failed`, `unique_violation`, `if_match_mismatch`. The
absence of a row in `details[]` means it would have committed
successfully; idempotent retries can skip already-applied rows by
omitting them from the next request, since `externalId` upserts are
already idempotent.

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
| `placements[].force.*` | int64 \| null | no | Debug overrides. Three-control gate: (1) project's `allow_force_decision` flag must be true; (2) Project Admin role or higher; (3) call is recorded in `knievel.audit_log` with actor, payload hash, and reason (passed via optional top-level `force_reason` string). Knievel rejects with `403 / force_disabled` if any control fails. A global kill-switch (`decisions.force_overrides_enabled: false`) disables the path entirely. |
| `block.creativeIds` | int64[] | no | Caller-derived suppression list. |
| `block.advertiserIds` | int64[] | no | |
| `block.campaignIds` | int64[] | no | |

`context` is informational only — it is **never** used for tenant
routing. The project ID in the path is the sole authoritative tenant
signal.

**Response (200):**

```json
{
  "snapshotVersion": 1234567,
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

- `snapshotVersion` is the monotonic configuration version that
  served this request. Stamped on the corresponding `events_raw`
  rows and emitted on per-request log lines so an operator can
  reproduce a decision deterministically: same input + same
  `snapshotVersion` → same output.
- `decisions[<id>]` is **always an array**, even when `count == 1`.
  Empty array = no eligible ad.
- `siteId` is the resolved site (useful when caller passed `siteUrl` or
  `siteExternalId`).
- `creative` is a `oneOf`:
  - `{"type":"image","imageUrl":..,"width":..,"height":..,"alt":..,"clickThroughUrl":..}`
  - `{"type":"html","body":..,"clickThroughUrl":..}` — `body` is the
    static HTML stored on the creative, returned verbatim.
  - `{"type":"native","template":..,"values":{...},"clickThroughUrl":..}`
    — caller renders `values` client-side using its own components.
  - `{"type":"templated","template":..,"values":{...},"body":..,"clickThroughUrl":..}`
    — server renders the referenced template's Liquid source against
    `values` at decision time and returns the result in `body`.
    `values` is also echoed so callers can re-render or inspect.
- All URL fields are absolute.

### `POST /v1/projects/{projectId}/decisions:explain`

A debug companion to `decisions`. Accepts the **same request body**
and returns the same `decisions` payload, plus a per-placement
`explanation` array showing every candidate ad and the rules that
were applied to it. No event is recorded; no impression / click URL
is minted (the URLs returned are dummy placeholders, marked as such).

Same auth as `decisions` (any role with read access to the project).
Rate-limited more aggressively than production decisions (default
60 req/min per token).

**Response (200):**

```json,ignore
{
  "snapshotVersion": 1234567,
  "decisions":       { "header": [ ...same shape as /decisions... ] },
  "explanation": {
    "header": {
      "priorityTier":  1,
      "selectedAdId":  9001,
      "candidates": [
        {
          "adId":         9001,
          "creativeId":   4242,
          "flightId":     333,
          "campaignId":   444,
          "advertiserId": 555,
          "weight":       100,
          "evaluation": [
            { "rule": "flight_active",      "result": "pass" },
            { "rule": "site_match",         "result": "pass" },
            { "rule": "ad_type_match",      "result": "pass" },
            { "rule": "block_creative_ids", "result": "pass" },
            { "rule": "weighted_random",    "result": "selected" }
          ]
        },
        {
          "adId":         9002,
          "creativeId":   4243,
          "flightId":     333,
          "campaignId":   444,
          "advertiserId": 555,
          "weight":       100,
          "evaluation": [
            { "rule": "flight_active", "result": "pass" },
            { "rule": "site_match",    "result": "fail",
              "detail": "site_id 12 not in flight.site_ids [99]" }
          ]
        }
      ]
    }
  }
}
```

`evaluation` entries appear in evaluation order; `result` is one of
`pass`, `fail`, `selected`, `not_selected`. `detail` is populated on
fails and on rules that produced random outcomes. The shape is meant
to be diff-friendly between two requests so traffickers can spot
"what changed."

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

### 2.4 Ad Library

An Org-scoped catalog of reusable creative content. Project Ads can
reference library items in lieu of inlining a creative
(`REQUIREMENTS.md` §5.1).

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/orgs/{orgId}/ad-library/items` | List. Filter: `externalId`, `isActive`. |
| `POST` | `/v1/orgs/{orgId}/ad-library/items` | Create. |
| `POST` | `/v1/orgs/{orgId}/ad-library/items:batchUpsert` | Bulk by `externalId`. |
| `GET` | `/v1/orgs/{orgId}/ad-library/items/{itemId}` | Read. |
| `PATCH` | `/v1/orgs/{orgId}/ad-library/items/{itemId}` | Update. |
| `GET` | `/v1/orgs/{orgId}/ad-library/items/{itemId}/references` | List the Project Ads referencing this item. |

Body — same `oneOf` creative shape as Project Creatives, plus
catalog metadata:

```json
{
  "externalId":  "library-spring-banner",
  "name":        "Spring banner — 728x90",
  "description": "Reusable cross-project spring banner",
  "creative": {
    "type":            "image",
    "imageUrl":        "https://cdn.example.com/banner.jpg",
    "width":           728,
    "height":          90,
    "alt":             "Spring sale",
    "clickThroughUrl": "https://acme.example.com/sale"
  },
  "isActive": true
}
```

`native` template values are validated against the referenced
template; the template must exist in **every Project that
references this item**. The `references` endpoint helps spot
references that would break if the item is archived.

Modifying a library item is reflected in all referring Ads after
the next snapshot swap (typically <5 s; see `REQUIREMENTS.md` §7.2).
There is no per-reference override of creative content — that's
what the inline form is for.

Manage with Org Admin role; Project tokens cannot mutate the library
but can read it (so referring Ads' creative content can be
introspected in the admin UI).

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

The flight↔creative binding plus a delivery weight. Each Ad either
**inlines** a project-scoped creative or **references** an item in
the org's Ad Library (§2.4).

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/v1/projects/{projectId}/ads` | List. Filter: `flightId`, `creativeId`, `adLibraryItemId`, `externalId`, `isActive`. |
| `POST` | `/v1/projects/{projectId}/ads` | Create. |
| `POST` | `/v1/projects/{projectId}/ads:batchUpsert` | Bulk. The hot management path for sync jobs. |
| `GET` | `/v1/projects/{projectId}/ads/{id}` | Read. |
| `PATCH` | `/v1/projects/{projectId}/ads/{id}` | Update. |

Body — `oneOf` on `creativeId` vs `adLibraryItemId` (exactly one
required):

```json,ignore
// Inline creative.
{
  "externalId": "ad-2024-spring-1",
  "flightId":   333,
  "creativeId": 4242,
  "weight":     100,
  "isActive":   true
}
```

```json,ignore
// Reference an org-shared library item.
{
  "externalId":      "ad-2024-spring-1",
  "flightId":        333,
  "adLibraryItemId": "ali_AbCd...",
  "weight":          100,
  "isActive":        true
}
```

Library references are resolved through the in-memory snapshot at
decision time; no extra round-trip. Updating a library item updates
all referencing Ads after the next snapshot swap.

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

```json
{
  "type":            "templated",
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

`values` for `native` and `templated` creatives is validated against
the referenced template's JSON Schema at write time; `422` on schema
violation. `templated` additionally requires the referenced template
to carry a non-null `template` (Liquid source) field — `422 /
template_missing_body` if the referenced template is
input-validation-only.

`templated` differs from `native` only in **who renders**:

- `native` — server returns `values`; the caller renders client-side.
  The decision response carries no `body`.
- `templated` — server renders `template` (Liquid) against `values`
  at decision time and returns the resulting string in `body` on the
  decision response. `values` is echoed too so a caller can fall back
  to its own renderer or display debug info.

Templated rendering happens only on the decision path (`POST
/v1/projects/{projectId}/decisions`); the creative resource itself
stores nothing rendered — the `body` field on the wire is not
persisted on the creative row.

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
  },
  "template":       "<a href=\"{{ad.clickUrl}}\"><img src=\"{{values.imageUrl}}\" alt=\"{{values.title}}\"><span>{{values.title}}</span><p>{{values.body}}</p><span class=\"cta\">{{values.ctaText}}</span><img src=\"{{ad.impressionUrl}}\" width=\"1\" height=\"1\"></a>",
  "templateEngine": "liquid"
}
```

`schema` is the JSON Schema validated against `values` at creative
write time. It is required.

`template` is an optional Liquid source string. When present, this
template can be referenced by `templated` creatives, which render it
server-side at decision time. When absent, the template is
input-validation-only and can only be referenced by `native`
creatives. `template` is parsed and rejected with `422 /
template_parse_error` at write time if it does not parse.

`templateEngine` is required when `template` is present; today the
only accepted value is `"liquid"` (DotLiquid-compatible). The field
exists so additional engines can be added later without a breaking
schema change.

Helpers exposed inside `template`:

- `ad.id`, `ad.clickUrl`, `ad.impressionUrl` — engine-injected at
  decision time; the URLs are the same signed values returned at the
  top level of the decision response.
- `placement.id` — echo of the request placement key.
- `decision.snapshotVersion` — the current `snapshotVersion`.
- `values.*` — the creative's `values` object, after JSON-Schema
  validation.

No file, network, or environment access is exposed. Templates run in
a sandbox with hard caps on render time and output size; both caps
are configurable via `config.yaml` and surfaced in `/version`.

Mutating `schema` does **not** retroactively re-validate existing
creatives; it applies to subsequent writes only. The same applies to
`template` — changes affect subsequent decisions only; in-flight
creative rows are not invalidated. Use a new template name for
breaking changes.

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

- Default: `204 No Content` (returned in all cases below; the ping
  is fire-and-forget from the browser's perspective).
- With `?fmt=gif`: `200 OK`, `image/gif`, 43-byte 1×1 transparent GIF.
- Tampered or expired signature: `204` (silent), counter incremented.

### `GET /e/c/{signed}`

Click ping.

- Records the click, `302` redirect to the creative's
  `clickThroughUrl`.
- Tampered or expired signature: `400`.
- Optional `?u=<url>` overrides the redirect target only if signed into
  the payload (prevents open-redirect abuse).
- Replays still 302 to the same URL (so users who hit Back-then-click
  still land at the destination); see "Replay, dedup, and counts" for
  what gets counted.

### Replay, dedup, and counts

There is **one canonical count** per event kind: a row is a
"countable" event iff `is_duplicate = false`. Reporting queries can
present either the canonical count or the raw count (which includes
duplicates) depending on use case; billing always uses the canonical
count.

| Field | Source |
|---|---|
| `dedup_key` | `HMAC-SHA256(per-project-secret, kind || signature_nonce)` truncated to 16 bytes. Stable for the lifetime of a signed URL. |
| Uniqueness | `(project_id, kind, dedup_key)` is unique within `events_raw`. The first hit lands with `is_duplicate = false`; subsequent hits with the same `(kind, dedup_key)` land with `is_duplicate = true`. |
| Window | Lifetime within retention (default 30 days). After the partition is dropped, dedup state is gone — but so is the original event, so this is moot. |

Behavior summary by event kind:

- **Impression**: every hit is recorded. First → `is_duplicate=false`,
  countable. Subsequent → `is_duplicate=true`, not counted for billing
  but visible in raw analysis.
- **Click**: every hit is recorded *and* every hit redirects (the
  user expects to land somewhere). First → `is_duplicate=false`,
  countable. Subsequent → `is_duplicate=true`, redirected, not
  counted for CTR.
- **Custom events** (post-v0): same shape.

Canonical SQL conventions:

```sql
-- Billable / reportable count (default).
SELECT count(*) FROM events_raw
WHERE kind = 2 AND ts >= ... AND NOT is_duplicate;

-- Raw traffic volume (analytics curiosity, abuse triage).
SELECT count(*) FROM events_raw
WHERE kind = 2 AND ts >= ...;
```

`events_rollup` aggregates the canonical (non-duplicate) count only.
Raw analysis goes against `events_raw` directly.

### Signature payload

HMAC-SHA256 over a compact binary record:

```
project_id | ad_id | creative_id | placement_id_hash | issued_at | nonce
```

URL-safe base64. Per-project secret. TTL is configurable per project
(default 24 h). The 8-hour rotation overlap (REQUIREMENTS.md §6.3)
applies to the signature secret; `dedup_key` is independent of which
secret the URL was signed under, so dedup spans rotation cleanly.

---

## 5. System

| Verb | Path | Purpose |
|---|---|---|
| `GET` | `/openapi.json` | OpenAPI 3.1 spec, generated from the binary. |
| `GET` | `/healthz` | Liveness. `200` if process is up. |
| `GET` | `/readyz` | Readiness. `200` only if snapshot loaded, DB reachable, flusher healthy. |
| `GET` | `/metrics` | Prometheus exposition. |
| `GET` | `/version` | Build metadata (git sha, build time, schema version) plus the **effective auth policy** — enabled modes and per-issuer summary (issuer URL, audience, algorithms, claim source, JWKS URL). Secrets are never returned. See `AUTH.md` "Startup Linting and Effective-Policy Visibility". |

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
- Frequency-cap configuration on flights.
- Geo / IP / day-parting / keyword / custom-property targeting fields.
- `POST /v1/projects/{projectId}/reports/*` — Reporting API.
- Webhook subscription endpoints.
- Custom event types beyond impression/click (`/e/x/{signed}`).
- Write endpoints for channels / priorities / ad-types.
- Site Group endpoints.
- Cross-project broadcast upsert.
