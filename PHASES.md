# Knievel Implementation Phases

The execution roadmap for taking knievel from a planning corpus to a
runnable, tested service. Companion to every other doc in the repo;
this file is the living progress log.

This is a **working document**, updated as work lands. Each task is
marked with status; completed tasks reference the commit message
that delivered them (find the SHA via `git log --grep "Phase X.Y:"`).

## Status legend

- `[ ]` — not started
- `[~]` — in progress
- `[x]` — done; commit subject prefixed `Phase X.Y:`

## How to use this file

1. Tasks are ordered. Don't skip ahead unless the dependency graph
   permits it explicitly (callouts on each phase note where
   parallelism is possible).
2. Every code commit subject is prefixed `Phase X.Y: <task name>`
   so `git log --oneline --grep "Phase"` is the audit trail.
3. PHASES.md updates ride along with each task's commit (same
   commit marks the task `[x]`). This file is therefore always
   one-commit-current.
4. Notes on surprises, deviations, or follow-up work get appended
   to the **Notes** subsection of the relevant phase, dated.

## Phase references

Each phase identifies which design docs (and which sections) it is
implementing. If a phase's behavior diverges from the spec, the spec
is updated in the same PR — the spec docs and the code agree.

---

## Phase 1 — Foundation

**Goal:** CI rails and DB scaffolding are in place before any
business code lands. The migration linter and the cross-tenant
endpoint-coverage gate exist before there's anything to lint, so
they protect the very first migrations and handlers.

**Spec references:**

- `REQUIREMENTS.md` § 7 (Storage), § 7.1 (Schema and isolation),
  § 7.1.1 (Tenant isolation verification — gates 1, 2, 3).
- `TESTING.md` § 12 (CI Pipeline), § 10.1 (Migration linter),
  § 6.5 (Cross-tenant negative tests).

**Parallelism:** Tasks 1.4 and 1.5 (composite action + workflows)
can land before 1.6 (first migration). Tasks 1.7 and 1.8 (linters)
need 1.3 (xtask) and 1.6 (migration to lint).

### Tasks

- [x] **1.1** PHASES.md (this file).
- [x] **1.2** Cargo workspace + pinned deps + `rust-toolchain.toml`
      + `.gitignore`. Workspace root doubles as the `knievel`
      package; `xtask` and `testlib` join as members in 1.3 and 1.9
      (no need to relocate `src/main.rs`). Shared dep versions
      pinned via `[workspace.dependencies]`; rust toolchain pinned
      to `1.94`. `cargo check --workspace` green.
      Refs: `TESTING.md` § 4, § 12.2.
- [x] **1.3** `xtask` crate scaffold with empty subcommands:
      `lint-migrations`, `check-cross-tenant`, `openapi`,
      `test-shape`, `check-doc-fences`, `check-api-doc`. `cargo
      xtask <cmd>` runs each stub via the `[alias]` in
      `.cargo/config.toml`. Each stub names the phase that will
      land its real implementation.
      Refs: `TESTING.md` § 10.1, § 12.5, § 12.7,
      `DOCUMENTATION_PLAN.md` § 11.2.
- [x] **1.4** `.github/actions/rust-setup/action.yml` composite —
      checkout + `dtolnay/rust-toolchain@stable` pinned to `1.94` +
      `Swatinem/rust-cache@v2` with default `shared-key:
      knievel-ci`. Inputs allow per-job override of `shared-key`
      and `components` for the rare case a job needs them.
      Refs: `TESTING.md` § 12.2, § 12.10.
- [x] **1.5** `.github/workflows/ci.yml` + `nightly.yml` +
      `release.yml` skeletons matching the per-PR DAG. Active jobs:
      `prime`, `unit-prop`, `db-integ`, `api-contract`,
      `xtask-lints`, `openapi-drift`. Inactive jobs (`helm-lint`,
      `build-image`, `acceptance`, `gem-smoke`, all of `nightly.yml`,
      most of `release.yml`) carry `if: false` with a comment naming
      the phase that flips it on. `ci.yml` exposes `workflow_call`
      so `release.yml` reuses the DAG.
      Refs: `TESTING.md` § 12.4, § 12.7, § 12.8, § 12.9.
- [x] **1.6** First migration `0001_init.sql`: `knievel.config_version`
      bookkeeping. Schema and extensions are operator-provisioned
      (`MIGRATION_RX.md` "One-time provisioning"); migrations only
      touch their own schema via `SET search_path TO knievel,
      public;`. `_sqlx_migrations` lands in `knievel` automatically
      because of the search_path.
      Refs: `REQUIREMENTS.md` § 7.1, § 7.2, § 7.7.

      **Note (1.6):** `config_version` is implemented as a SEQUENCE
      rather than a single-row table. Functional behavior matches
      `REQUIREMENTS.md` § 7.2 (`SELECT last_value` reads it,
      `SELECT nextval` bumps it). The choice avoids tripping
      gate (2) of § 7.1.1 — that rule requires every `CREATE TABLE`
      in `knievel` to carry RLS, which doesn't fit a non-tenant
      bookkeeping object. Migration carries an in-line comment
      explaining the deviation; revisit `REQUIREMENTS.md` § 7.2
      next time it changes to align the wording.
- [x] **1.7** `xtask lint-migrations` real implementation. All 4
      rules from `REQUIREMENTS.md` § 7.1.1 gate (2). Six fixtures
      from `TESTING.md` § 10.1 land at
      `xtask/tests/fixtures/migrations/`; six unit tests + a
      seventh sanity test that lints the real `migrations/`
      directory. Wired into CI via `xtask-lints` (Phase 1.5).
      Refs: `TESTING.md` § 10.1.

      **Note (1.7):** The implementation strips SQL comments
      (`--` and `/* */`) before regex matching — without that,
      prose like `"CREATE TABLE in knievel"` inside a migration's
      comments tripped rule 3 with a phantom table named `in`.
      Caught and fixed by the `real_migrations_are_clean` sanity
      test against `0001_init.sql`. The comment stripper is naive
      around string literals; if a future migration legitimately
      embeds `--` inside a single-quoted string we'll need to
      switch to `pg_query` for proper SQL parsing.
- [x] **1.8** `xtask check-cross-tenant` walks `openapi.yaml`,
      collects every `/v1/projects/{...}/...` operation, and
      fails if any is missing from `tests/cross_tenant_manifest.toml`.
      Today `openapi.yaml` doesn't exist (Phase 2.8 lands it), so
      the gate runs in skip-mode and exits 0. The manifest file is
      created with explanatory comments and zero entries. Two unit
      tests cover the project-scoped path detection and spec
      walking.
      Refs: `TESTING.md` § 6.5, `REQUIREMENTS.md` § 7.1.1 gate (1).
- [x] **1.9** `testlib::db::ephemeral` creates a uniquely-named
      Postgres database against `DATABASE_URL`, provisions
      schema+pgcrypto, runs migrations, hands back a pool.
      `ephemeral_drop` handles explicit cleanup; CI's per-job
      service container is the broader teardown. Integration test
      `tests/integration_migrations.rs` round-trips
      `0001_init.sql`: applies migrations, asserts
      `nextval('knievel.config_version')` increments 1→2,
      asserts `_sqlx_migrations` lives in the knievel schema. Test
      self-skips with a warning when `DATABASE_URL` is unset so
      contributors without Postgres can still run unit tests.
      Refs: `TESTING.md` § 5.1.

      **Note (1.9):** Docker daemon was unavailable in the
      authoring sandbox so the integration test couldn't be run
      locally; verified by `cargo check --workspace --all-targets`
      (clean) and by running the suite with the test self-skipping.
      First real run lands when the CI workflow executes the
      `db-integ` job against the Postgres service container.

**Milestone:** `cargo nextest run` passes, `cargo xtask
lint-migrations` passes, the CI DAG is green against an empty
business surface. Rails are real before any train rides them.

### Notes

**Post-milestone CI fixes** (caught when the workflows ran on
GitHub Actions for the first time, fixed in two follow-up
commits):

- **Phase 1.4 (composite action)** — local composite actions
  referenced as `./.github/actions/...` can't be resolved before
  `actions/checkout@v4` runs in the calling job. The composite
  was self-checking-out as its first step, which is a chicken-
  and-egg: GitHub can't load `action.yml` from a path that
  doesn't exist on disk yet. Fix in commit `41c5be9`: every
  active job in `ci.yml` now runs `actions/checkout@v4` as its
  first step; the composite drops its self-checkout and
  documents the caller contract.

- **Phase 1.5 / 1.9 (nextest filter + sqlx schema)** — fixed
  together in commit `231489e`:
  - `binary(/^api/)` filter parse-failed because no integration-
    test binary existed yet matching that regex. Added
    `tests/api_placeholder.rs` (comment-only) so the filter has
    something to resolve against; combined with `--no-tests=pass`
    the slice runs zero tests cleanly.
  - `_sqlx_migrations` was landing in `public` instead of
    `knievel`, because `sqlx::migrate` creates the tracking
    table on its first connection BEFORE the migration's own
    `SET search_path` runs. Fix: `testlib::db::ephemeral` now
    configures `after_connect` on `PgPoolOptions` so every
    connection has `search_path = knievel, public` from the
    start, mirroring the production `ALTER ROLE knievel_app SET
    search_path = ...` recipe in `MIGRATION_RX.md`.

**Goal:** A reachable `knievel` process — config loaded, tracing
emitting JSON, HTTP server bound, `/healthz` / `/readyz` /
`/version` / `/openapi.json` reporting honest values. No business
logic yet, but the operational surface is complete enough to put
behind a load balancer and a Prometheus scrape.

**Spec references:**

- `REQUIREMENTS.md` § 10.1 (Configuration), § 10.2 (Logging),
  § 10.6 (Health and readiness), § 10.7 (Graceful shutdown).
- `API.md` § 5 (System endpoints).
- `AUTH.md` "Effective-policy visibility" (`/version` shape).

**Parallelism:** 2.1, 2.2, and 2.3 are sequential (config →
tracing → server bootstrap). 2.4, 2.5, 2.6, 2.7 are independent
once 2.3 lands.

### Tasks

- [x] **2.1** figment config loader + typed `Config` struct.
      Layer: defaults → `config.yaml` (path from `KNIEVEL_CONFIG`)
      → env overrides under `KNIEVEL_` prefix with `__` delimiter.
      `${VAR}` and `${VAR:default}` interpolation applied to the
      raw YAML before parse; missing vars without defaults are a
      hard error reporting all unresolved names at once.
      Six inline tests cover interpolation behaviour and the
      no-file/no-env defaults path.
      Refs: `REQUIREMENTS.md` § 10.1.

      **Note (2.1):** Typed sections today are `api`, `database`,
      `logging`, `tracing`, `errors` (sentry/otel sub-blocks
      stubbed). Remaining sections (`auth`, `events`, `hmac`,
      `metrics`) are typed up as their consumer features land.
      Module carries `#![allow(dead_code)]` to keep clippy `-D
      warnings` happy until consumers land.
- [x] **2.2** `src/observability.rs` — tracing subscriber init
      driven by `cfg.logging.{level,format}`. JSON layer (default)
      flattens events; `compact`/`text` available for dev. Filter
      parses as `EnvFilter` so `knievel=info,sqlx=warn` style
      directives work. OTel and Sentry honor their `enabled`
      flags but log a "stub" line — real exporters land in Phase
      3 alongside their first consumers.
      Refs: `REQUIREMENTS.md` § 10.2, § 10.3, § 10.4.

      **Note (2.2):** Successful init has a process-global side
      effect (sets the tracing default dispatcher), so we don't
      unit-test the success path. Two negative tests cover the
      error paths (invalid format, invalid level directive); the
      success path is exercised by the binary at runtime and
      eventually by the acceptance suite.
- [x] **2.3** `src/server.rs` binds `poem` at `cfg.api.bind_addr`,
      installs SIGTERM/SIGINT handlers, runs with poem's graceful
      shutdown helper. Drain timeout from
      `cfg.api.shutdown_drain_timeout_secs` (default 30 s; total
      budget 60 s). Empty `Route::new()` today — handlers wired
      in 2.4–2.7.
      Refs: `REQUIREMENTS.md` § 10.7.
- [x] **2.4** `src/system.rs::healthz` returns `200 ok\n`. Wired
      into `server::routes()`. Two `poem::test::TestClient` tests:
      one against the handler in isolation, one through the
      production routes table to catch wiring regressions. Server
      `Route` is a single `at("/healthz", get(...))` today;
      subsequent endpoints chain on as they land.
      Refs: `API.md` § 5, `REQUIREMENTS.md` § 10.6.
- [x] **2.5** `src/state.rs::AppState` carries the optional
      `PgPool` (snapshot/events/leader fields land alongside their
      subsystems in Phase 3+). `server::build_state()` connects
      to Postgres at boot when `database.url` is set; failure is
      non-fatal during Phase 2 (server still starts; `/readyz`
      reports 503). `/readyz` checks `SELECT 1`; returns 200 with
      `ok\n` when reachable, `200 ok: no_db_configured\n` when no
      URL, `503 not_ready: db_unreachable\n` on error.
      Refs: `API.md` § 5, `REQUIREMENTS.md` § 10.6, § 10.9.

      **Note (2.5):** Only the DB-reachability check is real today
      (REQUIREMENTS.md § 10.6 lists four conditions; the
      snapshot, flusher, and leader checks land alongside their
      subsystems in Phase 3+). The DB-unreachable HTTP-level
      assertion lands in db-integ once Phase 3's test harness
      threads a real `PgPool` through `AppState`.
- [x] **2.6** `/version` handler returns JSON with `knievel`,
      `schema`, `git_sha`, `build_timestamp`, and `auth` (modes/
      issuers — empty until Phase 3.16). `build.rs` shells out to
      `git rev-parse HEAD` (with `-dirty` suffix when the working
      tree isn't clean) and `date -u` rather than pulling in vergen
      — the metadata needed is small enough to skip the dep.
      `cargo:rerun-if-changed` on `.git/HEAD` and `.git/index` so
      a new commit triggers a rebuild of the version metadata.
      Refs: `API.md` § 5, `AUTH.md` "Effective-policy visibility."
- [x] **2.7** `poem-openapi` setup. `SystemApi` is a single
      `#[OpenApi]` impl carrying `/healthz`, `/readyz`, `/version`
      with typed responses (`PlainText`, `ApiResponse` enum, typed
      `Object`). `OpenApiService::new(SystemApi, "knievel", PKG_VERSION)`
      mounts the operations and exposes `/openapi.json` via
      `.spec_endpoint()`. The fourth test asserts the served spec
      lists all three system paths.
      Refs: `REQUIREMENTS.md` § 3, `API.md` "Path Structure."
- [x] **2.8** `xtask openapi` and `xtask openapi --check` are real.
      Knievel root crate gained an `src/lib.rs` exposing
      `openapi_spec_yaml()` so xtask can build the spec without
      spawning a server. Initial `openapi.yaml` (2.5 KB) committed.
      `cargo xtask check-cross-tenant` is no longer skipping —
      with the spec present it walks paths and reports
      `0 project-scoped endpoint(s), all covered`.
      Refs: `TESTING.md` § 6.3, § 12.7.

      **Note (2.8):** `poem-openapi` 5 emits OpenAPI **3.0.0**
      while `REQUIREMENTS.md` § 6 specifies 3.1. v0 ships 3.0
      since that's what the library supports; revisit when
      `poem-openapi` adds 3.1 emission, or wrap the spec in a
      post-processing step.
- [x] **2.9** Phase milestone confirmed locally. Every per-PR
      gate from `TESTING.md` § 12.7 runs green:
      - `cargo fmt --check` — clean.
      - `cargo clippy --workspace --all-targets --locked -- -D warnings` — clean.
      - `cargo test --workspace` — 23 tests passing across the
        binary, lib, xtask, testlib, and integration crates.
      - `cargo xtask lint-migrations` — 1 file clean
        (`migrations/0001_init.sql`).
      - `cargo xtask check-cross-tenant` — 0 project-scoped
        endpoints, all covered (will be real once Phase 3 starts
        adding endpoints).
      - `cargo xtask test-shape` — stub (Phase 5.6 will implement).
      - `cargo xtask openapi --check` — `openapi.yaml` matches
        binary spec (2508 bytes).

      The integration test that needs real Postgres
      (`tests/integration_migrations.rs`) self-skips locally
      without `DATABASE_URL` and will run against the CI Postgres
      service container in the `db-integ` job.

**Milestone:** met. `cargo run` starts a server that responds to
`/healthz`, `/readyz`, `/version`, and `/openapi.json` with honest
values; the OpenAPI spec is the contract; the migration linter
and cross-tenant gate guard the very first migrations and
endpoints; the CI pipeline is in place to keep them honest. Rails
are real before any train rides them.

### Notes

- **Phase 1.5 → Phase 4:** several CI jobs (helm-lint,
  build-image, acceptance, gem-smoke, every job in nightly.yml,
  most of release.yml) carry `if: false` with a comment naming
  the phase that flips them on. Phase 4 owns most of the flips.
- **Phase 1.6 spec follow-up:** `REQUIREMENTS.md` § 7.2 calls
  `config_version` a "row in a bookkeeping table" but it's a
  SEQUENCE in the implementation (see Phase 1.6 note). Update the
  spec wording the next time § 7.2 changes.
- **Phase 2.8 spec follow-up:** `poem-openapi` 5 emits OpenAPI
  3.0.0 while `REQUIREMENTS.md` § 6 specifies 3.1. Decide whether
  to wait for `poem-openapi` to add 3.1 emission or post-process
  the spec.

---

## Phase 3 — Thicken to v0 surface

**Goal:** Every endpoint in `API.md` returns the documented shape.
Hot path proven end-to-end with snapshot loader + COPY flusher +
HMAC sign/verify. Auth real on both opaque and JWT paths. Partition
manager and leader election running.

**Spec references:**

- `REQUIREMENTS.md` §§ 4–7, 10.9.
- `API.md` (whole file).
- `AUTH.md` (whole file).

**Parallelism:** the phase reads top-to-bottom along five rails:

- **Tenancy + auth rail** (3.1 → 3.6) is sequential — every later
  task assumes RLS-bound projects and a working `Principal`.
- **Resource-CRUD rail** (3.7 → 3.14) follows once 3.6 lands and
  parallelizes per-resource (3.10/3.11/3.12/3.13 are independent
  of each other after the macro in 3.8).
- **Hot-path rail** (3.15 → 3.19) needs the tenant model and at
  least one resource (Site) but is otherwise independent of the
  CRUD rail; selection (3.15) and HMAC (3.16) are independent
  pure-Rust modules and can land in either order.
- **Events + periodic-jobs rail** (3.20 → 3.25) needs the leader
  (3.22) before partition manager (3.23) and rollup (3.24).
- **JWT + Ad Library + image upload** (3.26 → 3.29) are additive
  finishers; each unblocks an acceptance scenario.

### Tasks

- [ ] **3.1** Tenant model migration `0002`: `organizations`,
      `projects`, RLS policies bound on
      `current_setting('knievel.project_id')` (and `org_id` for
      org-scoped rows). Tenant-binding helper exposed via testlib
      (`testlib::tenant::with_project`). Integration test asserts a
      session bound to project A cannot read rows inserted under
      project B.
      Refs: `REQUIREMENTS.md` § 4, § 7.1, § 7.1.1, `AUTH.md`
      "Authorization."
- [ ] **3.2** Opaque-token foundation: `auth::opaque` parse +
      argon2id hash/verify, `auth::role` enum + ordering,
      migration `0003_api_tokens.sql` (tokens table with RLS by
      `org_id`), `Principal` poem-openapi extractor (opaque path
      only — JWT lands in 3.26). Unit tests per `TESTING.md` § 4.1
      for `auth::opaque::parse`, `auth::opaque::hash`, `auth::role`.
      Refs: `AUTH.md` "Opaque Tokens," "Authorization,"
      `REQUIREMENTS.md` § 4.3.
- [ ] **3.3** First handler — `POST /v1/orgs/{orgId}/projects` and
      `GET /v1/orgs/{orgId}/projects/{projectId}`. `OrgApi`
      `OpenApiService` mounted alongside `SystemApi`. Insta-snapshot
      contract test for `create_returns_201`. Cross-org negative
      test asserts an org-A token receives `403 wrong_tenant` when
      addressing org B. Manifest entry not required (org-scoped
      paths aren't gated by `xtask check-cross-tenant`); the
      negative test rides in `tests/api/orgs_projects.rs`. Updates
      `openapi.yaml`.
      Refs: `API.md` § 2.1, `TESTING.md` § 6.4, § 6.5.
- [ ] **3.4** Audit-log migration `0004`: `audit_log` (monthly
      range-partitioned, RLS by `org_id`, append-only enforced via
      policy — `UPDATE`/`DELETE` rejected). Integration test
      asserts append-only behavior. No writers yet — first writer
      lands in 3.6 (token mint), then 3.19 (force.*).
      Refs: `REQUIREMENTS.md` § 7.3.
- [ ] **3.5** Idempotency middleware (24 h replay). Migration
      `0005_idempotency_keys.sql` — per-project store keyed on
      `(project_id, key, route, body_hash)`. Middleware fits
      between auth and handler; replay returns cached body with
      `Idempotent-Replay: true`; body mismatch → `409
      idempotency_conflict`. Reaper deferred to leader (3.22).
      Refs: `API.md` "Idempotency," `TESTING.md` § 6.4.
- [ ] **3.6** Org-level Tokens API — `POST/GET/DELETE
      /v1/orgs/{orgId}/tokens`. Mint returns the secret exactly
      once; subsequent reads are metadata-only. Token mint emits
      one `audit_log` row (first real audit writer). Org-scope
      cross-tenant negative test included.
      Refs: `API.md` § 2.2, `AUTH.md` "Opaque Tokens."
- [ ] **3.7** Inventory + demand-chain migrations. Three migrations
      sequenced: `0006_demand.sql` (advertisers, campaigns,
      flights, ads, creatives, creative_templates),
      `0007_inventory.sql` (sites with aliases, zones),
      `0008_taxonomy.sql` (channels, priorities, ad_types — seeded
      defaults). Every table RLS-bound; FK relationships match
      `API.md` §§ 3.1–3.9.
      Refs: `API.md` §§ 3.1–3.9, `REQUIREMENTS.md` § 5.
- [ ] **3.8** `crud_contract!` macro — emits the 11-test table from
      `TESTING.md` § 6.4 from a single invocation
      (`create_returns_201`, `create_idempotent_on_external_id`,
      idempotency-key replay, etag matching, listing/pagination,
      soft delete, batch atomic, cross-entity FK in batch). One
      worked instance against `Advertiser` to validate the shape.
      Refs: `TESTING.md` § 6.4.
- [ ] **3.9** Demand-chain CRUD (advertisers, campaigns, flights).
      Each resource gets its `crud_contract!` invocation, handler
      module, and cross-tenant manifest entry per project-scoped
      operation.
      Refs: `API.md` §§ 3.1–3.3.
- [ ] **3.10** Creative + CreativeTemplate CRUD. CreativeTemplate
      requires the `poem-openapi` JSON-Schema round-trip spike
      called out in cross-cutting risk (1) — runs as a
      proof-of-concept test before the handler lands. Image upload
      stays deferred to 3.29.
      Refs: `API.md` § 3.5, § 3.6, cross-cutting risk (1).
- [ ] **3.11** Ad CRUD — inline-creative variant only; library
      reference deferred to 3.28 once the Ad Library lands. Schema
      reserves the `oneOf` shape so 3.28 is additive.
      Refs: `API.md` § 3.4.
- [ ] **3.12** Site + Zone CRUD; `:upsertByUrl` natural-key
      endpoint for sites. Site URL/aliases uniqueness enforced at
      the table level.
      Refs: `API.md` § 3.7, § 3.8.
- [ ] **3.13** Read-only inventory taxonomy endpoints
      (channels/priorities/ad-types). Seeded by 3.7.
      Refs: `API.md` § 3.9.
- [ ] **3.14** `:batchUpsert` — single Postgres transaction with
      per-row diagnostics matching `API.md` "Write contract."
      Cross-entity FK validation inside the transaction (a flight
      created earlier in the array resolves for an ad later in the
      array). Wired into every CRUD resource that declares it.
      Refs: `API.md` "Write contract," `TESTING.md` § 6.4.
- [ ] **3.15** Selection algorithm — `selection::filter` (site /
      zone / ad_type / date), `selection::priority` (highest
      non-empty tier wins), `selection::weighted_random` (seeded
      `StdRng`). Pure-Rust unit tests per `TESTING.md` § 4.1, plus
      `proptest` for the priority + blocklist invariant.
      Refs: `API.md` § 1, `REQUIREMENTS.md` § 6.1.
- [ ] **3.16** HMAC sign + verify with 8 h rotation overlap.
      Per-project secret stored on the `projects` row from 3.1.
      `proptest` over the rotation window confirming `dedup_key`
      stability across rotation. Cross-cutting risk (3) lands here.
      Refs: `API.md` § 4 "Signature payload,"
      `REQUIREMENTS.md` § 6.3.
- [ ] **3.17** Snapshot loader — cold load, `LISTEN
      config_changed`, 5 s poll backstop, Aurora-failover
      reconnect-with-backoff. Snapshot keyed by `(project_id,
      resource)`. Integration tests under `tests/integration/`
      cover the load + diff + swap path.
      Refs: `REQUIREMENTS.md` § 7.2, `TESTING.md` § 5.2.
- [ ] **3.18** Decision API — `POST
      /v1/projects/{projectId}/decisions`. Wires snapshot + 3.15 +
      3.16. HMAC-minted impression/click URLs in the response.
      Refs: `API.md` § 1.
- [ ] **3.19** Decision explainer — `POST
      /v1/projects/{projectId}/decisions:explain`. Three-control
      gate for `force.*` (`allow_force_decision` project flag,
      Project Admin role, global kill-switch); each forced call
      writes one `audit_log` row.
      Refs: `API.md` § 1, `AUTH.md` "Endpoint → minimum role."
- [ ] **3.20** Events migration `0009_events_raw.sql` — partitioned
      by day on `ts`, no default partition, no secondary indexes,
      RLS by `org_id`. First leaf for today included; partition
      manager creates the rest from 3.23.
      Refs: `REQUIREMENTS.md` § 7.3, § 7.6.
- [ ] **3.21** Event channel + COPY flusher. Bounded
      `tokio::sync::mpsc`, drain every 1–2 s or 5 k events, `COPY`
      to `events_raw`. Channel saturation → `503
      event_channel_saturated` on the decision endpoint. `dedup_key`
      computation per `API.md` "Replay, dedup, and counts."
      Graceful shutdown drains the channel.
      Refs: `REQUIREMENTS.md` § 7.6, `API.md` § 4.
- [ ] **3.22** Leader election — `pg_try_advisory_lock` on a
      dedicated connection, watchdog ("must complete a maintenance
      run every N hours" → process exits on miss), `/readyz`
      reflects leader/follower state. Idempotency-key reaper
      (3.5 follow-up) hangs off the leader.
      Refs: `REQUIREMENTS.md` § 7.5.
- [ ] **3.23** Partition manager — premake 4 days of
      `events_raw_p<YYYY_MM_DD>` partitions, retention drop with
      `DETACH PARTITION CONCURRENTLY`. Runs hourly off the 3.22
      leader handle. Idempotent.
      Refs: `REQUIREMENTS.md` § 7.4.
- [ ] **3.24** Migration `0010_events_rollup.sql` + leader-elected
      hourly rollup compute. Watermark advances monotonically; only
      `is_duplicate = false` rows feed the rollup.
      Refs: `REQUIREMENTS.md` § 7.3, `REPORTING.md` § "Schema for
      Reporters."
- [ ] **3.25** Event endpoints — `GET /e/i/{signed}` (204 default,
      `?fmt=gif` GIF), `GET /e/c/{signed}` (302 redirect, signed
      open-redirect block via `?u=`).
      Refs: `API.md` § 4.
- [ ] **3.26** JWT validator + JWKS cache + `claim_mapping` +
      boot-time auth lint. Issuer auto-discovery via
      `/.well-known/openid-configuration`; per-issuer `kid` index;
      cache miss triggers a refresh; algorithm allow-list rejects
      `alg: none` and any `HS*`. Mocked OIDC provider via
      `wiremock`.
      Refs: `AUTH.md` "JWTs," "Kubernetes ServiceAccount Tokens,"
      "Startup Linting."
- [ ] **3.27** `/version` real auth block — issuers, audiences,
      algorithms, claim source (`claim` or `claim_mapping` rule
      count), JWKS URL. Mirrors the startup INFO log line. Updates
      `openapi.yaml`.
      Refs: `AUTH.md` "Effective-policy visibility."
- [ ] **3.28** Ad Library (org-scoped) — migration
      `0011_ad_library.sql`, CRUD per `API.md` § 2.4, Ad-side
      `oneOf` reference (`adLibraryItemId`) wired through 3.11 and
      the snapshot loader. References resolve at decision time.
      Refs: `REQUIREMENTS.md` § 5.1, `API.md` § 2.4, § 3.4.
- [ ] **3.29** S3-compatible image upload. `POST
      /v1/projects/{projectId}/creatives/{id}/image` (multipart),
      magic-byte sniffing per `REQUIREMENTS.md` § 7.9, returns
      `imageUrl`. Adapter trait so MinIO/S3/GCS back ends share
      code.
      Refs: `REQUIREMENTS.md` § 7.9, `API.md` § 3.5.

**Milestone:** Every endpoint in `API.md` returns the documented
shape. Full API-contract suite + cross-tenant suite green for every
project-scoped endpoint.

### Notes

(none yet)

---

## Phase 4 — Make it deployable

**Goal:** Anyone can `docker compose up` and get a working knievel
or `helm install` it into a real cluster. Acceptance suite running
in CI. Generated Ruby gem published from the OpenAPI spec.

**Spec references:**

- `REQUIREMENTS.md` § 8 (Deliverables), § 8.1 (Helm chart).
- `TESTING.md` § 7 (E2E Acceptance).
- `DOCUMENTATION_PLAN.md` § 6 (DEPLOYMENT.md).

**Tasks (broad strokes):**

- [ ] **4.1** `examples/compose/` reference stack — `knievel-cli
      seed-demo` is the canonical fixture.
      Refs: `MIGRATION_RX.md` "Local Development for RX Engineers,"
      `TESTING.md` § 11.1.
- [ ] **4.2** `knievel-cli seed-demo` implementation.
      Refs: `REQUIREMENTS.md` § 8 item 4, `AUTH.md` "Local
      Development."
- [ ] **4.3** Acceptance scenarios ACC-01..30.
      Refs: `TESTING.md` § 7.1.
- [ ] **4.4** Acceptance sharding in CI (4-way nextest partition).
      Refs: `TESTING.md` § 12.6.
- [ ] **4.5** `charts/knievel` Helm chart; `helm lint` +
      `kubeconform` gate.
      Refs: `REQUIREMENTS.md` § 8.1.
- [ ] **4.6** Multi-arch container image build (`docker buildx`,
      amd64 + arm64); `cosign` signing.
      Refs: `REQUIREMENTS.md` § 8 item 5.
- [ ] **4.7** Chaos suite skeleton paired 1:1 with
      `REQUIREMENTS.md` § 10.9.
      Refs: `TESTING.md` § 9.
- [ ] **4.8** `openapi-generator-cli` wired into CI; Ruby gem with
      `Resource` wrappers + `Enumerable` pagination; gem-smoke job.
      Refs: `REQUIREMENTS.md` § 8 item 3, `API.md` "Pagination."

**Milestone:** `docker compose up` boots a working knievel against
Postgres + MinIO + wiremock; a third party can integrate from the
gem alone.

### Notes

(none yet)

---

## Phase 5 — Ship v0.1.0

**Goal:** Tag a release that an operator can build a deployment
plan around. Docs pass CI, bench artifact present, security
checklist green.

**Spec references:**

- `DOCUMENTATION_PLAN.md` (whole file).
- `REQUIREMENTS.md` § 7.1.1 gate (3) (release security checklist).
- `TESTING.md` § 8 (Performance), § 12.9 (Release workflow).

**Tasks (broad strokes):**

- [ ] **5.1** `README.md` — landing page with working quickstart.
      Refs: `DOCUMENTATION_PLAN.md` § 4.
- [ ] **5.2** `ARCHITECTURE.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 5.
- [ ] **5.3** `DEPLOYMENT.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 6.
- [ ] **5.4** `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 7.
- [ ] **5.5** `RELEASE_CHECKLIST.md`, `RELEASE_PLAYBOOK.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 7.4, § 7.5.
- [ ] **5.6** `xtask check-doc-fences`, `check-api-doc`, lychee
      link checking in CI.
      Refs: `DOCUMENTATION_PLAN.md` § 11.2.
- [ ] **5.7** First benchmark run; `bench/results/v0.1.md`
      committed.
      Refs: `REQUIREMENTS.md` § 9.2, `TESTING.md` § 8.
- [ ] **5.8** Release-tagging workflow — tag `v0.1.0`, multi-arch
      image published, gem published, GitHub Release created.
      Refs: `TESTING.md` § 12.9.

**Milestone:** `v0.1.0` tagged. Container image, Helm chart, and
gem published.

### Notes

(none yet)

---

## Cross-cutting risks (front-load these)

Carried over from the conversational plan; revisit at the start of
each phase.

1. **`poem-openapi` JSON Schema round-trip** — the
   `CreativeTemplate.schema` field is an arbitrary JSON Schema
   document. Verify `poem-openapi` round-trips it through the
   generated OpenAPI without flattening or escaping. **Spike before
   Phase 3.5** (CreativeTemplate handlers); recorded as Open
   Question in `REQUIREMENTS.md` § 12.

2. **Aurora-specific behavior** — NOTIFY drop on failover, advisory
   lock release semantics. Simulated in Phases 1–3; budget a week
   of staging-cluster validation before Phase 5 tag.

3. **HMAC rotation overlap** — 8 h dual-secret window with stable
   `dedup_key` across rotation is subtle. Land with `proptest`
   coverage in **Phase 3.9**, not as a Phase 4 follow-up.

---

## Conventions for this file

- Every task that lands updates this file in the same commit.
- Commit subject: `Phase X.Y: <imperative task name>`. Subject is
  the audit trail; the file marks `[x]` and adds notes if anything
  surprised us.
- A task that splits into more than one commit gets its sub-commits
  prefixed `Phase X.Y.N:` and the parent task line annotates the
  split in **Notes**.
- A task that changes scope updates the task description in the
  same commit that does the work.
- Don't delete completed tasks. The `[x]` lines are the audit
  trail.
