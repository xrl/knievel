# Architecture

This doc describes how knievel is built and what runs in-process. The
audience is an evaluator or operator who's read the README and is
asking "will it work in my stack?" Diagrams first; code where
behavior is non-obvious. Detailed spec rules live in
`REQUIREMENTS.md`; this doc is the picture.

## 1. Where knievel sits

```
                    ┌─────────────────────┐
                    │ Calling app         │
                    │ (Rails/Node/Go/...) │
                    └──────────┬──────────┘
                               │  HTTPS, Bearer token
                               ▼
                ┌─────────────────────────────┐
                │           knievel           │
                │ ┌─────────────────────────┐ │
                │ │ poem + poem-openapi     │ │
                │ │ in-mem snapshot         │ │
                │ │ event channel + flusher │ │
                │ │ partition manager       │ │
                │ │ idempotency cache       │ │
                │ │ HMAC URL signer         │ │
                │ └─────────────────────────┘ │
                └────┬─────────────────┬──────┘
                     │                 │
            SQL +    │                 │  S3 PUT/GET
            LISTEN   ▼                 ▼
            ┌────────────────┐  ┌────────────────────┐
            │   Postgres     │  │ S3-compatible store │
            │ (config rows + │  │ (creative images)   │
            │  partitioned   │  └────────────────────┘
            │  events)       │
            └───────┬────────┘
                    │ COPY downstream (operator-owned)
                    ▼
           ┌─────────────────────┐
           │  Warehouse / BI     │
           └─────────────────────┘

       ┌────────────────────────┐    ┌────────────────────┐
       │   OTel collector       │    │      Sentry        │
       │ (metrics + traces)     │    │ (error reports)    │
       └────────────────────────┘    └────────────────────┘
              ▲                                ▲
              │ OTLP                           │ HTTP
              └────────────────────────────────┘
                          all from knievel
```

The trust boundary is **server-to-server** in v0. Every HTTP call to
knievel comes from a calling application authenticated by a bearer
token; browser-direct calls are out of scope (no CORS, no first-party
cookies, no public ad-decision endpoint). The `/e/i/{sig}` and
`/e/c/{sig}` event endpoints are unauthenticated by design — they're
hit from end-user browsers — but the HMAC signature in the URL is the
authorization, not session state.

Postgres, the object store, the OTel collector, and Sentry are all
operator-supplied. Knievel ships with sensible defaults but doesn't
embed any of them.

## 2. Component map

In-process pieces, all running in the same Rust binary:

| Component | Source | Responsibility |
|---|---|---|
| HTTP server | `src/server.rs`, `src/handlers.rs` | `poem` route table; OpenAPI spec at `/openapi.json`; per-request authz prologue. |
| Snapshot | `src/snapshot.rs`, `src/state.rs` | In-memory `(project_id, resource)` map. The decision hot path reads from here. |
| Snapshot loader | `src/snapshot.rs` | LISTEN on `knievel.snapshot_changes`; 5 s poll backstop; atomic swap on diff. |
| Event channel + flusher | `src/events.rs`, `src/event_endpoints.rs` | Per-event channel sender; background flusher batches via `COPY` every 1–2 s. |
| Partition manager | `src/partitions.rs`, `src/leader.rs` | Pre-makes 4 days of `events_raw` leaf partitions; drops past-retention; runs only on the elected leader. |
| Idempotency cache | `src/idempotency.rs` | Replay store keyed on `(project, key, route, body-hash)`; 24 h TTL. |
| Auth | `src/auth/` | Opaque-token verify (argon2id), JWT verify (JWKS auto-discovery), role mapping. |
| HMAC URL signer | `src/hmac.rs` | Mints + verifies impression/click URL signatures with stable `dedup_key` across signing-secret rotation. |
| Image upload | `src/image_upload.rs` | Multipart receive → magic-byte sniff → S3 PUT through the configured store. |
| Migrations | `src/migrate.rs` | `sqlx::migrate!` runner; `auto_migrate: true` on startup or invoked via `knievel-cli migrate`. |

There is no second datastore. The snapshot is rebuilt from Postgres on
boot; the event channel buffer is bounded and lossy under saturation
(see § 10).

## 3. Hot path: a decision request, end to end

```
 caller                                                     knievel                                            postgres
   │                                                          │                                                  │
   │  POST /v1/projects/{p}/decisions  (Bearer kvl_…)         │                                                  │
   ├─────────────────────────────────────────────────────────▶│                                                  │
   │                                                          │ ➀ parse + verify bearer (argon2id) ──────────────│
   │                                                          │ ➁ resolve principal → (org, project, role)        │
   │                                                          │ ➂ read snapshot[project_id]            (in-memory)│
   │                                                          │ ➃ filter ads (site/zone/ad_type/date/blocks)      │
   │                                                          │ ➄ priority tier + weighted-random selection       │
   │                                                          │ ➅ HMAC-mint /e/i/* and /e/c/* URLs                │
   │                                                          │ ➆ enqueue `decision` event (lossy on full chan)   │
   │                          200 OK + decisions[]             │                                                  │
   │◀─────────────────────────────────────────────────────────┤                                                  │
   │                                                          │                                                  │
```

Steps 1–6 are pure RAM after the auth-table read. The auth read is the
only DB hop on the hot path; it's covered by a per-request transaction
that binds `(org_id, project_id)` GUCs for RLS. Step 7 is non-blocking
— the channel has 8 192 default slots; over-saturation drops events
and the metric `events.dropped` ticks (see § 10).

Sub-millisecond budget targets (per `REQUIREMENTS.md` § 9; "TARGET
(unverified)" until the bench in `bench/results/v0.1.md` lands):

- p50 ≤ 8 ms
- p99 ≤ 25 ms
- p99.9 ≤ 75 ms

Steps 1–2 dominate the budget today (auth verify + DB round-trip).
Step 3 is a single hashmap read.

## 4. Cold path: an event ping, end to end

```
 browser                                                    knievel                                           postgres
   │                                                          │                                                  │
   │  GET /e/i/<signed>                                       │                                                  │
   ├─────────────────────────────────────────────────────────▶│                                                  │
   │                                                          │ ➀ HMAC verify (current + previous secret window) │
   │                                                          │ ➁ look up dedup_key in events.dedup table        │
   │                                                          │ ➂ enqueue event (silent on dup or invalid sig)   │
   │                                                          │ ➃ respond 204 (or transparent GIF on ?fmt=gif)   │
   │                          204 No Content                  │                                                  │
   │◀─────────────────────────────────────────────────────────┤                                                  │
   │                                                          │                                                  │
   │                                          (every 1–2 s)   │                                                  │
   │                                                          │ ➄ flusher COPY-batch enqueued events ────────────▶│
   │                                                          │                              INSERT events_raw   │
   │                                                          │                                                  │
   │                                          (hourly)        │                                                  │
   │                                                          │ ➅ rollup leader: events_raw → events_rollup_*   ▶│
```

The endpoint **always** returns 204 (or the transparent GIF if
`?fmt=gif`) — even on bogus signatures — so an attacker can't probe
for valid `dedup_key`s by status code. Invalid sigs ignore the event
silently; the metric `events.bad_signature` ticks.

## 5. Configuration lifecycle

Management writes don't touch the snapshot directly. They go through
the same DB write path every CRUD endpoint uses, and the snapshot
catches up via Postgres notifications.

```
   ┌────────────┐         ┌────────────┐        ┌───────────────────┐
   │ caller     │  PATCH  │ knievel    │  SQL   │ Postgres          │
   │ (Ruby gem) │ ───────▶│ HTTP layer │ ──────▶│  - row UPDATE     │
   └────────────┘         └────────────┘        │  - NOTIFY snap_ch │
                                                 └─────────┬─────────┘
                                                           │
                                                           │ LISTEN
                                                           ▼
                                                 ┌───────────────────┐
                                                 │ snapshot loader   │
                                                 │  (every knievel)  │
                                                 │ - diff-pull rows  │
                                                 │ - new map built   │
                                                 │ - atomic swap     │
                                                 └───────────────────┘
```

The bound: **5 s worst-case staleness.** LISTEN/NOTIFY is the fast
path; a 5 s poll backstop catches dropped notifications (Aurora
failovers drop them silently — see `CLAUDE.md` cross-cutting risk 2).
Decisions made within that 5 s window can read pre-update state.

The snapshot swap is wholesale (build a new map, replace the
`Arc<Snapshot>`). No per-key locking on the read path; readers see a
consistent view of one version or the other.

## 6. Storage model

One Postgres schema, `knievel`, holds everything. Every table has
`FORCE ROW LEVEL SECURITY` enabled with a policy keyed on
`current_setting('knievel.org_id')` and (for project-scoped tables)
`current_setting('knievel.project_id')`. The handler layer's
`open_project_tx` prologue sets both GUCs at the start of every
project-scoped request.

```
          ┌──────────────────────────────────────────┐
          │ schema: knievel                          │
          │                                          │
          │  ┌──────────┐    ┌──────────┐  ┌───────┐ │
          │  │ orgs     │ ─▶ │ projects │  │ tokens│ │
          │  └──────────┘    └────┬─────┘  └───────┘ │
          │                       │                  │
          │  ┌────────────────────┼──────────────┐   │
          │  │  per-project: advertisers,        │   │
          │  │   campaigns, flights, ads,        │   │
          │  │   creatives, creative_templates,  │   │
          │  │   sites, zones, taxonomy          │   │
          │  └───────────────────────────────────┘   │
          │                                          │
          │  ┌────────────┐  ┌──────────────────┐    │
          │  │ ad_library │  │ events_raw       │    │
          │  │ (org)      │  │ (partitioned)    │    │
          │  └────────────┘  └──────────────────┘    │
          │                  ┌──────────────────┐    │
          │                  │ events_rollup_*  │    │
          │                  └──────────────────┘    │
          └──────────────────────────────────────────┘
```

`events_raw` is **partitioned by date**. Leaf partitions are managed
in-process by the partition manager (see § 7). The retention default
is 30 days; older partitions are dropped, not archived. Operators who
want event history for compliance run a downstream `COPY`-to-warehouse
job (see `REPORTING.md`).

`events_rollup_hourly` is a leader-rolled aggregate keyed on
`(project_id, ad_id, hour)` for cheap reporting reads.

Detailed RLS rules and the four-rule migration linter contract live in
`REQUIREMENTS.md` § 7.1.1.

## 7. Multi-tenancy

Two-level tenancy: **Org → Project**. Every resource is project-
scoped (or org-scoped for cross-project resources like Ad Library
items and tokens). RLS is the enforcement floor; the query layer
enforces it again at every transaction; CI enforces it a third time
via the cross-tenant manifest gate (`xtask check-cross-tenant`,
`tests/cross_tenant_manifest.toml`).

Three deployment shapes match different consumer patterns:

1. **Single-project** — one knievel deployment serves one publisher.
   Multi-tenancy is unused but the rails are still there. Cheapest
   to operate; least flexible.
2. **Project-per-environment** — one knievel deployment per
   environment (dev/stage/prod), each with its own project. The
   tenant boundary is the environment boundary.
3. **Project-per-tenant** — one knievel deployment serves N
   publishers, each their own project. The default v0 shape; what
   `MIGRATION_RX.md` documents.

Picking between 2 and 3 is mostly about who owns the project — the
operator or the tenant. Project tokens can be revoked independently
in shape 3; in shape 2 the operator owns all tokens.

## 8. Auth at a glance

| Method | When | Token shape | Validates against |
|---|---|---|---|
| Opaque bearer (default) | Every API call | `kvl_<env>_<scope>_<short-id>_<secret>` | `knievel.api_tokens` row, argon2id verify on `secret_hash` |
| JWT (optional) | Every API call | `Authorization: Bearer <jwt>` | JWKS auto-discovered from `iss` + `claim_mapping` config |
| K8s SA token | JWT mode special case | Standard projected SA token | Cluster's OpenID provider (or an inline JWKS endpoint) |
| HMAC URL sig | `/e/i/*` and `/e/c/*` | Path-embedded | Current signing secret + 8 h previous-secret overlap window |
| Anonymous | `/healthz`, `/readyz`, `/version`, `/openapi.json` | n/a | n/a |

`AUTH.md` covers the full surface — claim-mapping config, role
matrix, K8s integration recipe, signing-secret rotation procedure.

## 9. Observability stack

Three concurrent streams, all carrying the same `request_id` /
`trace_id`:

- **Logs**: structured JSON via `tracing`. Per-request fields:
  `request_id`, `org_id`, `project_id`, `route`, `status`, `duration_ms`.
  Handler-internal events get added as nested fields.
- **Traces**: OpenTelemetry spans exported via OTLP/gRPC.
  `poem-otel`-style middleware mints the root span; downstream DB,
  HTTP, and S3 calls inherit. Default sampler is parent-based with a
  10% root-sample rate.
- **Errors**: Sentry via `sentry-tower`. Per-request hub keeps
  fingerprinting per-tenant. Any 5xx from a handler files a Sentry
  issue; 4xx and timeouts don't.

Per-tenant data lives in **logs and traces**, not Prometheus —
Prometheus default-low cardinality is mandatory to keep the
collector stable. Per-project metrics can be opt-in for an
investigation via the `metrics.per_tenant_enabled` config block.

`REQUIREMENTS.md` § 10 has the full contract; `DEPLOYMENT.md` § 11
covers the operator-side wiring.

## 10. Failure model

Two cross-cutting principles, in order:

1. **Reads degrade later than writes.** A snapshot that's seconds
   stale still serves decisions; a Postgres writer outage means
   POST/PATCH return 503 and the snapshot loader's poll keeps the
   already-loaded snapshot live.
2. **Failures surface, not silently drop.** Save: the events
   channel under saturation, which is intentionally lossy with a
   `events.dropped` counter — this is a deliberate tradeoff
   covered in `REQUIREMENTS.md` § 10.9.

Other failure modes ranked by visibility (most-visible first):

| Failure | Visibility | Behavior |
|---|---|---|
| DB writer unreachable | `/readyz` 503; LB drains | Snapshot keeps serving reads; writes 503. |
| JWKS endpoint unreachable | Per-request 401 with explicit cause | Tokens minted before outage continue working until cache TTL. |
| S3 unreachable | `/v1/.../image` 503 | Existing creative images still serve; new uploads fail loudly. |
| Snapshot stale (LISTEN dropped) | `snapshot.staleness_seconds` metric | Poll backstop catches up within 5 s. |
| Event channel saturated | `events.dropped` metric | Decisions still served; events lost (lossy by design). |
| Leader miss (advisory lock churn) | `partition.leader_changes` metric | Partitions catch up on next leader's tick (10 min). |
| Aurora failover | `db.failover` metric (synthetic) | LISTEN drops; reconnect handler re-establishes. |

`REQUIREMENTS.md` § 10.9 enumerates the chaos rigs that exercise each;
`DEPLOYMENT.md` § 13 has the runbook links.

## 11. Capacity model

SLO targets — currently **TARGET (unverified)** until the v0.1.0 bench
lands in `bench/results/v0.1.md`:

- **Decision latency**: p50 ≤ 8 ms, p99 ≤ 25 ms, p99.9 ≤ 75 ms (per
  pod, against a snapshot of ≤ 10k ads).
- **Decision throughput**: 5k req/s per pod at the SLO above.
- **Event ingest**: 50k events/s per pod (in-memory channel + every
  ~1.5 s `COPY` batch).
- **Snapshot staleness**: ≤ 5 s.

Scaling axes:

- **Horizontal (pods)**: each pod carries its own snapshot copy and
  its own event channel. Horizontal scale is linear up to the DB's
  connection budget. The chart's default budget is 12 connections per
  pod — see `REQUIREMENTS.md` § 7.8.
- **Vertical (Postgres)**: writer tier scales with event-write
  throughput; reader replicas don't help knievel's hot path (RAM
  snapshot) but help downstream warehouse copies.
- **Object store**: independent of knievel's tier; use whatever is
  closest to your serving region.

## 12. Named tradeoffs

One paragraph per substantive design decision:

- **Postgres-only in v0.** No Redis, no Cassandra, no Kafka. The
  snapshot lives in process memory and recovers from Postgres on
  boot; events are buffered in a bounded in-process channel and
  `COPY`'d in batches. This bounds operational complexity dramatically
  but forces a hard cap on event-rate scaling: when the channel
  saturates, events drop. The tradeoff is intentional — real-world
  publishers tolerate the loss far better than they tolerate a
  second-datastore operational burden.
- **In-memory snapshot vs. per-request DB lookup.** Decisions touch
  RAM only. The snapshot trades a 5-second-bounded staleness window
  for sub-millisecond hot-path reads. Anyone who needs strongly
  consistent reads should not use knievel for the decision path —
  there's no API for "force-read the writer DB."
- **Server-to-server only in v0.** No browser-direct decision endpoint,
  no CORS-relaxed paths, no public anonymous access except the
  HMAC-signed event endpoints. A v0 caller is always a server
  (Rails, Node, Go, …) holding a bearer token that knievel issued.
  Browser-direct (signed URL or session-cookie) is a real future need
  but adds an authn surface knievel doesn't ship today.
- **Opaque tokens AND JWTs both supported.** Operators with K8s
  service accounts get JWT mode for free; operators without get
  opaque bearers issued from the management API. Picking just one
  would force the other group to build a translation layer.
- **No required Postgres extensions beyond `pgcrypto`.** Aurora,
  RDS, Supabase, and self-hosted Postgres 14+ all run knievel
  unchanged. We could squeeze better partitioning ergonomics from
  `pg_partman` but the dependency would scope us to operators who
  can install it.
- **In-process partition manager.** A second daemon (or a `pg_partman`
  install) would be one more thing to monitor. The advisory-lock
  leader election + 4-day-look-ahead partitions cover the operational
  envelope without an extra service.
- **One workflow per release, not per artifact.** The `release.yml`
  workflow on `v*` tag does CI, image, CLI binaries, GitHub Release,
  cosign, and provenance attestation in one DAG. Splitting them
  across workflows would have been more "modular" but would have
  multiplied the trigger configuration surface.

## 13. Where to read more

| Topic | Source |
|---|---|
| Spec — full system requirements | `REQUIREMENTS.md` |
| Wire surface — request/response shapes | `API.md` |
| Auth — token shape, JWT mode, role matrix | `AUTH.md` |
| Reporting — event schema, rollup tables | `REPORTING.md` |
| Operator guide — install, sizing, runbooks | `DEPLOYMENT.md` |
| Test plan — slice naming, CI gate matrix | `TESTING.md` |
| Live progress — what's done, what's next | `PHASES.md` |
| Per-consumer migration recipe (RX) | `MIGRATION_RX.md` |
| Cross-session contributor primer | `CLAUDE.md` |

The platform contract is the four files at the top
(`REQUIREMENTS.md`, `API.md`, `AUTH.md`, `REPORTING.md`). Everything
else is supporting material.
