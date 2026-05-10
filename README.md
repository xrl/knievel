# knievel

> Fearlessly fast ad delivery that steals the show.

**knievel** is a self-hosted, multi-tenant ad-serving platform written
in Rust on top of `poem-openapi` and Postgres. The OpenAPI spec is the
contract: a single Rust binary, a generated client gem, and a Helm
chart deploy together to give a publisher direct ad delivery without
running an exchange. Suitable for teams running ad delivery for one
publisher or many — the multi-tenant rails are first-class, not bolted
on.

## Status

| | |
|---|---|
| **Version** | `0.1.6` (v0 — surface stabilizing, additive-forever once published) |
| **License** | MIT |
| **Postgres** | 14+ (vanilla; tested against 16 in CI, against Aurora in MIGRATION_RX.md) |
| **Architectures** | linux/amd64, linux/arm64 |
| **OpenAPI version** | 3.0 (poem-openapi 5 emits 3.0; revisits 3.1 when the library catches up) |
| **Ruby gem** | [`knievel`](https://rubygems.org/gems/knievel) — auto-generated, idiomatic `Enumerable` wrappers for cursor walks |

`v0` means the wire shape is stable and forward-compatible per the
[compatibility policy](#project-status-and-stability) but the
operational story (chaos rigs, real Aurora validation, performance
benchmarks) is still being filled in. See `PHASES.md` for the live
progress log.

## Why knievel

- **vs. Kevel** — domain model is openly inspired by Kevel's
  advertiser/campaign/flight/ad/site/zone shape, but knievel is
  self-hostable on your own Postgres. No third-party data path.
- **vs. building from scratch** — the boring parts ship: opaque-token
  auth, project-scoped row-level security, partition manager + leader
  election, snapshot loader, idempotency cache, `:batchUpsert` per
  resource, cursor pagination, and a generated Ruby gem with a
  hand-written `Enumerable` wrapper layer.
- **vs. a generic ad framework** — knievel is opinionated about
  Postgres + OpenAPI 3.0 + multi-tenant. No pluggable storage backend,
  no GraphQL second-API, no per-tenant code. The narrowness is the
  point.
- **vs. an OpenRTB stack** — knievel is publisher-side direct ad
  serving. No bid request flow, no SSP integration, no real-time
  auction. If you need OpenRTB you need a different system.

## What's in v0

**Decision API (the hot path)**
- `POST /v1/projects/{project_id}/decisions` — multi-placement decisions
  with priority-tiered selection, weighted random, blocklists, force
  overrides (gated three ways).
- `POST /v1/projects/{project_id}/decisions:explain` — debug companion
  showing every candidate and the rules applied. Same request shape;
  no event recorded.
- HMAC-signed `/e/i/{sig}` and `/e/c/{sig}` impression / click endpoints
  with stable `dedup_key` across signing-secret rotation.

**Management API**
- Full CRUD plus `:batchUpsert` for advertisers, campaigns, flights,
  ads, sites, zones (single Postgres transaction, per-row diagnostics
  matching `API.md` "Write contract").
- Cursor-paginated lists across the eight demand+inventory chains
  (`?limit=N&cursor=…`, default 50, max 500).
- Org/project lifecycle, opaque-token mint/list/revoke, taxonomy
  (channels, priorities, ad-types) seeded per project.
- Org-scoped Ad Library + project-scoped creative templates with
  arbitrary JSON Schema validation.
- Multipart creative-image upload to S3-compatible object stores.

**Events**
- Decision/impression/click rows buffered in-process, `COPY`'d to
  partitioned Postgres tables every 1–2s.
- Hourly rollup table for cheap reporting reads (`REPORTING.md`).
- In-process partition manager pre-makes 4 days of partitions, drops
  partitions past retention, leader-elected via Postgres advisory locks.

**Auth**
- Opaque-token bearers (`kvl_<env>_<scope>_<short-id>_<secret>`) hashed
  with argon2id at rest; argon2 verify on every request.
- Optional JWT principals via JWKS auto-discovery + claim mapping.
- Project-scoped Reader / Editor / Admin roles. RLS-enforced tenancy
  via `set_config('knievel.org_id', …)` and
  `set_config('knievel.project_id', …)` per-transaction.

**Multi-tenancy**
- Postgres `FORCE ROW LEVEL SECURITY` on every table that holds
  per-tenant data. Tested cross-tenant: 47 project-scoped endpoints,
  all with explicit `tests/cross_tenant_manifest.toml` coverage.
- `knievel-cli seed-demo` for fresh-install bootstrap with a sample
  fixture (drops a bearer + creates one of each resource so
  `docker compose up` produces a decisioning system in under a minute).
- `knievel-cli admin create-org --external-id <id> --name <name>` for
  production tenant provisioning — adds one row in `organizations`
  plus an org-admin bootstrap token, with no demo fixture chain.

**Observability**
- Structured JSON logs via `tracing`, OTLP-exported OpenTelemetry
  spans, Sentry error reporting. All three carry the same `request_id`
  / `trace_id` for correlation.
- `/healthz`, `/readyz`, `/version` endpoints; `/openapi.json` served
  live (so generated clients can re-key off the running deployment).

## What's deliberately not in v0

- **No OpenRTB** — knievel is direct serving.
- **No experiment framework** — A/B testing rides on top via the force
  overrides + `decisions:explain`, not as a built-in feature.
- **No reporting UI** — `events_raw` + `events_rollup_hourly` feed your
  warehouse; charts are downstream.
- **No second datastore** — no Redis, no Kafka, no separate snapshot
  service. Postgres + RAM only.
- **No experiment-mode persistence** — the snapshot is recreated from
  Postgres on boot; nothing in-memory is durable.
- **Per-row idempotency on POST creates** — POST returns
  `409 external_id_conflict` today; `:batchUpsert` is the canonical
  idempotent surface. POST-side parity moves to Phase 6.1.

See `REQUIREMENTS.md` § 11 for the full not-in-v0 list and the
rationale on each.

## Quickstart (5 minutes)

Brings up Postgres + knievel + a seed sidecar that creates one of each
resource and drops a bearer at `tmp/knievel-dev-token`:

```sh
git clone https://github.com/knievel-ads/knievel
cd knievel
docker compose -f examples/compose/compose.yaml up -d
# wait ~10s for postgres + knievel /healthz to come up
curl -fsS http://localhost:8080/healthz
# => ok
```

Grab the seeded token + the deterministic project id (the seed-demo
CLI uses `sha256(...)[:12]` over the well-known external ids):

```sh
TOKEN="$(cat tmp/knievel-dev-token)"
ORG_ID="org_$(printf demo-org | sha256sum | cut -c-12)"
PROJECT_ID="pj_$(printf "${ORG_ID}/demo-project" | sha256sum | cut -c-12)"
echo "$ORG_ID / $PROJECT_ID"
```

List the seeded advertiser:

```sh
curl -fsS \
  -H "Authorization: Bearer ${TOKEN}" \
  "http://localhost:8080/v1/projects/${PROJECT_ID}/advertisers"
```

```json
{
  "items": [
    {"id": 1, "external_id": "demo-advertiser", "name": "Demo Advertiser", "is_active": true, "etag": "..."}
  ],
  "next_cursor": null
}
```

The same flow from Ruby — `gem install knievel` and the
`Enumerable` wrapper hides the cursor walk:

```ruby
require "knievel/client"

client = Knievel::Client.new(host: "http://localhost:8080", access_token: ENV.fetch("TOKEN"))
client.advertisers(ENV.fetch("PROJECT_ID")).each { |adv| puts adv.name }
client.advertisers(ENV.fetch("PROJECT_ID")).first(10)
client.advertisers(ENV.fetch("PROJECT_ID")).lazy.select(&:is_active).first(20)
```

Tear down:

```sh
docker compose -f examples/compose/compose.yaml down -v
```

## Hello-world decision

The seeded fixture includes one site, one zone, one ad. On a fresh
`compose up -v`, those land at id `1`. A minimal decision request hits
that ad:

```sh
curl -fsS -X POST \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "placements": [
      {"id": "header", "site_id": 1, "ad_types": [1]}
    ]
  }' \
  "http://localhost:8080/v1/projects/${PROJECT_ID}/decisions"
```

```json
{
  "snapshot_version": 42,
  "decisions": {
    "header": [
      {
        "ad_id":          1,
        "creative_id":    1,
        "flight_id":      1,
        "campaign_id":    1,
        "advertiser_id":  1,
        "site_id":        1,
        "external_id":    "demo-ad",
        "click_url":      "http://localhost:8080/e/c/AbCd…",
        "impression_url": "http://localhost:8080/e/i/EfGh…",
        "creative":       { "kind": "image", "image_url": "…", "width": 300, "height": 250 }
      }
    ]
  }
}
```

`API.md` § "Decisions" documents the full request and response shape
including blocklists, force overrides, multi-placement, count, and the
`:explain` companion. A documented contract gap: API.md uses camelCase
in examples (`site_external_id`, `ad_types`, `snapshot_version`) but the
generated `openapi.yaml` and the wire format are snake_case.
Reconciling the two is a follow-up; the snake-case shape is what
ships today.

## Architecture in one diagram

```
┌──────────────┐     POST /v1/projects/{p}/decisions    ┌──────────────┐
│ Calling app  │ ──────────────────────────────────────▶│   knievel    │
│ (gem / curl) │ ◀──────────────────────────────────────│ (Rust/poem)  │
└──────────────┘                                        └──────┬───────┘
       ▲                                          in-mem       │
       │                                          snapshot     ▼
       │                                                  ┌──────────────┐
       └─ impression/click pings ─────────────────────────▶│   Postgres   │
          GET /e/i/<sig>                                   │  (config +   │
          GET /e/c/<sig>                                   │  partitioned │
                                                           │   events)    │
                                                           └──────────────┘
```

Decisions touch RAM only — the configuration snapshot is keyed by
`(project_id, resource)` and refreshed on Postgres `LISTEN`/`NOTIFY`
plus a 30-second poll backstop. Events are buffered in an in-process
channel and `COPY`'d to partitioned tables every 1–2s. The partition
manager runs in-process under a Postgres advisory-lock leader election;
no second datastore. See `ARCHITECTURE.md`.

## Deployment

- **Container image:** `ghcr.io/knievel-ads/knievel:vX.Y.Z` (multi-arch,
  cosign-signed, build-provenance-attested). Tagged on semver tags
  only — no `main`-branch images.
- **Helm chart:** `charts/knievel/` (OCI-published; `helm install
  oci://ghcr.io/knievel-ads/charts/knievel`).
- **Reference compose stack:** `examples/compose/` for local + bring
  -your-own-Postgres single-node deployments.
- **Operator-supplied Postgres** (14+) and an S3-compatible object
  store for creative images.

See `DEPLOYMENT.md` for the full operational picture.

## Documentation map

| Doc | Audience | What it covers |
|---|---|---|
| `README.md` | first-time reader | This file. |
| `REQUIREMENTS.md` | platform contract | The complete spec for a v0 platform. Authoritative. |
| `API.md` | API consumer | Wire surface, error taxonomy, write contract, idempotency, pagination. |
| `AUTH.md` | API consumer + integrator | Token shape, JWT mode, role matrix. |
| `REPORTING.md` | data engineer | Event schema, rollup tables, downstream warehouse contract. |
| `ARCHITECTURE.md` | operator + contributor | How the binary is built and what runs in-process. |
| `DEPLOYMENT.md` | operator | Helm + compose, Postgres sizing, image storage, observability. |
| `TESTING.md` | contributor | Test plan, slice naming, CI gate matrix, acceptance suite. |
| `PHASES.md` | contributor | Live progress log; one commit per task. |
| `CONTRIBUTING.md` | contributor | Setup, gates, commit conventions. |
| `MIGRATION_RX.md` | RX-style consumers | Per-consumer migration recipe; not part of the platform contract. |

`REQUIREMENTS.md`, `API.md`, `AUTH.md`, and `REPORTING.md` are the
**platform contract** — they describe knievel as a generic
multi-tenant ad platform and stay free of consumer-specific
identifiers. RX is the first integration but not the only intended
consumer.

## Project status and stability

`0.1.x` is "surface stabilizing." The wire contract is forward-
compatible from this point: every change is **additive** unless a
deprecation window has elapsed. Concretely:

- **Always allowed**: new endpoints, new optional request fields, new
  response fields (clients must ignore unknown fields — the generated
  gem already does), new variants in a `oneOf`, new error codes inside
  an existing category.
- **Allowed under a 6-month deprecation window**: removing or renaming
  a field, removing or renaming an endpoint. Deprecations carry HTTP
  `Deprecation: true` and `Sunset: <RFC-3339-date>` headers and are
  marked `deprecated: true` in the OpenAPI spec.
- **Never allowed in v0**: changing the HTTP status code returned for
  an existing error category, changing default values in a way that
  changes behavior, removing backward-compatible behavior.

Generated client compatibility is governed by **major.minor mirror**:
gem `X.Y.*` is generated from server spec `X.Y` and works against any
server `>= X.Y`. Patch versions of the gem are gem-internal (helper
changes, dep bumps).

`v0.1.0` ships when Phase 5 closes — see `PHASES.md` for the running
list. Phase 4 (deployable) and Phase 3 (full surface + cross-tenant
suite) are complete.

## Building from source

```sh
# Pre-PR CI parity (fast):
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
cargo xtask lint-migrations
cargo xtask check-cross-tenant
cargo xtask test-shape
cargo xtask openapi --check

# Integration tests against a real Postgres:
DATABASE_URL=postgres://knievel_app:dev@localhost:5432/knievel \
  cargo test --workspace
```

`CONTRIBUTING.md` covers the per-task commit convention, the migration
linter rules, and the cross-tenant manifest gate.

## License + acknowledgements

Knievel is MIT-licensed. The domain model (advertiser → campaign →
flight → ad; site / zone / priority / ad-type) is openly inspired by
[Kevel](https://www.kevel.com/), with thanks. The Rust web stack rests
on [`poem`](https://github.com/poem-web/poem),
[`poem-openapi`](https://docs.rs/poem-openapi/), and
[`sqlx`](https://github.com/launchbadge/sqlx); generated clients ride
on [openapi-generator](https://openapi-generator.tech/).
