# Knievel Testing

How knievel is tested — unit through end-to-end, plus the security
gates that block release. Companion to `REQUIREMENTS.md`, `API.md`,
`AUTH.md`, and `REPORTING.md`.

This document is the working spec for the test suite. It is meant to
be precise enough that a contributor can answer "where should this
test live?" without guessing, and that a release engineer can
answer "what blocks the tag?" without reading code.

## 1. Goals

1. **Correctness over coverage.** Tests exist to encode invariants we
   refuse to ship without. Line coverage is a side effect, not a
   target.
2. **Fast feedback on the hot path.** Decision-selection and
   snapshot-loader tests are pure-Rust unit tests that run in
   milliseconds. The full suite is parallelizable; `cargo nextest run`
   on a developer laptop completes in under 60 s when the DB harness
   is warm.
3. **Tenant isolation is verified, not asserted.** §7.1.1 of
   `REQUIREMENTS.md` specifies three release-blocking gates for
   tenant isolation. They are implemented here, not aspirational.
4. **The OpenAPI spec is contract-tested.** Every public endpoint is
   exercised through the same surface generated clients use. Spec
   drift between the binary, `openapi.yaml`, and the generated Ruby
   gem is caught in CI.
5. **Acceptance tests describe user journeys**, not endpoints. The
   suite reads like the rollout shape in `MIGRATION_RX.md`: provision,
   sync, decide, observe.
6. **Degraded-mode behavior is testable.** The matrix in
   `REQUIREMENTS.md` §10.9 has a paired test for every row. "We
   handle a DB writer outage gracefully" is a green check, not a
   claim in a doc.

## 2. Non-Goals (v0)

- **100% line coverage.** Coverage above 80 % on the selection
  algorithm and auth layer; coverage of generated boilerplate is
  uninteresting.
- **Mutation testing.** Maybe later (e.g. `cargo-mutants`); not
  release-blocking in v0.
- **Fuzzing as a CI gate.** We run `cargo fuzz` on the HMAC and JWT
  validators nightly, but a clean nightly is not a tag prerequisite.
- **Browser testing.** No admin UI in v0.
- **Cross-cloud E2E.** The acceptance suite runs against
  containerized Postgres; Aurora-specific behaviors (failover, NOTIFY
  drop on leader change) are simulated, not exercised against a real
  cluster.

## 3. Test Pyramid

```
                       ┌───────────────────────────────┐
                       │ E2E Acceptance (§ 7)          │  ~30 scenarios,
                       │ docker compose, real HTTP     │   ~5 min CI
                       ├───────────────────────────────┤
                       │ API / Contract (§ 6)          │  ~200 tests,
                       │ poem TestClient + sqlx::test  │   ~60 s CI
                       ├───────────────────────────────┤
                       │ Integration (§ 5)             │  ~150 tests,
                       │ real Postgres, narrow scope   │   ~45 s CI
                       ├───────────────────────────────┤
                       │ Unit (§ 4)                    │  ~500 tests,
                       │ pure Rust, no I/O             │   < 10 s CI
                       └───────────────────────────────┘
```

A test belongs at the lowest layer that can prove the property. A
selection-algorithm property is a unit test; a tenant-isolation
property crosses the auth/handler/DB boundary and lives at the API
layer; "the calling app's gem can run a real sync" is acceptance.

## 4. Unit Tests

Pure-Rust, no I/O, no async runtime spin-up beyond `tokio::test`
where genuinely needed. Organized as `#[cfg(test)] mod tests`
co-located with the code under test (Rust convention).

### 4.1 What lives here

| Module | Properties tested |
|---|---|
| `selection::filter` | Site/zone/ad-type predicates; `force.*` overrides; `block.*` exclusions; date-window evaluation. |
| `selection::weighted` | Weighted-random selection with a seeded `StdRng` — same seed → same selection; weight 0 never selected; single-candidate tier always selects. |
| `selection::priority` | Highest non-empty tier wins; empty tier falls through. |
| `hmac::sign` / `hmac::verify` | Round-trip, TTL expiry, payload tampering rejected, base64url encoding. |
| `hmac::rotation` | Two-secret window: previous secret accepted within 8 h, rejected after. |
| `auth::opaque::parse` | Format detection (`kvl_` prefix), scope/env extraction, malformed strings rejected. |
| `auth::opaque::hash` | argon2id round-trip; constant-time comparison; mismatched hash rejected. |
| `auth::jwt::validate` | Algorithm allow-list (asymmetric only); `alg: none` rejected; `kid` lookup; expiry / nbf / iat with skew tolerance; audience and issuer checks; `knievel` claim presence and shape. |
| `auth::jwt::claim_mapping` | First-match-wins; multi-key matches; default reject when no rule matches. |
| `auth::lint` | Boot-time validation from `AUTH.md` "Startup Linting": every hard-fail rule is exercised with a malformed config. |
| `idempotency::key` | Hash stability across body whitespace; replay match keyed on `(project, key, route, body-hash)`. |
| `config::layering` | Defaults < file < env override; `${VAR}` and `${VAR:default}` interpolation; missing required `${VAR}` is a hard error. |
| `partitions::names` | `events_raw_p<YYYY_MM_DD>` parsing/round-trip; rejects malformed leaf names. |
| `events::dedup_key` | Stable per `(kind, signature_nonce)`; truncation length; survives signing-secret rotation. |

### 4.2 Property tests

`proptest` is used where the input space is wider than table-driven
cases comfortably cover:

- **Selection algorithm**: for any (flights, ads, blocklist) input,
  the selected ad is always in the highest non-empty priority tier
  and is not in the blocklist. Run with 10 000 cases per CI.
- **HMAC payload**: any byte mutation of a signed URL fails
  verification.
- **Idempotency hash**: any two requests with identical canonical
  JSON produce identical keys regardless of map ordering.

### 4.3 What does not live here

- Anything that touches a real DB connection, real HTTP, or real
  filesystem — those live at §5 or above.
- Tests of derived `serde` impls. Trust the framework.

## 5. Integration Tests

Narrow scope, but with real moving parts: a real Postgres for DB
tests, a real `tokio` runtime for async-state-machine tests. Each
test owns its preconditions; no cross-test fixture state.

### 5.1 Database harness

[`sqlx::test`](https://docs.rs/sqlx/latest/sqlx/attr.test.html)
attribute creates a fresh, named, throwaway DB per test from a
template, runs migrations, and tears down on completion. Test runs
in transactional isolation when the test only reads/writes data; in
a real database when the test exercises DDL (partitions, RLS).

```rust
#[sqlx::test(migrations = "./migrations")]
async fn snapshot_loads_after_notify(pool: PgPool) -> Result<()> {
    // ...
}
```

A small wrapper (`testlib::db::ephemeral`) handles the cases
`sqlx::test` doesn't cover — multi-connection setups for
`LISTEN/NOTIFY` and advisory-lock leader-election tests.

Containerized Postgres is provided by
[`testcontainers`](https://docs.rs/testcontainers/) when running
locally without a host Postgres. CI uses a service container for
speed.

### 5.2 What lives here

| Subsystem | Properties tested |
|---|---|
| **Migrations** | All migrations apply cleanly to an empty DB; idempotent re-runs; rollback safety where applicable; `_sqlx_migrations` lives in `knievel` schema. |
| **Snapshot loader** | Cold load from a populated DB matches expected `(project_id, resource)` map. `NOTIFY config_changed` triggers a diff-pull and atomic swap. Poll backstop (5 s) catches a missed NOTIFY. Aurora-failover simulation: drop the LISTEN connection mid-test and verify reconnect-with-backoff completes within the budget. |
| **Event flusher** | Channel → `COPY` round-trip lands rows in the correct daily partition. Channel saturation surfaces as an error to the caller, never silent loss. Graceful shutdown drains the channel before exit. |
| **Partition manager** | Premake creates 4 days of future partitions. Retention drops partitions older than `retention_days`. `DETACH PARTITION CONCURRENTLY` succeeds under concurrent inserts. Idempotent — running maintenance twice in a row is a no-op. |
| **Leader election** | `pg_try_advisory_lock` acquires once per cluster. Closing the leader's connection releases the lock; a follower acquires within 30 s. The watchdog exits the process on a missed maintenance window. |
| **HMAC verifier under rotation** | Mint URL with secret v1, rotate to v2, verify within 8 h: ok. Verify after 8 h + 1 s: `400 expired`. Mint with v2, verify with v1 in cache: rejected. |
| **Audit log** | Every `force.*` decision, secret rotation, project-deletion, member-role change, token mint/revoke writes a row with the documented schema. Append-only — `UPDATE` on `audit_log` is denied by RLS policy. |
| **Idempotency cache** | Replay returns the original 200/201; replay with a different body returns `409 idempotency_conflict`; cache-miss after TTL re-executes. |

### 5.3 RLS verification at the DB layer

A targeted suite exercises RLS policies at the SQL level, separate
from the cross-tenant API tests in §6.5:

- For each table in `knievel`, verify `relrowsecurity = true` and
  `relforcerowsecurity = true` (the `FORCE` matters — without it
  table owners bypass policies).
- For each policy, verify the `USING` clause references
  `current_setting('knievel.project_id')` (or the documented
  session-scoped tenant binding).
- A direct-SQL probe with two `SET LOCAL knievel.project_id = ...`
  values confirms isolation: rows inserted under project A are
  invisible to a session bound to project B.

These tests overlap with the migration linter (§ 9.2) but cover the
runtime behavior; the linter covers the SQL text.

## 6. API / Contract Tests

The largest layer by test count. Every endpoint declared in `API.md`
is exercised through `poem::test::TestClient`, which routes against
the same handler stack production uses — same auth extractors, same
idempotency middleware, same OpenAPI surface — but skips the network
hop.

### 6.1 Test client + auth fixtures

```rust,ignore
let app = knievel::test_app(pool).await;
let resp = app
    .post("/v1/projects/pj_demo/decisions")
    .bearer_auth(token::editor_for(org, project))
    .json(&decision_request())
    .send()
    .await;

resp.assert_status_is_ok();
resp.assert_json(&expected);
```

Token fixtures (`token::editor_for`, `token::reader_for`,
`token::wrong_project`, `token::wrong_org`, `token::expired_jwt`,
`token::sa_for_namespace`) cover every (mode × scope × role)
combination so any handler test can ask for the credential it needs
in one line.

### 6.2 Response-shape contracts

Every successful response is asserted with [`insta`](https://docs.rs/insta/)
snapshot tests. A handler-shape change requires an explicit
`cargo insta review` step; accidental field renames or type changes
fail CI loudly.

Error responses are snapshot-tested too — `error.code` is part of
the public contract per `API.md` § 4 "Error body."

### 6.3 OpenAPI spec drift

A single test compiles the binary's runtime OpenAPI document, diffs
it against `openapi.yaml` checked into the repo, and fails if they
disagree. The fix is `cargo xtask openapi`. This is the gate that
keeps the spec, server, and generated client in lockstep.

A second test asserts that `openapi.yaml` validates against the
OpenAPI 3.1 meta-schema — so a malformed spec never reaches the
generator.

### 6.4 Per-resource CRUD contracts

Each project-scoped resource (Advertiser, Campaign, Flight, Ad,
Creative, CreativeTemplate, Site, Zone, plus AdLibraryItem at org
level) has a uniform suite generated from a single `crud_contract!`
macro:

| Test | Property |
|---|---|
| `create_returns_201` | Server-assigned `id`, echoed `external_id`, `etag`, `created_at`/`updated_at`. |
| `create_idempotent_on_external_id` | Second create with same `external_id` is a no-op returning the first row. |
| `create_idempotency_key_replay` | Same `Idempotency-Key` returns cached body with `Idempotent-Replay: true`. |
| `create_idempotency_key_mismatch_body` | Same key + different body → `409 idempotency_conflict`. |
| `read_404_unknown_id` | Unknown `id` and unknown `external_id` both 404. |
| `update_etag_match` | `If-Match: <etag>` succeeds; stale etag → `409 if_match_mismatch`. |
| `list_paginates` | `limit` honored; `next_cursor` returns the next page; cursor stable across writes. |
| `filter_by_external_id` | `?external_id=...` filter narrows to one row. |
| `soft_delete` | `is_active: false` round-trips; `GET` still returns the row. |
| `batch_upsert_atomic` | One bad row in a batch rolls back all rows; `details[]` reports the offending index. |
| `cross_entity_fks_in_batch` | A flight referencing a campaign created earlier in the same batch resolves. |

Adding a new resource means adding one `crud_contract!` invocation;
the macro emits the full table.

### 6.5 Cross-tenant negative tests (release-blocking)

`REQUIREMENTS.md` §7.1.1 gate (1). Every project-scoped endpoint has
a paired test:

```rust
#[knievel_test]
async fn cross_tenant_403_advertiser_get(ctx: TestCtx) {
    let (_org, project_a) = ctx.org_with_project().await;
    let (_org_b, project_b) = ctx.org_with_project().await;

    let advertiser = ctx
        .as_editor_of(&project_a)
        .create_advertiser("acme")
        .await;

    let resp = ctx
        .as_editor_of(&project_b)            // wrong project, same org? no — diff org.
        .get(&format!("/v1/projects/{}/advertisers/{}",
                      project_a.id, advertiser.id))
        .await;

    resp.assert_status(403);
    resp.assert_error_code("wrong_tenant");
}
```

`#[knievel_test]` is a custom attribute that registers the endpoint
under test in a CI manifest. A separate `cargo xtask check-cross-tenant`
binary walks the OpenAPI spec, lists every `/v1/projects/{p}/...`
operation, and **fails the build** if any is missing a paired
cross-tenant test. New endpoints cannot land without one.

The same harness covers `wrong_project` (same org, wrong project for
a project-scoped token) and `role_insufficient` (a `reader` calling
a write endpoint).

### 6.6 Auth path tests

Per `AUTH.md`:

- **Opaque tokens**: valid token → 200; revoked token → 401; wrong
  scope → 403; ip-allowlist mismatch → 403.
- **JWT**: valid signature → 200; bad signature → 401; expired → 401;
  wrong audience → 401; wrong issuer → 401; missing `kid` → 401;
  `alg: none` → 401; `HS256` → 401; missing `knievel` claim → 401;
  malformed `knievel` claim → 401.
- **JWKS rotation**: a new `kid` triggers a JWKS refresh; cache hits
  before TTL; cache refresh on TTL.
- **Claim mapping**: first-rule-wins; no-match → 401.
- **Boot-time lint**: every malformed-config path from
  `AUTH.md` "Startup Linting" hard-fails the binary; happy path
  emits the expected `INFO` line and `/version` payload.

A mocked OIDC provider — `wiremock` standing in for Keycloak and the
Kubernetes API server — provides `/.well-known/openid-configuration`
and `/jwks.json` so JWT tests run hermetically without external
network.

### 6.7 Decision-API specifics

The decision endpoint gets its own focused suite on top of the
generic CRUD harness:

| Test | Property |
|---|---|
| `decisions_empty_when_no_eligible_ads` | `decisions[<id>] = []`, never null, never absent. |
| `decisions_select_by_priority_tier` | Higher tier always wins over lower. |
| `decisions_weighted_random_with_seed` | Deterministic given a seeded RNG; weight 0 never selects. |
| `decisions_block_creative_ids` | Blocked creative excluded post-priority-grouping. |
| `decisions_force_admin_only` | Editor → 403; admin without flag → 403; admin with flag → 200 + audit row. |
| `decisions_force_global_kill_switch` | `decisions.force_overrides_enabled: false` → 403 cluster-wide. |
| `decisions_force_audit_row` | Forced decision writes one and only one `audit_log` row with actor, payload hash, reason. |
| `decisions_site_resolution` | `site_id`, `site_url`, and `site_external_id` all resolve to the same `site_id` in the response. |
| `decisions_url_alias_match` | Site `aliases` resolve identically to canonical `url`. |
| `decisions_signed_urls_round_trip` | Minted impression/click URLs verify with the per-project secret and TTL. |
| `decisions_snapshot_version_stamp` | Response `snapshot_version` matches the snapshot at request time and ends up on the corresponding `events_raw` row. |
| `decisions_explain_no_event_recorded` | `:explain` mints dummy URLs and writes no events. |
| `decisions_explain_evaluation_shape` | Every candidate has a deterministic `evaluation` array. |

### 6.8 Event-tracking specifics

| Test | Property |
|---|---|
| `impression_204_default` | `GET /e/i/<sig>` → 204 on a fresh sig. |
| `impression_gif_when_requested` | `?fmt=gif` → 200 with the 43-byte transparent GIF. |
| `impression_tampered_204_silent` | Tampered sig → 204, internal `tampered` counter increments. |
| `click_302_redirect` | `GET /e/c/<sig>` → 302 to the creative's `click_through_url`. |
| `click_open_redirect_blocked` | `?u=<url>` is honored only when signed in. |
| `dedup_first_hit_countable` | First hit lands `is_duplicate = false`. |
| `dedup_second_hit_marked` | Second hit with same sig lands `is_duplicate = true`; click still 302s. |
| `dedup_spans_secret_rotation` | `dedup_key` is stable across the 8-h rotation overlap. |

### 6.9 Reporting-shape contracts

Per `REPORTING.md`:

- `events_rollup_watermark` advances monotonically.
- A `WHERE NOT is_duplicate` count of `events_raw` for a window
  matches the same window's `events_rollup` total once the watermark
  has caught up.
- `events_rollup` never includes `is_duplicate = true` rows.
- The `knievel_reader` role can `SELECT` from `knievel.*` and cannot
  `INSERT` / `UPDATE` / `DELETE` on any of them.

## 7. E2E Acceptance Suite

Black-box, runs against a real running stack via `docker compose`.
The harness brings up:

- `postgres:16`
- `knievel` (built locally; `auto_migrate: true`)
- `wiremock` for JWKS (Keycloak + K8s API server stand-ins)
- `minio` for S3-compatible image upload
- `otel-collector` (sink-only; sanity-check spans are emitted)

The compose file is the same one shipped in `examples/compose/` for
operators to use as a reference deployment, with one extra service
(the test runner). Acceptance is the deployment artifact, exercised.

### 7.1 Scenarios

Each scenario is a top-to-bottom user journey. Tests assert on
external behavior only — HTTP responses, DB rows visible to
`knievel_reader`, files in object storage, log lines, OTel spans.
No reaching into knievel internals.

| ID | Scenario | Source of truth |
|---|---|---|
| ACC-01 | Provision an Org and a Project, mint an Org Editor token, list the empty project. | `API.md` § 2.1, 2.2 |
| ACC-02 | Full demand chain: Advertiser → Campaign → Flight → Ad → Creative; issue a decision; assert the response shape and HMAC URLs. | `API.md` § 1, 3.1–3.5 |
| ACC-03 | Site lookup via `site_url` and `site_external_id` returns the same `site_id`. | `API.md` § 1, 3.7 |
| ACC-04 | URL aliases: a site with two aliases resolves all three URLs identically. | `API.md` § 3.7 |
| ACC-05 | Bulk sync: a single `:batchUpsert` call lands a coherent advertiser/campaign/flight/ad/creative graph. | `API.md` § 4 "Write contract" |
| ACC-06 | Bulk sync failure: one bad row rolls back the whole batch with per-row diagnostics. | `API.md` § 4 |
| ACC-07 | Idempotency: replay returns cached, body-mismatch is `409`. | `API.md` § "Idempotency" |
| ACC-08 | Soft delete via `is_active: false` round-trips. | `API.md` § "Common entity fields" |
| ACC-09 | Hot path: 1 000 sequential decisions; p99 < 50 ms (informational; SLO bench is § 8). | `REQUIREMENTS.md` § 9 |
| ACC-10 | Snapshot refresh: management write → `NOTIFY` → next decision sees new state within 5 s. | `REQUIREMENTS.md` § 7.2 |
| ACC-11 | Snapshot poll backstop: management write with NOTIFY suppressed → next decision sees new state within 6 s (poll interval + slack). | `REQUIREMENTS.md` § 7.2 |
| ACC-12 | Ad Library reference: org item → project ad reference → decision returns library content; mutate item → decision returns updated content within 5 s. | `API.md` § 2.4, `REQUIREMENTS.md` § 5.1 |
| ACC-13 | HMAC rotation: rotate, verify both old and new URLs work for 8 h overlap, old fails after. | `REQUIREMENTS.md` § 6.3 |
| ACC-14 | Impression + click round-trip: minted URL → event row in `events_raw` with the right `snapshot_version` and `dedup_key`. | `API.md` § 4 |
| ACC-15 | Replay dedup: hit the same impression URL twice → two rows, second `is_duplicate = true`. | `API.md` § "Replay, dedup, and counts" |
| ACC-16 | Click replay still redirects: hit click URL twice → both 302, second `is_duplicate = true`. | `API.md` § "Replay, dedup, and counts" |
| ACC-17 | Cross-tenant: project A token → project B → 403, `error.code = wrong_tenant`. | `REQUIREMENTS.md` § 7.1.1 (1) |
| ACC-18 | JWT path (Keycloak stand-in): valid token → 200; expired → 401; wrong audience → 401. | `AUTH.md` § "JWTs" |
| ACC-19 | K8s SA path: valid SA JWT for namespace `rx-prod` → mapped principal → 200; SA from unmapped namespace → 401. | `AUTH.md` § "Kubernetes ServiceAccount Tokens" |
| ACC-20 | Mode coexistence: opaque + JWT both enabled, both succeed in the same test run. | `AUTH.md` § "Mixing Modes During Cutover" |
| ACC-21 | Force overrides: editor → 403; admin with flag off → 403; admin with flag on → 200 + audit row. Global kill-switch overrides project setting. | `API.md` § 1, `AUTH.md` § "Endpoint → minimum role" |
| ACC-22 | Image upload: `POST /creatives/{id}/image` lands an object in MinIO; subsequent `GET /creatives/{id}` returns the URL; SVG upload → 415. | `REQUIREMENTS.md` § 7.9 |
| ACC-23 | Partition lifecycle: today's partition exists; partitions for `today + 4d` are pre-made; retention drop happens at the maintenance tick. | `REQUIREMENTS.md` § 7.4 |
| ACC-24 | Leader election: kill the current leader's container → another pod takes over within 30 s; partition maintenance still runs. | `REQUIREMENTS.md` § 7.5 |
| ACC-25 | Degraded — DB writer unreachable: pause the writer connection; decisions still serve from the snapshot; writes return `503 db_writer_unreachable`; recovery clears the failure. | `REQUIREMENTS.md` § 10.9 |
| ACC-26 | Degraded — snapshot stale: stop refreshing the snapshot; reads carry `X-Knievel-Stale-Snapshot`; > 300 s → `/readyz` returns 503. | `REQUIREMENTS.md` § 10.9 |
| ACC-27 | Degraded — event channel saturated: throttle the flusher; channel fills; decisions return `503 event_channel_saturated`; flusher recovery clears the failure. | `REQUIREMENTS.md` § 10.9 |
| ACC-28 | Reporting: a decision + impression + click chain produces the expected rows in `events_raw`; after a forced rollup tick, `events_rollup` aggregates the non-duplicate count. | `REPORTING.md` § "Schema for Reporters" |
| ACC-29 | `knievel_reader` role: can `SELECT` from `knievel.*`, cannot `INSERT` / `UPDATE` / `DELETE`; new tables created by future migrations are reachable via default privileges. | `REPORTING.md` § "Access Pattern" |
| ACC-30 | OpenAPI spec served at `/openapi.json` matches the committed `openapi.yaml`; `/version` carries the documented `auth` block (no secrets). | `API.md` § 5, `AUTH.md` § "Effective-policy visibility" |

Acceptance scenarios run sequentially in a single compose-up to keep
the per-PR cost bounded. Adding a 31st scenario should cost roughly
2–4 s of CI time.

### 7.2 Generated-client smoke pass

The Ruby gem (`knievel-ruby`, `REQUIREMENTS.md` § 8 item 3) ships
its own RSpec suite hitting the same compose stack. The platform
CI runs the smoke subset (provision, sync, decide, paginate) so
gem-server skew is caught before tag, not at integration time.

### 7.3 What the suite is not

- **Not a load test.** § 8 is.
- **Not a test of operator-supplied infrastructure** (e.g. real
  Aurora). The compose Postgres stands in. Aurora-specific properties
  (failover semantics, NOTIFY drop on leader change) are simulated in
  § 5.
- **Not a chaos suite.** § 9 is.

## 8. Performance and Capacity

`REQUIREMENTS.md` § 9.2 is the source of truth. Summary of how the
test suite plugs in:

- **Macro bench harness** (`bench/macro/`): `vegeta` driving a
  knievel binary built in `--release` against a synthetic project
  with 100 k active flights (`bench/macro/seed.sh`). Operator-run
  on dedicated hardware; not part of CI.
- **Micro + heap bench harness** (`benches/`): criterion (wall-
  clock) + iai-callgrind (deterministic CPU instructions / cache
  misses) + dhat-rs (heap allocations) covering the
  `selection::*` inner loop, `hmac::verify`, and the full
  Postgres-free `decisions::decide_pure` path.
- **Runner**: **Claude Code cloud sessions, not CI.** A session
  invokes `cargo xtask bench-all`, which reads the workspace
  version from `Cargo.toml`, runs the entire suite, captures a
  host fingerprint via `cargo xtask bench-env`, and writes
  `bench/results/v<MAJ>.<MIN>.{md,json}` matching the schema in
  `bench/results/SCHEMA.md`. Procedure documented in
  `bench/README.md`.
- **Trigger**: the release-tagging PR for any release that
  bumps the workspace minor version MUST include the new
  `bench/results/v<X>.json` entry; macro numbers in the
  `macro` slot can be back-filled by an operator before the
  tag fires.
- **Reportable artifact**: pinned by `bench/results/SCHEMA.md`.
  Top-level keys: `env` (CPU, memory, kernel, governor, rustc),
  `micro_criterion` (wall-clock per fixture), `micro_iai`
  (instruction counts per fixture), `heap_dhat` (allocations
  per decision), `macro` (vegeta p50/p95/p99/QPS plus
  concurrent CPU/RSS sampling).

### Regression policy

Per signal:

| Signal | Threshold | Blocking? |
|---|---|---|
| `iai-callgrind` `events.Ir` | > 5% | Issue (deterministic; any drift is real) |
| `criterion` micro `mean_ns` | > 30% | Issue |
| `criterion` macro `p50_ms` / `p99_ms` | > 20% | Release tag |
| `criterion` macro `throughput_qps` | > 20% | Release tag |
| `dhat` `total_bytes` | > 30% | Issue |

`cargo xtask bench-all --against v<prev>` reads the previous
release's JSON and prints a markdown delta table; paste it into
the release-tagging PR. The check is agent-driven, not gated by
a workflow.

The deterministic instruction counter is what makes the
historical record portable across runners — wall-clock is
±20% on cloud runners, but `events.Ir` is bit-identical across
identical source on identical rustc. That's what lets
`bench/results/v<X>.json` deltas survive a runner change.

## 9. Chaos / Degraded-Mode Suite

A separate suite (`tests/chaos/`) that runs nightly and on demand,
not on every PR. Built on the same compose harness as § 7.

| Failure | Injected via | Asserted behavior |
|---|---|---|
| DB writer unreachable | `iptables` drop rule on the Postgres container's port 5432 | Decisions continue from snapshot; writes return `503 db_writer_unreachable`; metrics + Sentry breadcrumb emitted |
| LISTEN connection drops | Force-close the loader's connection in Postgres | Snapshot loader reconnects with backoff; poll backstop catches any divergence within 5 s |
| NOTIFY queue overflow | Spam `pg_notify` from a side connection | Loader handles the dropped notifies; poll backstop reconciles |
| Aurora failover (simulated) | Restart the Postgres container | Loader reconnects to "writer"; advisory lock released and re-acquired by another pod within 30 s |
| Event channel saturation | Throttle flusher with `tc qdisc` to 0 bandwidth | Decision endpoint returns `503 event_channel_saturated`; channel never silently drops |
| JWKS unreachable | Block egress to wiremock | Cached keys serve until TTL; `kid` cache miss → 401 for that issuer; other issuers unaffected |
| Connection pool exhaustion | Hold all pool connections in test code | Endpoints return `503 db_pool_exhausted`; `/healthz` and `/metrics` still 200 |
| Leader watchdog miss | Pause the leader's process for `watchdog_hours + 1` | Process exits non-zero; another pod takes leadership; `/readyz` reports the watchdog state during the gap |
| Image upload mid-flight failure | Kill MinIO during a multi-part upload | Client gets 5xx; no partial creative row is committed |

Every row of `REQUIREMENTS.md` § 10.9 is paired with a chaos test
here. New degraded-mode rows require a paired test before merging.

## 10. Security Tests

### 10.1 Migration linter (release-blocking)

`REQUIREMENTS.md` § 7.1.1 gate (2). Implemented as `cargo xtask
lint-migrations`. Test fixtures in `xtask/tests/fixtures/migrations/`
cover every failure mode:

| Fixture | Expected outcome |
|---|---|
| `01_disable_rls.sql` | reject — `DISABLE ROW LEVEL SECURITY` |
| `02_no_force_rls.sql` | reject — `NO FORCE ROW LEVEL SECURITY` |
| `03_table_without_rls.sql` | reject — `CREATE TABLE` in `knievel` without paired `ENABLE ROW LEVEL SECURITY` |
| `04_policy_without_tenant.sql` | reject — `CREATE POLICY` whose `USING` clause doesn't reference `current_setting('knievel.project_id')` |
| `05_table_outside_knievel.sql` | accept — `public.something` is not knievel's concern |
| `06_clean_table.sql` | accept — RLS, FORCE, and tenant-bound policy all present |

The linter is unit-tested separately and called from CI on every
push.

### 10.2 Cross-tenant API tests

§ 6.5 and § 7 ACC-17. Both layers required: the API tests cover
endpoint coverage, the acceptance test covers the realistic deployment.

### 10.3 Release security checklist (release-blocking)

`REQUIREMENTS.md` § 7.1.1 gate (3). The checklist lives in
`RELEASE_CHECKLIST.md` (created at the same time as this doc). The
release-tagging PR template auto-renders the checklist; CI fails the
PR if any unchecked item lacks a written justification in the PR
body.

The checklist items map to:

- §6.5 + §7 ACC-17 green → "All cross-tenant integration tests pass."
- §10.1 green → "Migration linter passes."
- A diff of `auth/*` and migrations since the last tag, with a
  maintainer's signoff comment in the PR.
- A grep over the diff for handler signatures that take `org_id` /
  `project_id` from `Json<…>` bodies (rejected automatically by a CI
  step: tenant identity must come from path or token).
- A grep over the diff for new logging calls, gated against an
  allowlist of fields. Raw user agents, IP addresses outside
  `events_raw`, and JWT contents fail the gate.

### 10.4 Auth boot lint

§ 6.6 covers the unit-level cases. A small acceptance scenario
(part of §7 ACC-30) confirms the binary refuses to start with a
malformed config and exits with the documented structured error.

### 10.5 Fuzzing (nightly, not release-blocking)

- `cargo fuzz` against `hmac::verify` — must not panic on any byte
  string up to 4 KiB.
- `cargo fuzz` against `auth::jwt::validate` — must not panic on any
  byte string up to 16 KiB.
- `cargo fuzz` against the OpenAPI request-body decoders for the
  decision endpoint and `:batchUpsert` — must not panic; must always
  produce a 4xx for invalid input, never a 5xx.

Failed fuzz finds are filed as issues; clean fuzzing is not a tag
prerequisite, but a known panic on a fuzz-discovered input is.

## 11. Test Data and Fixtures

### 11.1 `seed-demo` is the canonical fixture

`knievel-cli seed-demo` (`REQUIREMENTS.md` § 8 item 4) populates a
demo Org / Project / Site / Zone / Advertiser / Campaign / Flight /
Ad / Creative chain. Acceptance tests start from this state — no
duplicate fixture code in tests.

A flag (`--reproducible`) freezes IDs and timestamps so insta
snapshots are stable across runs.

### 11.2 Per-test factories

Inside the test suite, fixtures are constructed by small typed
factories (`testlib::factory::*`) rather than sql files. Each
factory accepts overrides:

```rust,ignore
let advertiser = factory::advertiser(&pool, &project)
    .with_external_id("acme")
    .with_active(false)
    .insert()
    .await;
```

Factories never create cross-test state; everything is scoped to
the test's own schema.

### 11.3 Sample wire payloads

`tests/fixtures/wire/` mirrors the example bodies in `API.md` —
checked into the repo so doc and code review the same shape. A
codegen test asserts the spec's example bodies parse against their
declared schemas.

## 12. CI Pipeline and Gates

The CI provider is **GitHub Actions**. Three workflow files:

- `.github/workflows/ci.yml` — per-PR, required.
- `.github/workflows/nightly.yml` — scheduled, advisory.
- `.github/workflows/release.yml` — on `v*` tag, required.

Runners are GitHub-hosted `ubuntu-latest` by default. Self-hosted
runners are an operator choice; jobs are written so a runner swap
is a one-line `runs-on:` change. A composite action at
`.github/actions/rust-setup/` handles the `checkout` + `toolchain`
+ `rust-cache` boilerplate so every job stays a few lines.

### 12.1 Concurrency

```yaml
concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

Force-pushes and rapid re-pushes cancel in-flight runs. The release
workflow opts out (`cancel-in-progress: false`) — a tag build, once
started, runs to completion.

### 12.2 Cargo caching

A single shared cache slot per workflow, populated once by a primer
job and read by every downstream job. The action of record is
[`Swatinem/rust-cache@v2`](https://github.com/Swatinem/rust-cache):

```yaml
- uses: Swatinem/rust-cache@v2
  with:
    shared-key: knievel-ci
    cache-on-failure: true
```

Caches `~/.cargo/registry`, `~/.cargo/git`, and `target/` keyed on
`Cargo.lock` + `rustc -V` + the workflow file content. Across jobs
in the same workflow run, downstream jobs restore from the GHA
cache backend that `prime` populated rather than rebuilding.

The **`prime` job** runs first and pays the compile cost once:

```yaml
prime:
  runs-on: ubuntu-latest
  steps:
    - uses: ./.github/actions/rust-setup
    - run: cargo fmt --all --check
    - run: cargo clippy --all-targets --locked -- -D warnings
    - run: cargo nextest run --no-run --all-targets --locked
```

`fmt` and `clippy` ride along — they need the same dep graph and
pay no extra wall-clock once `target/` is warm. Subsequent test
jobs `needs: prime`, restore the same cache, and run only their
slice.

`sccache` is intentionally not used: with `rust-cache` plus a primer
job, the marginal speedup doesn't justify the extra moving piece.

The single shared cache slot means a `Cargo.toml` change invalidates
everything at once — but that's correct, and `prime` absorbs the
hit so test jobs stay fast. A per-job `target/` slot would
parallelize the cold path but cost ~2 GB of cache per job, blowing
past GitHub's 10 GB per-repo cache cap fast.

### 12.3 Docker layer caching

The acceptance suite needs the knievel container image. A
`build-image` job builds once per workflow run with `docker/buildx`
backed by the GitHub Actions layer-cache backend:

```yaml
- uses: docker/build-push-action@v5
  with:
    context: .
    tags: knievel:ci
    cache-from: type=gha,scope=knievel
    cache-to:   type=gha,scope=knievel,mode=max
    outputs:    type=docker,dest=/tmp/knievel-image.tar
- uses: actions/upload-artifact@v4
  with: { name: knievel-image, path: /tmp/knievel-image.tar }
```

Acceptance shards download the artifact and `docker load` it —
no registry round-trip, no pull-rate-limit risk on `ubuntu-latest`,
and the same image is exercised by every shard.

### 12.4 Per-PR DAG

```
                              ┌─────────────────┐
                              │ prime           │  cargo build, fmt, clippy
                              │ (warm target/)  │  cache populated
                              └────────┬────────┘
                                       │
        ┌──────────┬──────────┬────────┼────────┬──────────┬─────────────┐
        ▼          ▼          ▼        ▼        ▼          ▼             ▼
   unit-prop   db-integ   api-contract   xtask-lints   openapi-drift   helm-lint
   (no DB)    (pg svc)    (pg svc)       (mig+xtenant) (spec match)    (kubeconform)
        │          │          │            │            │                │
        └──────────┴──────────┴────────────┴────────────┴────────────────┘
                                       │
                              ┌────────▼────────┐
                              │ build-image     │  buildx + GHA layer cache
                              └────────┬────────┘
                                       │
                              ┌────────▼────────┐
                              │ acceptance      │  matrix: shard 1..4
                              │ (compose)       │  ACC-01..30 partitioned
                              └────────┬────────┘
                                       │
                              ┌────────▼────────┐
                              │ gem-smoke       │  ruby + RSpec subset
                              └─────────────────┘
```

The fan-out after `prime` is the wall-clock floor. With a warm
cache, every middle-row job finishes in under 90 s. Cold cache
(e.g. a `Cargo.lock` change) costs `prime` ~5 min and downstream
jobs add another ~30 s on top.

### 12.5 Test slicing

Test files follow a naming convention so a single `nextest` filter
expression maps cleanly to a CI shard:

| Slice | nextest filter | Postgres service? |
|---|---|---|
| `unit-prop`     | `-E 'kind(lib) + kind(bin) + binary(unit)'`        | no |
| `db-integ`      | `-E 'kind(test) & binary(integration)'`            | yes |
| `api-contract`  | `-E 'kind(test) & binary(api)'`                    | yes |
| `acceptance`    | `-E 'kind(test) & binary(acceptance)'`             | compose stack |

A `cargo xtask test-shape` check fails CI if a test lands outside
the expected naming. Slices stay stable as the suite grows — no
editing the workflow when a new test file lands.

The `db-integ` and `api-contract` jobs declare a Postgres service
container:

```yaml
services:
  postgres:
    image: postgres:16
    env:
      POSTGRES_USER:     knievel_app
      POSTGRES_PASSWORD: dev
      POSTGRES_DB:       knievel
    options: >-
      --health-cmd pg_isready --health-interval 2s
      --health-timeout 2s --health-retries 30
```

### 12.6 Acceptance sharding

ACC-01..30 (§ 7) runs across an N=4 matrix, partitioned by nextest:

```yaml
acceptance:
  needs: build-image
  strategy:
    fail-fast: false
    matrix:
      shard: [1, 2, 3, 4]
  runs-on: ubuntu-latest
  steps:
    - uses: ./.github/actions/rust-setup
    - uses: actions/download-artifact@v4
      with: { name: knievel-image, path: /tmp }
    - run:  docker load -i /tmp/knievel-image.tar
    - run: |
        docker compose -f tests/acceptance/compose.yaml \
          -p knievel-acc-${{ matrix.shard }} \
          up -d --wait
    - run: |
        cargo nextest run \
          --partition count:${{ matrix.shard }}/4 \
          -E 'kind(test) & binary(acceptance)'
```

Each shard runs under its own compose project name so docker
network and port collisions are impossible. Total acceptance wall
time drops from ~5 min single-threaded to ~90 s sharded.
`fail-fast: false` keeps the diagnostic value of "shards 1 and 3
failed" rather than cancelling on the first red.

4 is the sweet spot on `ubuntu-latest`. Going higher spends more
time on compose-up than on tests.

### 12.7 Per-PR gates (required)

| Stage | Gate | Job |
|---|---|---|
| `cargo fmt --check` | Required | `prime` |
| `cargo clippy -- -D warnings` | Required | `prime` |
| `cargo nextest run` (unit + integration + API) | Required | `unit-prop`, `db-integ`, `api-contract` |
| `cargo xtask lint-migrations` | Required | `xtask-lints` |
| `cargo xtask check-cross-tenant` | Required | `xtask-lints` |
| `cargo xtask test-shape` | Required | `xtask-lints` |
| `cargo xtask openapi --check` | Required (binary ↔ `openapi.yaml`) | `openapi-drift` |
| OpenAPI 3.1 meta-schema validation | Required | `openapi-drift` |
| Acceptance suite (4 shards) | Required, all shards green | `acceptance` |
| Generated-gem smoke pass | Required | `gem-smoke` |
| Helm chart `helm lint` + `kubeconform` | Required | `helm-lint` |
| Release-checklist enforcer (release-tagging PRs only) | Required | `release.yml` |

GitHub branch protection requires every required job. `prime` is
not directly gated — its failures surface via the `fmt` / `clippy`
/ build-failure rows downstream; making it required would
double-count.

### 12.8 Nightly (advisory)

`.github/workflows/nightly.yml`, scheduled `cron: '13 7 * * *'`
(low-collision time, post-US-PT). Reuses the same `prime` +
`shared-key` cache strategy.

| Stage | Behavior on failure |
|---|---|
| Chaos suite (`tests/chaos/`, § 9) | Open issue via `peter-evans/create-issue-from-file`; page `#knievel-oncall` |
| `cargo fuzz` (60 min budget per target: hmac, jwt, decisions) | Open issue with the offending input |
| `criterion` benchmark vs. last main | Open issue if any metric regresses > 30 % |
| Multi-Postgres-version matrix (14, 15, 16) | Open issue |

A failed nightly does not block tags. § 8 and § 10.5 spell out
which of these are tag prerequisites when the underlying code path
changes.

### 12.9 Release-tagging workflow

`.github/workflows/release.yml`, triggered on `push` to tags
matching `v*`. Runs the per-PR DAG (so a tag never goes out without
green tests), then:

1. **Bench regression check.** `bench/results/<version>.md` must be
   present for any release that touches `selection::*` /
   `snapshot::*` / `events::flusher::*`. Fails the workflow on a
   missing artifact, or on a > 20 % regression vs. the previous
   release on (p50, p99, sustained QPS), unless an explicit waiver
   appears in the tag's release notes.
2. **Release security checklist enforcer** (§ 10.3). Fails on any
   unchecked item without a written justification in the release
   notes.
3. **Container image build.** Multi-arch `docker buildx` for
   `amd64` + `arm64`, signed with `cosign`, pushed to
   `ghcr.io/knievel-ads/knievel:<version>` and `:latest`.
4. **Helm chart packaged** and published to the chart's index.
5. **Ruby gem rebuilt** from the released spec, version bumped to
   match the spec version, pushed to RubyGems.
6. **GitHub Release** created with the changelog and artifact links.

Release jobs do not run with `cancel-in-progress`. A retried tag
creates a new run; partial publishes are documented in
`RELEASE_PLAYBOOK.md` (separate runbook, not part of this spec).

Manual acceptance — a maintainer running the compose stack against
a fresh dev DB and walking ACC-01..30 visually — is captured as a
short note in the release-tagging PR. Green CI is what blocks
merge; the manual pass is belt-and-suspenders.

### 12.10 Workflow file layout

```
.github/
├── workflows/
│   ├── ci.yml         # per-PR (12.4 – 12.7)
│   ├── nightly.yml    # 12.8
│   └── release.yml    # 12.9
└── actions/
    └── rust-setup/    # composite: checkout + toolchain + rust-cache
        └── action.yml
```

Keeping the composite action under `.github/actions/` (vs.
publishing it) means CI doesn't depend on a `marketplace` dance for
a repo-internal helper. Updates ride along with workflow PRs.

## 13. Coverage Policy

- **No global percentage target.** A line-coverage minimum dropped
  on the build is a coverage-as-target antipattern.
- **Module floors** for the modules where coverage is meaningful:
  - `selection::*`: 90 %, branch coverage included.
  - `auth::*`: 90 %, branch coverage included.
  - `hmac::*`: 95 %.
  - `migrations::lint`: every documented rule has a fixture (binary
    coverage rather than line).
  - `partitions::*`: 80 %, with the leader-election state machine at
    100 % branch.
- **Generated code, derived impls, error-conversion boilerplate:** no
  target.

A `cargo llvm-cov` report is uploaded as a CI artifact for inspection
but does not block merge. Floors are enforced as discrete tests
("the algorithm-level test for X exists") rather than a percentage
gate, so a regression manifests as a missing test, not a percentage
drift.

## 14. Local Developer Workflow

```
cargo nextest run                 # unit + integration + API, ~60 s
cargo xtask lint-migrations       # migration linter
cargo xtask check-cross-tenant    # endpoint-coverage gate
cargo xtask openapi --check       # spec drift gate
just acceptance                   # compose-up + acceptance, ~5 min
```

`just` recipes wrap the compose harness so contributors don't memorize
flags. `just acceptance-one ACC-12` runs a single scenario, useful
when iterating on Ad Library reference resolution or partition
maintenance without paying for the full pass.

`just watch` runs `cargo nextest run` on every save with the DB
harness kept warm; first run pays the migration cost, subsequent
runs reuse the template DB.

## 15. What Tests Don't Catch

Honest list, kept maintained, called out in code review:

- **Real Aurora failover semantics.** Simulated, not exercised.
- **Real Keycloak token-mapper edge cases.** Wiremock stand-in.
  Misconfigured mappers in real Keycloak → manifest as `401
  invalid_token / claim_missing` and are caught at integration time
  by the operator, not by us.
- **Browser ad-blocker interactions** with `/e/...` URLs. v0 doesn't
  ship browser-direct mode; this comes back when it does.
- **CDN cache behavior** for impression GIFs. Operator-owned.
- **dbt model correctness against `events_raw`.** We assert the
  schema and the watermark contract; we don't run the consumer's
  dbt. The `examples/dbt/` skeleton is `dbt parse`'d in CI to catch
  syntactic drift, no more.
- **Real S3 bucket policies.** MinIO stand-in; bucket-policy
  configuration is operator-owned.
- **Long-running leak detection.** `tokio-console` is wired in dev
  but not driven by CI.

These live in operator-side smoke tests, not knievel's CI. The doc
that makes them visible is this one — adding to the list is a
deliberate acknowledgement, not a quiet oversight.

## References

- [`sqlx::test`](https://docs.rs/sqlx/latest/sqlx/attr.test.html) — ephemeral test databases
- [`testcontainers-rs`](https://docs.rs/testcontainers/) — managed Postgres containers
- [`poem::test`](https://docs.rs/poem/latest/poem/test/index.html) — handler-level test client
- [`insta`](https://docs.rs/insta/) — snapshot tests
- [`wiremock`](https://docs.rs/wiremock/) — HTTP stand-ins for OIDC/JWKS
- [`proptest`](https://docs.rs/proptest/) — property-based tests
- [`criterion`](https://docs.rs/criterion/) — micro-benchmarks
- [`cargo-nextest`](https://nexte.st) — parallel test runner
- [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) — coverage reports
- [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html) — fuzzing
- [`vegeta`](https://github.com/tsenart/vegeta) / [`k6`](https://k6.io) — load generation
- [OpenAPI 3.1 meta-schema](https://spec.openapis.org/oas/3.1/schema/2022-10-07.html)
