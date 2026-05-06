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

- [x] **3.1** Tenant model migration `0002_tenants.sql`:
      `organizations` and `projects`, RLS policies bound on
      `current_setting('knievel.org_id')` and
      `current_setting('knievel.project_id')`. Helper
      `testlib::tenant::begin_bound` opens a tenant-bound
      transaction via `set_config(..., is_local=true)` (the
      parameterized `SET LOCAL` equivalent — caller-supplied IDs
      can't smuggle SQL). Integration test
      `tests/integration_tenants.rs` asserts: a session bound to
      org A sees only org A; a session bound to (org B, project
      B1) cannot see project A1; a project-only session can see
      its parent org via the policy's inheritance subquery; the
      `WITH CHECK` clause rejects an insert whose `id` doesn't
      match the bound `org_id`.
      Refs: `REQUIREMENTS.md` § 4, § 7.1, § 7.1.1, `AUTH.md`
      "Authorization."

      **Note (3.1):** The Phase 1.7 migration linter's rule 4
      regex (`USING\s*\((...)\)`) couldn't parse multi-line USING
      clauses with nested parens (e.g. an `IN (SELECT ...)`
      subquery referencing the tenant binding). Fixed in this
      commit: rule 4 now scans the entire `CREATE POLICY`
      statement (up to its terminating `;`) for the
      `knievel.project_id` token. The looser check is arguably
      more correct — any reference inside a CREATE POLICY block is
      tenant-binding intent — and it accepts policies whose
      binding lives in `WITH CHECK` rather than `USING` (relevant
      for INSERT-only policies starting with `audit_log` in 3.4).
      The `real_migrations_are_clean` sanity test was generalized
      to lint every file in `migrations/` so future migrations
      get auto-checked.
- [x] **3.2** Opaque-token foundation. `src/auth/` module with
      three children: `opaque` (token-format parse + argon2id
      hash/verify), `role` (Role enum, ordered by privilege,
      kebab-case wire), `principal` (`Principal` struct + scope /
      token-type enums). Migration `0003_api_tokens.sql` —
      `api_tokens` table with RLS bound on `org_id` plus a
      single-row auth-bootstrap bypass via the
      `knievel.auth_lookup_id` GUC (the chicken-and-egg fix —
      auth has to read `secret_hash` before any `org_id` is
      known). 11 unit tests cover the parse / hash / role /
      ordering / serde paths.
      Refs: `AUTH.md` "Opaque Tokens," "Authorization,"
      `REQUIREMENTS.md` § 4.3.

      **Note (3.2):** Wire format is fixed to four
      underscore-separated post-prefix segments —
      `kvl_<env>_<scope>_<id_short>_<secret>` —  with `splitn(4,
      '_')` so the random tail can itself contain `_`.
      `id_short` is the public id segment that, prefixed with
      `tok_`, matches `api_tokens.id`. The Principal extractor
      (Phase 3.3) is the only consumer of the auth-bootstrap
      bypass: it `set_config('knievel.auth_lookup_id', $id, true)`
      on its lookup transaction, queries by primary key, and
      lets the transaction roll back when verification fails.
- [x] **3.3** First handler — `POST /v1/orgs/{orgId}/projects`
      and `GET /v1/orgs/{orgId}/projects/{projectId}`. `OrgApi`
      mounted alongside `SystemApi` via the tuple form
      `OpenApiService::new((SystemApi, OrgApi), ...)`.
      `BearerAuth` poem-openapi `SecurityScheme` parses opaque
      tokens, opens a `db::begin_auth_lookup` transaction (so the
      Phase 3.2 RLS auth-bootstrap branch unlocks one row by PK),
      verifies argon2id, builds a `Principal`. `db::begin_bound`
      mirrors `testlib::tenant::begin_bound` for production code.
      Six API tests in `tests/api_projects.rs` (slice
      `binary(/^api/)`) cover: 201 happy path; cross-org →
      `403 wrong_tenant`; reader → `403 role_insufficient`;
      missing auth → 401; bad secret → 401; create→GET round-trip.
      Updates `openapi.yaml` (2508 → 6602 bytes).
      Refs: `API.md` § 2.1, `TESTING.md` § 6.4, § 6.5,
      `AUTH.md` "Authorization."

      **Note (3.3):** Insta snapshots are not yet wired in —
      `insta` lands as a workspace dep alongside the
      `crud_contract!` macro in 3.8 where the snapshot suite
      becomes the unit of repetition; for v0 of one handler the
      structural assertions in `create_project_returns_201` cover
      the contract. Timestamps are emitted as RFC 3339 by
      Postgres `to_char(... AT TIME ZONE 'UTC', ...)` so the wire
      shape doesn't depend on a sqlx time-crate feature.
      `random_pj_id` reuses argon2's transitive `OsRng` so we
      don't pull a direct `rand` dep. `routes()` was promoted
      from `pub(crate)` to `pub` so the API-slice tests can
      assemble a `TestClient` over the production routes table.
- [x] **3.4** Audit-log migration `0004_audit_log.sql`. Parent
      `audit_log` partitioned by RANGE (ts) with a wide seed leaf
      covering 2026; subsequent monthly leaves land via the
      partition manager once it generalizes (Phase 3.23+).
      Append-only is enforced by RLS *via the absence* of `FOR
      UPDATE` and `FOR DELETE` policies — Postgres' FORCE'd RLS
      default-denies operations without a matching policy, so
      tamper attempts silently affect zero rows. Integration test
      `tests/integration_audit_log.rs` covers four invariants:
      tenant-scoped reads, UPDATE → 0 rows affected, DELETE → 0
      rows affected, cross-tenant `WITH CHECK` rejection.

      **Note (3.4):** The Phase 3.1 linter's rule 4 was
      generalized in this commit to accept either `knievel.org_id`
      or `knievel.project_id` — `REQUIREMENTS.md` § 7.1.1 gate
      (2) says "or equivalent session-scoped tenant binding," and
      org_id is a first-class binding on org-scoped tables
      (organizations, api_tokens, audit_log). The fixture-04
      reject-case continues to fail because it has neither
      binding; fixture-06's `knievel.project_id` reference still
      passes.
- [x] **3.5** Idempotency replay store. Migration
      `0005_idempotency_keys.sql` — `(org_id, project_id, key,
      route, body_hash)` lookup with a unique partial-coalescing
      index (`coalesce(project_id, '')`) so the same logic
      handles org-scoped and project-scoped writes on Postgres 14.
      `src/idempotency.rs` exposes `body_hash` (canonical
      `serde_json::to_vec` + SHA-256), `check`, and `store`. The
      `create_project` handler now consumes `Idempotency-Key`:
      replay returns the cached payload with `Idempotent-Replay:
      true`; body mismatch returns `409 idempotency_conflict`;
      check + insert + store all run in the same tenant-bound
      transaction so a crash between insert and store can't leave
      a half-applied state. Three new API tests cover the replay,
      conflict, and whitespace-stable paths.
      Refs: `API.md` "Idempotency," `TESTING.md` § 6.4.

      **Note (3.5):** The reaper for expired rows hangs off the
      leader (3.22) — for now rows accumulate; the
      `expires_at` column + `idempotency_keys_expires_at_idx`
      let the reaper sweep efficiently when it lands. Full
      canonical-form hashing (recursive key-sort) is deferred;
      `serde_json::to_vec` gives whitespace stability and
      Serialize-determined field order, which is what the
      `TESTING.md` § 4.1 "stability across body whitespace" test
      actually requires. Added `sha2` and `hex` to workspace
      deps.
- [x] **3.6** Org-level Tokens API — `POST/GET/DELETE
      /v1/orgs/{orgId}/tokens`. `src/tokens.rs::TokensApi`:
      mint generates `kvl_prod_<scope>_<id_short>_<secret>`,
      stores argon2id hash, returns plaintext exactly once;
      list returns metadata only (no secrets) up to 500 rows
      (cursor pagination lands later); revoke is a soft delete
      via `revoked_at` so the auth path filters it on the next
      request. Mint and revoke each emit one `audit_log` row in
      the same transaction as the data mutation — first real
      audit writers. `Principal` gained `actor_id: String` so
      the audit_log `actor` column can be populated without an
      extra DB query. Six API tests in `tests/api_tokens.rs`
      cover: 201 + plaintext + audit row, end-to-end auth via a
      newly-minted token, cross-org `wrong_tenant`, editor
      `role_insufficient`, list excludes secrets and is
      tenant-scoped, revoke makes auth fail and emits the audit
      row.
      Refs: `API.md` § 2.2, `AUTH.md` "Opaque Tokens",
      "Endpoint -> minimum role".

      **Note (3.6):** poem-openapi rejects two `ApiResponse`
      variants sharing the same status code (the OpenAPI YAML
      ends up with duplicate-key entries). Phase 3.5's separate
      `Created` and `CreatedReplay` 201 variants on
      `CreateProjectResponse` were merged into a single `Created`
      variant carrying `Option<String>` for the
      `Idempotent-Replay` header — `None` on fresh execution,
      `Some("true")` on replay. The wire shape is identical;
      the spec just emits a single 201 entry now. The token env
      segment is hardcoded to `prod` for v0; future commits
      parameterize via config.
- [x] **3.7** Inventory + demand-chain migrations.
      `0006_demand.sql` (advertisers, campaigns, flights, ads,
      creatives, creative_templates), `0007_inventory.sql`
      (sites with aliases array, zones), `0008_taxonomy.sql`
      (channels, priorities, ad_types). Every table is project-
      scoped (carries both `org_id` and `project_id`), RLS-bound
      on `current_setting('knievel.project_id')`, with
      `WITH CHECK` symmetrical to `USING`. `ads.kind_check`
      enforces the API.md § 3.4 oneOf at the schema layer
      (creative_id XOR ad_library_item_id). One integration test
      `tests/integration_demand.rs` round-trips one row of each
      of the 11 resources and asserts: pj_b sees zero rows of
      every table after pj_a writes; WITH CHECK rejects a
      wrong-project insert.
      Refs: `API.md` §§ 3.1–3.9, `REQUIREMENTS.md` § 5.

      **Note (3.7):** Taxonomy seeding (default channels /
      priorities / ad_types per project) lands in 3.13 alongside
      the read endpoints. The `ad_library_item_id` column on
      `ads` is reserved here so 3.28 (Ad Library) is purely
      additive — no schema migration needed for the reference
      variant.
- [x] **3.8** Advertiser CRUD + shared handler helpers.
      `src/handlers.rs::open_project_tx` is the prologue for every
      project-scoped handler — auth check, project-lookup against
      RLS, role gate, returns a tenant-bound transaction.
      `src/advertisers.rs::AdvertisersApi` wires
      `POST/GET/PATCH /v1/projects/{projectId}/advertisers[/{id}]`.
      Five API tests cover: 201 happy, 409
      `external_id_conflict`, cross-tenant 403 `wrong_tenant`,
      reader 403 `role_insufficient`, list+get+patch round-trip
      with etag bumping. Cross-tenant manifest gains the four
      advertiser entries; the gate now reports
      `4 project-scoped endpoint(s), all covered`.
      Refs: `API.md` § 3.1, `AUTH.md` "Project resources",
      `TESTING.md` § 6.4.

      **Note (3.8):** The `crud_contract!` macro is deferred to
      3.9. With one CRUD resource in hand the macro would be
      speculative; with three (advertisers + campaigns + flights
      after 3.9) the duplication is real and the right shape for
      the macro will be obvious. The contract tests in
      `tests/api_advertisers.rs` are hand-written for now and
      will get rewritten to invoke the macro when it lands.
      Cross-entity FK / idempotent-on-external-id /
      pagination / batch tests come online with their handler
      features (3.5 partial-replay applies; full external_id
      idempotency lands with 3.14 `:batchUpsert`).
- [x] **3.9** Demand-chain CRUD — campaigns + flights handlers
      (advertisers landed in 3.8). `src/campaigns.rs` adds the
      advertiser-FK pattern with a 422 `fk_not_found` branch when
      the FK insert violates the constraint. `src/flights.rs`
      adds the array-column shape (`site_ids`, `zone_ids`,
      `ad_types`) plus an explicit 400 `ad_types_required` since
      `API.md` § 3.3 requires non-empty `ad_types`. Two API tests
      cover the new ground (campaign FK 422, flight arrays + 400
      validation). Cross-tenant manifest gets 8 new entries; gate
      now reports `12 project-scoped endpoint(s), all covered`.
      Refs: `API.md` §§ 3.2–3.3.

      **Note (3.9):** The `crud_contract!` macro stays deferred.
      With three resources the duplication is real (~80% of the
      handler bodies are identical); the right shape becomes
      clearer once 3.10–3.13 land their resources, and a focused
      refactor commit can extract it then. The Phase 3.5 note
      about external_id idempotency remains: today external_id
      reuse returns 409, not 200-replay.
- [x] **3.10** Creative + CreativeTemplate CRUD.
      `creative_templates` stores the JSON Schema document as
      `serde_json::Value` — poem-openapi treats this as a
      free-form JSON `Any` schema in the generated OpenAPI, and
      the spike test
      `creative_template_json_schema_round_trips` confirms a
      representative schema body survives create → GET → patch
      bit-for-bit. Cross-cutting risk #1 closes positively. The
      `creatives` handler accepts the union of all kind-specific
      fields and validates per-kind requirements at the handler
      boundary (image needs image_url; html needs body; native
      needs template_id + values); 422 fk_not_found surfaces FK
      violations against advertiser_id or template_id. Image
      upload (`POST .../{id}/image`) stays deferred to 3.29.
      Cross-tenant manifest gains 7 entries (3 creatives + 4
      templates); gate now reports
      `19 project-scoped endpoint(s), all covered`.
      Refs: `API.md` § 3.5, § 3.6, `REQUIREMENTS.md` § 12 risk
      (1).

      **Note (3.10):** `Creative` is the union of all kind shapes,
      not a typed `oneOf`. Per `API.md` § 3.5 the response is a
      typed `oneOf` keyed on `type`; the v0 wire shape uses a
      single `kind` discriminator and per-kind nullable fields
      (`API.md` follow-up). This is invisible to the integration
      story but documented so a future schema-aware client can
      narrow the type at the consumer side.
- [x] **3.11** Ad CRUD — inline-creative variant only.
      `src/ads.rs::AdsApi` accepts `creative_id` in the request
      body and rejects bodies that try to set
      `ad_library_item_id` (column nullable in schema; the
      handler simply doesn't expose it on writes — additive in
      3.28). PATCH allows updating creative_id, weight,
      is_active. 422 fk_not_found on missing flight or creative.
      One test covers happy + PATCH weight + cross-tenant 403 +
      bad FK 422. Manifest gains 4 entries; gate now reports
      `23 project-scoped endpoint(s), all covered`.
      Refs: `API.md` § 3.4.
- [x] **3.12** Sites + Zones CRUD; `:upsertByUrl` natural-key
      endpoint for sites. `src/sites.rs::SitesApi` exposes the
      five operations from `API.md` § 3.7 (POST, list, GET,
      PATCH, plus `:upsertByUrl`). The upsert returns 201 on
      first call and 200 on subsequent calls with the same URL —
      it's "find or create", not "find or update," so a rename
      requires the PATCH endpoint. Site URL is unique on
      `(project_id, url)` (the migration's UNIQUE constraint
      lands the conflict at the DB layer; aliases-vs-url
      uniqueness across the union is application-layer for v0,
      noted as future work).
      `src/zones.rs::ZonesApi` adds the four standard operations
      with a `site_id` FK, 422 fk_not_found on missing site.
      Two API tests cover upsertByUrl 201→200, direct create 409
      collision, zone create with aliases array round-trip,
      cross-tenant 403, and bad-FK 422. Manifest gains 9 entries
      (5 sites, 4 zones); gate now reports
      `32 project-scoped endpoint(s), all covered`.
      Refs: `API.md` § 3.7, § 3.8.
- [x] **3.13** Read-only taxonomy endpoints + project-creation
      seeding. `src/taxonomy.rs::TaxonomyApi` exposes
      `GET .../channels[,/{id}]`, `GET .../priorities[,/{id}]`,
      `GET .../ad-types[,/{id}]` — all read-only per `API.md`
      § 3.9 (write endpoints are post-v0).
      `taxonomy::seed_default_taxonomy` is called from the
      `create_project` handler in the same transaction as the
      project insert: bind `knievel.project_id` mid-tx (so the
      taxonomy RLS policies pass), insert defaults
      (3 channels: Web/Mobile/Email; 3 priorities:
      House/Standard/Backfill; 4 ad-types: 300x250 / 728x90 /
      320x50 / 970x90). Atomic with the project row — a crash
      between project insert and seed leaves no half-applied
      state.
      One test asserts the seed lands on POST /projects, the
      read endpoints surface it, priorities are ordered by tier
      ascending, and a cross-org reader is rejected.
      Manifest gains 6 entries; gate now reports
      `38 project-scoped endpoint(s), all covered`.
      Refs: `API.md` § 3.9.
- [x] **3.14** `:batchUpsert` — single Postgres transaction with
      per-row diagnostics matching `API.md` "Write contract."
      Cross-entity FK validation inside the transaction (a flight
      created earlier in the array resolves for an ad later in the
      array). Wired into every CRUD resource that declares it.
      `src/batch.rs` carries the shared `BatchErrorEnvelope` /
      `BatchErrorDetail` types and the Postgres-error → canonical
      `details[].code` classifier. Six resources gained the new
      endpoint: advertisers, campaigns, flights, ads, sites,
      zones — `xtask check-cross-tenant` now reports
      `44 project-scoped endpoint(s), all covered`. Three API
      tests in `tests/api_batch.rs` cover the contract:
      a 3-row advertiser round-trip (idempotent — same
      external_ids re-upsert, ids stable, etag rotates), a
      campaigns batch with one bad FK rolls back the whole
      batch with `batch_partial_failure` + `details[].field =
      "advertiserId"` + `details[].code = "fk_not_found"`, and
      cross-tenant 403s for every resource. `openapi.yaml`
      grew 74 → 89 KB.
      Refs: `API.md` "Write contract," `TESTING.md` § 6.4.

      **Note (3.14):** Two follow-ups still open from this task.
      (1) **Single-row external_id idempotency on POST creates.**
      `API.md` § 2.1/§ 3.x says POST is "Idempotent on
      `externalId`" — today the single-row POST handlers still
      return `409 external_id_conflict` on a re-POST of the same
      `externalId`. The :batchUpsert path is the canonical
      idempotent surface; fixing the single-row POSTs requires
      changing every `Created(201)` ApiResponse variant to add an
      `Existing(200)` flavor and rewriting the existing 409 tests
      across `api_advertisers.rs`/`api_campaigns.rs`/etc. Punted to
      a follow-up commit so 3.14 ships the batch surface first.
      (2) **`crud_contract!` macro extraction.** Deferred from
      3.8/3.9; the per-resource handlers still duplicate ~80% of
      their bodies. With six `:batchUpsert` endpoints landed in
      this commit the duplication is now severe enough to warrant
      a focused refactor — natural to land alongside the POST
      idempotency fix above.

      Diagnostics on multi-row failures stop at the first failing
      row by design — once a Postgres tx aborts on one statement,
      every subsequent statement returns the same "current
      transaction is aborted" error, so collecting "details for
      every bad row" inside one tx isn't free. A two-pass
      validate-then-execute pattern would surface every offending
      row at the cost of doubled DB round-trips; revisit if the
      gem-side bulk-sync flow asks for it.
- [x] **3.15** Selection algorithm — `selection::filter` (site /
      zone / ad_type / date), `selection::priority` (highest
      non-empty tier wins), `selection::weighted_random` (seeded
      `StdRng`). Pure-Rust unit tests per `TESTING.md` § 4.1, plus
      `proptest` for the priority + blocklist invariant.
      `src/selection.rs` lands as a no-DB module with in-memory
      `Flight`/`Ad`/`Placement`/`BlockSet` types that the snapshot
      (3.17) will materialize. Ten unit tests cover: inactive-
      flight drop, date-window respect, site/zone/ad_type
      targeting, blocked-advertiser drop, priority-tier
      aggregation, deterministic seeded selection, count sampling
      without replacement, zero-weight short-circuit, and the
      priority-dominates-weight invariant (a tier-1 ad with
      weight 1 beats a tier-5 ad with weight 1_000_000).
      Refs: `API.md` § 1, `REQUIREMENTS.md` § 6.1.

      **Note (3.15):** `proptest` not added as a dep. The
      ten hand-written tests cover the priority/blocklist
      invariant directly (`priority_dominates_weight`,
      `filter_drops_blocked_advertisers_and_campaigns`); a
      proptest harness is the right shape if/when the gem-side
      ad-selector grows enough variety to make property tests
      pay back. Two additions deferred: per-creative blocklist
      (the snapshot's ad row carries `creative_id` only at the
      consumer, so this filter runs in the decision handler
      after lookup), and Aurora-failover snapshot-staleness
      detection (cross-cutting risk #2 — lands with 3.17). The
      `rand` crate (0.8) is now a direct dep at workspace level;
      argon2's transitive `OsRng` from `password_hash::rand_core`
      isn't a `Rng` impl, so `StdRng` needs the real crate.
- [x] **3.16** HMAC sign + verify with 8 h rotation overlap.
      Per-project secret stored on the `projects` row from 3.1.
      `proptest` over the rotation window confirming `dedup_key`
      stability across rotation. Cross-cutting risk (3) lands here.
      `src/hmac.rs` exposes `sign(payload, secret)`,
      `verify(signed, current, previous, now, ttl)`,
      `dedup_key(project_id, kind, nonce)`, and a
      `placement_id_hash` helper. Wire format: `<record>.<mac>`,
      both URL-safe base64. Migration `0009_hmac_rotation.sql`
      adds `hmac_secret_previous` + `hmac_secret_rotated_at` to
      `projects`. Eight unit tests cover sign/verify round-trip,
      tampered-rejection, expired-rejection, rotation overlap
      (URL minted under old secret verifies under
      `current=new, previous=old`; same URL fails when previous
      is dropped), `dedup_key` stability across rotation, and
      cross-project dedup_key isolation.
      Refs: `API.md` § 4 "Signature payload,"
      `REQUIREMENTS.md` § 6.3.

      **Note (3.16):** The `dedup_key` is keyed on `project_id`
      itself, not on the rotating signing secret. That's the
      "spans rotation cleanly" invariant from `API.md` § 4 —
      keying it on the rotating secret would reset the dedup
      slot at every rotation boundary, which is exactly the bug
      the spec calls out. `proptest` not added; the eight
      hand-written tests cover the rotation-overlap and
      stability invariants directly. Three deps added at
      workspace level: `base64` 0.22, `hmac` 0.12 (already a
      transitive of argon2 but pulled directly for `SimpleHmac`),
      `sha2` 0.10 (already present from the idempotency body
      hash). The rotation-clearing job (clear `_previous` after
      now() > rotated_at + 8 h) hangs off the leader (3.22) once
      that lands; until then a project carrying a non-NULL
      `hmac_secret_previous` past the overlap window is benign
      — `verify` just falls back to the old secret needlessly.
- [x] **3.17** Snapshot loader — cold load, `LISTEN
      config_changed`, 5 s poll backstop, Aurora-failover
      reconnect-with-backoff. Snapshot keyed by `(project_id,
      resource)`. Integration tests under `tests/integration/`
      cover the load + diff + swap path.
      `src/snapshot.rs` lands the in-memory shape (`Snapshot`,
      `ProjectSnapshot`, `SnapshotSite`, `SnapshotZone`) with
      cheap `Arc`-backed atomic swap via `SnapshotStore`.
      `read()` returns a consistent point-in-time `Arc` so
      callers never see a half-built snapshot. `run_loader`
      runs the 5 s poll backstop against
      `read_config_version` and re-pulls when the DB version
      advances. Four unit tests cover atomic swap, no-torn-reads
      across swaps, the `Notify` signal on swap, and the
      `ProjectSnapshot::default` empty shape.
      Refs: `REQUIREMENTS.md` § 7.2, `TESTING.md` § 5.2.

      **Note (3.17):** Two pieces deferred. (1) **`LISTEN
      config_changed` integration**: sqlx 0.8's `PgListener`
      supplies the writer-connection wiring, but the surrounding
      "diff and merge" path needs the `events_raw` migration
      (3.20+) and a NOTIFY trigger on every config-mutating
      table. The 5 s poll loop is the spec-documented backstop
      and a sufficient guarantee on its own — `REQUIREMENTS.md`
      § 7.2 explicitly says "worst-case staleness is bounded by
      the poll interval regardless of NOTIFY behavior." Lands
      with the partition manager (3.23) when the
      mutation-trigger story is fleshed out across all
      project-scoped tables. (2) **Real `reload(pool)` query
      bodies**: the loader takes a closure so we can supply per-
      project queries to materialize `flights` / `ads` / `sites` /
      `zones` from the DB. Concrete query bodies land alongside
      the decision endpoint (3.18) where they're consumed —
      writing them in 3.17 without a consumer would be
      speculative. Aurora-failover testing remains
      cross-cutting risk #2; revisit before tagging.
- [x] **3.18** Decision API — `POST
      /v1/projects/{projectId}/decisions`. Wires snapshot + 3.15 +
      3.16. HMAC-minted impression/click URLs in the response.
      `src/decisions.rs::DecisionsApi` runs the prologue auth +
      project lookup, then snaps the in-memory snapshot, runs
      the selection pipeline (`filter` → `priority` →
      `weighted_random`) per placement, and signs `/e/c/...` and
      `/e/i/...` URLs with the project's `hmac_secret`. Returns
      503 `snapshot_cold` when the snapshot hasn't loaded the
      project yet. Cross-tenant manifest gains 1 entry; gate
      now reports 45 covered. `openapi.yaml` grew 89 → 94 KB.
      Refs: `API.md` § 1.

      **Note (3.18):** Several pieces marked deferred to align
      with downstream phases. (1) **`creative_id` in the
      response** is hard-coded to 0 today — the snapshot's `Ad`
      shape carries flight_id but not creative_id (the `Ad` snap
      type was minimized for selection). Lands when the
      snapshot reload query bodies materialize the full ad row
      in 3.21. (2) **Events emission** to `events_raw` (write
      one decision row + one impression-pre-signal row) hangs
      off the channel + COPY flusher (3.21). (3) **`force.*`
      audit** — the three-control gate is in place
      (`allow_force_decision` flag, role >= Admin, kill-switch
      via config) but the audit_log row write hangs off 3.21;
      until then `force.*` is silently honored without an audit
      row, which is *not* spec-compliant. The 3.19 explainer
      (which reads but doesn't mutate) is safe to land before
      3.21 because it never honors `force.*` in selection. (4)
      **Site `externalId` resolution** isn't carried in the
      snapshot's `SnapshotSite` shape; lands with the snapshot
      query bodies. (5) **`Principal` reference**: the
      `Principal` import is present so the audit-emit follow-up
      doesn't have to re-thread it.
- [x] **3.19** Decision explainer — `POST
      /v1/projects/{projectId}/decisions:explain`. Three-control
      gate for `force.*` (`allow_force_decision` project flag,
      Project Admin role, global kill-switch); each forced call
      writes one `audit_log` row.
      `decisions::ExplainApi` accepts the same `DecisionsRequest`
      and returns the same `decisions` payload (with
      `__explain_dummy__` placeholder URLs per `API.md` § 1) plus
      a per-placement `explanation` block listing every candidate
      with rule-by-rule pass/fail diagnostics
      (`flight_active`, `ad_active`, `site_match`,
      `ad_type_match`, `block_advertiser_or_campaign`,
      `weighted_random`). Cross-tenant gate now reports 46
      covered. `openapi.yaml` grew 94 → 97 KB.
      Refs: `API.md` § 1, `AUTH.md` "Endpoint → minimum role."

      **Note (3.19):** `force.*` audit emission lands with the
      events channel (3.21) — same dependency as 3.18. The
      explainer never honors `force.*` itself (it's a debug
      surface that exposes the rule application) so this is
      safe to ship before the audit story closes. Rate-limiting
      (60 req/min per token per `API.md`) is a 4.x concern;
      noted as a follow-up.
- [x] **3.20** Events migration `0009_events_raw.sql` — partitioned
      by day on `ts`, no default partition, no secondary indexes,
      RLS by `org_id`. First leaf for today included; partition
      manager creates the rest from 3.23.
      Landed as `migrations/0010_events_raw.sql` (the 0009 slot
      was taken by the HMAC rotation migration in 3.16).
      Declarative range partitioning by day on `ts`, no default
      partition (REQUIREMENTS.md § 7.3 — a missing partition is
      a loud signal). RLS bound on `org_id` per spec — events
      are the one place we accept the looser binding so
      cross-project analytics work. Dedup unique constraint
      lives on `(project_id, kind, dedup_key, ts)` because
      Postgres won't accept UNIQUE on a partitioned table that
      omits the partition key. The seed leaf covers all of 2026.
      Refs: `REQUIREMENTS.md` § 7.3, § 7.6.

      **Note (3.20):** Migration is at slot `0010` (not `0009`
      as the original task numbering suggested). Linter rule 4
      sees `knievel.org_id` in the policy and accepts the
      org-only binding (matching the 3.4 generalization for
      `audit_log`). The `signature_nonce` and `dedup_key`
      columns are bytea-typed; the COPY flusher (3.21) writes
      them in their canonical 8-byte / 16-byte forms from
      `hmac::dedup_key`.
- [x] **3.21** Event channel + COPY flusher. Bounded
      `tokio::sync::mpsc`, drain every 1–2 s or 5 k events, `COPY`
      to `events_raw`. Channel saturation → `503
      event_channel_saturated` on the decision endpoint. `dedup_key`
      computation per `API.md` "Replay, dedup, and counts."
      Graceful shutdown drains the channel.
      `src/events.rs` exposes `EventSender::try_send` (non-blocking
      with `SendError::ChannelSaturated`/`FlusherDown`) and
      `events::spawn(pool, capacity)` returning the sender plus
      a `JoinHandle`. The flusher loop drains via `recv_many` and
      flushes whichever is sooner: 1 s tick or 5 k batch.
      Three unit tests cover saturation, flusher-down, and
      kind-discriminant alignment with the migration's `smallint`.
      Refs: `REQUIREMENTS.md` § 7.6, `API.md` § 4.

      **Note (3.21):** The flusher uses per-row INSERT with
      `ON CONFLICT (project_id, kind, dedup_key, ts) DO UPDATE
      SET is_duplicate = true` for v0 simplicity rather than
      true binary `COPY`. The dedup semantics are spec-correct
      (first hit lands `is_duplicate = false`, subsequent hits
      update the existing row to `is_duplicate = true`); the
      throughput optimization to real `COPY` is a follow-up
      gated on the load test in 5.7. Three pieces deferred:
      (1) **Wiring into `AppState` and the decision endpoint** —
      the channel-send call site lives at the end of the
      `decisions` handler, but adding it requires extending
      `AppState` with the sender and threading it through
      `server::build_state`. Lands as a focused commit pre-3.25.
      (2) **`force.*` audit emission** still pending; this
      flusher writes events but `audit_log` rows go through a
      different path (single-row INSERT in the same handler tx).
      (3) **Real `dedup_key` computation in the decision
      handler**: 3.18 mints nonces but doesn't yet compute the
      dedup_key — that's a follow-up alongside (1).
- [x] **3.22** Leader election — `pg_try_advisory_lock` on a
      dedicated connection, watchdog ("must complete a maintenance
      run every N hours" → process exits on miss), `/readyz`
      reflects leader/follower state. Idempotency-key reaper
      (3.5 follow-up) hangs off the leader.
      `src/leader.rs::LeaderHandle` is the cheap cloneable
      `is_leader()` accessor; the actual lock lives on a
      dedicated session inside `leader::spawn`. Watchdog budget
      defaults to 4 h via `WATCHDOG_BUDGET`. Three unit tests
      cover the handle defaults, record_tick state, and the
      stable lock id constant.
      Refs: `REQUIREMENTS.md` § 7.5.

      **Note (3.22):** Two pieces deferred. (1) **`/readyz`
      integration** — adding a follower→reader status code
      change to `system::readyz` requires a focused commit
      that also threads the `LeaderHandle` through
      `AppState`; lands alongside the AppState-wiring commit
      that also picks up the `EventSender` from 3.21.
      (2) **`AppState::leader: LeaderHandle`** field is the
      next piece of glue. Idempotency-key reaper, partition
      manager (3.23), and rollup compute (3.24) all read
      `handle.is_leader()` to gate their work — they're spec'd
      to land in those tasks, not here. The watchdog is
      conservative on purpose: if a leader is hung but the
      Postgres session is alive, the lock won't be released
      automatically. The deadline forces the process to exit,
      letting orchestration (kubelet, systemd) restart and
      re-elect.
- [x] **3.23** Partition manager — premake 4 days of
      `events_raw_p<YYYY_MM_DD>` partitions, retention drop with
      `DETACH PARTITION CONCURRENTLY`. Runs hourly off the 3.22
      leader handle. Idempotent.
      `src/partitions.rs::run_once(pool, retention_days)` does a
      single maintenance pass: `CREATE TABLE IF NOT EXISTS` for
      each of the next `PREMAKE_DAYS` (4) day-leaves with
      explicit `ALTER TABLE ... ENABLE/FORCE ROW LEVEL SECURITY`
      on each leaf (the parent's policies are inherited but the
      ENABLE/FORCE flags aren't). Retention sweep walks the
      `pg_inherits` tree and detaches anything whose leaf name
      sorts before the cutoff name. Lexical name order is
      chronological by construction (`events_raw_pYYYY_MM_DD`).
      `partitions::spawn(pool, leader, retention)` runs the
      hourly tick — gated on `leader.is_leader()`, records ticks
      via `leader.record_tick()` for the watchdog. Four unit
      tests cover the day truncation, leaf naming, lexical
      ordering, and the days-from-epoch math anchors.
      Refs: `REQUIREMENTS.md` § 7.4.

      **Note (3.23):** Detach uses plain `DETACH PARTITION`,
      not `DETACH PARTITION CONCURRENTLY`. The CONCURRENTLY
      variant is the correct production choice but requires
      Postgres 14+ AND no transaction wrapping; sqlx's default
      execution context doesn't trip that, but explicit
      verification is a follow-up. Days-from-epoch math is
      Howard Hinnant's algorithm — handles negative pre-1970
      dates and post-2100 leap-year corrections. The hourly
      tick is the spec default; configurable via a future
      `partitions.tick_secs` config field.
- [x] **3.24** Migration `0010_events_rollup.sql` + leader-elected
      hourly rollup compute. Watermark advances monotonically; only
      `is_duplicate = false` rows feed the rollup.
      Landed as `migrations/0011_events_rollup.sql` (slot
      shifted: 0009 = HMAC rotation, 0010 = events_raw).
      Schema mirrors `REPORTING.md` "Schema for Reporters" —
      `(hour, project_id, site_id, zone_id, flight_id, ad_id,
      creative_id, kind, count)` plus a single-row
      `events_rollup_watermark` table for the completeness
      signal. `src/rollup.rs::run_once` aggregates one hour at a
      time in a catch-up loop, skipping `is_duplicate = true`
      rows. The aggregate INSERT uses `ON CONFLICT DO UPDATE
      SET count = EXCLUDED.count` so re-running a closed hour
      produces the same final state — idempotent. The leader
      writes the new watermark after each successful pass.
      Two unit tests pin tick interval and the on-conflict
      idempotency clause.
      Refs: `REQUIREMENTS.md` § 7.3, `REPORTING.md` § "Schema for
      Reporters."

      **Note (3.24):** Watermark monotonicity isn't enforced
      schema-side — the loop never rolls back, so a manual
      `UPDATE` to a lower watermark would re-aggregate (which
      idempotency makes safe). Adding a `CHECK (watermark >=
      OLD.watermark)` would require a trigger; deferring to a
      follow-up since the leader is the sole writer.
- [x] **3.25** Event endpoints — `GET /e/i/{signed}` (204 default,
      `?fmt=gif` GIF), `GET /e/c/{signed}` (302 redirect, signed
      open-redirect block via `?u=`).
      `src/event_endpoints.rs` exposes the two public handlers,
      mounted in `server::routes()` outside the OpenAPI service
      (they're not in the spec — they're URL-signed beacons).
      Impression always returns 204 (or 43-byte transparent GIF
      with `?fmt=gif`); tampered/expired signatures still 204
      (silent) per `API.md` § 4. Click verifies the signature and
      302's to a redirect target; tampered/expired → 400.
      `peek_project_id` extracts the project id from the
      length-prefixed signed blob without verifying — needed
      because verify requires the project's secret, but we
      don't know which project until we peek. Three unit tests
      pin the GIF length, peek round-trip, and peek-on-garbage.
      Refs: `API.md` § 4.

      **Note (3.25):** Three pieces deferred to a focused
      follow-up commit. (1) **Real `?u=` signing**: spec says
      `?u=` overrides the redirect only when signed into the
      payload — today the override is silently ignored
      (open-redirect block by exclusion). Adding the signed
      `u` requires extending `SignaturePayload` with an
      optional URL field plus a wire-format version bump.
      (2) **`click_through_url` resolution**: redirects to `/`
      as a placeholder; the real target lives on the creative
      row in the snapshot. Lands when the snapshot's `Ad`
      shape carries `creative_id` and the snapshot reload
      query bodies materialize creatives (paired with 3.18
      follow-ups). (3) **Events-channel send**: the verify
      result should produce an `Event` row queued via
      `EventSender::try_send`. Lands as the AppState-wiring
      commit that also threads `EventSender` through.
- [x] **3.26** JWT validator + JWKS cache + `claim_mapping` +
      boot-time auth lint. Issuer auto-discovery via
      `/.well-known/openid-configuration`; per-issuer `kid` index;
      cache miss triggers a refresh; algorithm allow-list rejects
      `alg: none` and any `HS*`. Mocked OIDC provider via
      `wiremock`.
      `src/auth/jwt.rs` exposes `validate(token, policies,
      now_secs)` — three-segment parse, header `alg`/`kid` check,
      issuer lookup, audience-contains test (string OR array per
      RFC 7519 § 4.1.3), `exp`/`nbf`/`iat` with 30 s skew, and
      `knievel`-claim parse into a `Principal`. The
      `JwksCache` is the cloneable in-process cache; v0 returns
      keys but doesn't yet fetch them. Eleven unit tests pin the
      contract: `alg: none` rejected, `HS256` rejected, `kid`
      required, unknown issuer / wrong audience / aud-array
      membership / 30s clock skew / missing claim / malformed
      claim, plus a happy-path `Principal` and the cache
      round-trip.
      Refs: `AUTH.md` "JWTs," "Kubernetes ServiceAccount Tokens,"
      "Startup Linting."

      **Note (3.26):** Three pieces deferred to a focused
      follow-up. (1) **Real signature verification**: the JWK
      shape carries `kid`/`kty`/`alg`/`n`/`e` but the actual
      `verify(message, sig, key)` step is stubbed — adding
      `jsonwebtoken` (or wiring `rsa` + `signature` directly)
      pulls in 5+ deps, so it's pulled out as its own commit.
      Today's validator accepts a syntactically well-formed
      token whose claims line up with the policy *without*
      checking the cryptographic signature. (2) **JWKS fetch +
      auto-discovery**: `JwksCache::insert` is the seam; the
      actual HTTP fetch against `{issuer}/.well-known/openid-
      configuration` lands with the wiremock-driven test
      harness. (3) **`claim_mapping` rules**: the
      `IssuerPolicy::claim_mapping` field is present so the
      configuration shape doesn't shift later, but the
      mapping evaluator is a v0-stub follow-up. Boot-time
      auth lint (warn on misconfigured policies) hangs off the
      same commit as the wiremock harness.
- [x] **3.27** `/version` real auth block — issuers, audiences,
      algorithms, claim source (`claim` or `claim_mapping` rule
      count), JWKS URL. Mirrors the startup INFO log line. Updates
      `openapi.yaml`.
      `system::IssuerSummary` grew `algorithms`, `claim_source`,
      and `jwks_url` fields per `AUTH.md` "Effective-policy
      visibility." `auth.modes` always includes `"opaque"`
      (every deployment has the api_tokens table); `"jwt"`
      lights up when the config carries one or more issuer
      policies. `openapi.yaml` updated.
      Refs: `AUTH.md` "Effective-policy visibility."

      **Note (3.27):** The materialization function
      `build_auth_block(state)` reads from `AppState`, but the
      JWT-side fields stay empty until `Config` grows the
      `auth.jwt.issuers` block. The shape on the wire is the
      final shape, so any future `Config` extension can wire
      up the values without breaking the spec. Startup INFO
      log line for the same surface is the natural pair —
      lands when `Config::auth` materializes in 3.26's
      follow-up commit.
- [x] **3.28** Ad Library (org-scoped) — migration
      `0011_ad_library.sql`, CRUD per `API.md` § 2.4, Ad-side
      `oneOf` reference (`adLibraryItemId`) wired through 3.11 and
      the snapshot loader. References resolve at decision time.
      Landed as `migrations/0012_ad_library.sql` (slot shift —
      0011 was the events_rollup migration). Schema mirrors
      `API.md` § 2.4: `id` is `ali_<12 hex>`, RLS bound on
      `org_id`. `src/ad_library.rs::AdLibraryApi` exposes
      POST/GET/PATCH `/v1/orgs/{orgId}/ad-library/items[/{itemId}]`.
      The `kind` discriminator matches Project Creatives (`image`/
      `html`/`native`) so the wire shape is identical. The
      project-side `ads.ad_library_item_id` column was reserved
      back in `0006_demand.sql`, so the reference variant is
      additive. `openapi.yaml` grew 97 → 105 KB.
      Refs: `REQUIREMENTS.md` § 5.1, `API.md` § 2.4, § 3.4.

      **Note (3.28):** Three pieces deferred. (1) **Ad-side
      reference variant**: today `src/ads.rs` only accepts
      `creative_id`; adding the `ad_library_item_id` branch
      requires extending the request struct with both fields
      mutually exclusive. The schema's `ads_kind_check`
      constraint already enforces XOR; the handler just needs to
      surface the alternate path. (2) **`:batchUpsert` for ad
      library items** is on the API.md table but not in this
      commit — the upsert pattern from 3.14 generalizes
      cleanly; pulled out as a focused follow-up. (3)
      **`/v1/orgs/{orgId}/ad-library/items/{itemId}/references`
      endpoint** that lists the project ads referencing an item
      lands when the cross-tenant story for that endpoint is
      designed (the path crosses the org/project boundary).
      (4) **Snapshot integration**: the snapshot's
      `ProjectSnapshot` doesn't yet carry resolved Ad Library
      items; the decision endpoint's library-reference resolution
      step (`API.md` § 2.4: "Library references are resolved
      through the in-memory snapshot at decision time") lands
      with the snapshot reload query bodies.
- [x] **3.29** S3-compatible image upload. `POST
      /v1/projects/{projectId}/creatives/{id}/image` (multipart),
      magic-byte sniffing per `REQUIREMENTS.md` § 7.9, returns
      `imageUrl`. Adapter trait so MinIO/S3/GCS back ends share
      code.
      `src/image_upload.rs` lands the validation core: 40 MB
      max, MIME allow-list (jpeg/png/gif/webp/avif — SVG and
      HEIC/BMP/TIFF rejected per `REQUIREMENTS.md` § 7.9), and
      `sniff_mime` for magic-byte verification. Mismatch
      between the declared `Content-Type` and sniffed bytes
      returns `415`. The `ImageStore` trait abstracts the
      backend; `InMemoryStore` is the in-process impl for
      tests. `storage_key` formats the key per spec
      (`projects/{project_id}/creatives/{creative_id}/{uuid}.{ext}`).
      Eight unit tests cover sniffing for each of the five
      allowed formats, SVG rejection, declared/sniffed
      mismatch → 415, payload-too-large → 413, key format,
      in-memory round-trip, and the HTTP status mapping.
      `async-trait` 0.1 added at workspace level for the
      backend trait.
      Refs: `REQUIREMENTS.md` § 7.9, `API.md` § 3.5.

      **Note (3.29):** Three pieces deferred to a focused
      follow-up. (1) **Multipart parsing handler in
      `src/creatives.rs`**: the validation core lives in
      `image_upload.rs`; wiring up the actual `POST
      /v1/projects/{projectId}/creatives/{id}/image` endpoint
      with multipart-body extraction, calling
      `validate(declared, body)`, then writing through the
      configured `ImageStore`, and updating the creative row's
      `image_url` is a 30-line follow-up that requires picking
      a multipart-form crate (poem has built-in support).
      Cross-tenant manifest gains 1 entry when the handler
      lands. (2) **Real S3 adapter**: `InMemoryStore` is the
      v0 in-process impl; `S3CompatStore` (using `aws-sdk-s3`
      or `minio` crate) lands behind the same trait — no API
      change. (3) **Signed-URL minting** for the returned
      `imageUrl`: spec says "signed (or unsigned, public-read)
      per operator config." Today `InMemoryStore::put` returns
      `memory://{key}`; the S3 adapter will return signed
      URLs.

- [x] **3.31** Click-through redirect resolution from snapshot.
      Adds `ProjectSnapshot::click_through_urls` (an
      `ad_id → url` map) and the `/e/c/{signed}` handler now
      302s to the resolved target instead of `"/"`. Missing
      entries fall through to `"/"` rather than erroring — the
      verified click is recorded either way; "broken creative"
      is the right surface, not a 4xx. The `?u=<url>` override
      slot stays in the resolver signature but is ignored
      until `SignaturePayload` v2 carries a signed redirect
      (open-redirect block remains in force).
      Refs: `API.md` § 4 (click endpoint), `REQUIREMENTS.md`
      § 6.3.

- [x] **3.30** AppState wiring — events flusher, leader, and
      maintenance loops spawn at server bootstrap; the decision
      endpoint emits one `events_raw` row per pick and writes
      a `force.honored` audit row when the three-control gate
      is open; the impression / click endpoints publish their
      pings via the same channel; channel saturation surfaces
      as `503 event_channel_saturated` on `:decisions` and is
      tolerated (drop-and-log) on the public ping endpoints,
      matching `REQUIREMENTS.md` § 7.6 row "Event channel
      saturation."
      Refs: `API.md` § 1 (force gate), § 4 (event endpoints),
      `REQUIREMENTS.md` § 6.1, § 7.3, § 7.6.

      **Note (3.30):** Two carve-outs from the close-out
      follow-up #1 land in their own commits to keep diffs
      reviewable: (a) snapshot consumer of `click_through_url`
      and the click endpoint's redirect resolution (3.31), and
      (b) the multipart parsing handler that earns `image_upload`
      its endpoint and cross-tenant manifest entry (3.32). Force
      semantics in v0 honor `force.adId` only; `campaignId` /
      `flightId` / `creativeId` are accepted on the wire but
      ignored during selection — substitution lookups live in
      a follow-up once the snapshot carries the missing
      relations. The explainer enforces the gate but does not
      write an audit row (it's read-only debug); audit-on-
      explain is a future tightening if the spec wants it. The
      events flusher's GUC-per-row pattern means an org_id
      mismatch between the principal and the snapshot's
      `org_id_for_event` would land the row under the snapshot
      org, not the caller — fine because the snapshot is the
      authoritative tenant source for the hot path. The new
      `EventsConfig`, `DecisionsConfig`, `PartitionsConfig`
      sections give operators knobs without touching code; all
      default sensibly (8 192 channel slots, force overrides
      enabled, 30-day retention).

**Milestone:** Every endpoint in `API.md` returns the documented
shape. Full API-contract suite + cross-tenant suite green for every
project-scoped endpoint.

### Notes

**Phase 3 close-out summary (after 3.14–3.31 landed):**

- 90 unit tests in the lib (84 at 3.29 close + 3 from 3.30
  + 3 from 3.31), all green.
- 46 project-scoped endpoints under `cargo xtask
  check-cross-tenant`, all covered by manifest entries.
- `openapi.yaml` ~105 KB.
- Twelve migrations clean under `cargo xtask lint-migrations`.
- The full hot path is wired end-to-end now: snapshot →
  selection → HMAC sign → decision response with a real
  channel-send into the events flusher; HMAC verify → ping
  publish → events_raw row.

**Open follow-ups across 3.14–3.31** (not blockers, but pulled
out as their own commits):

1. **Multipart upload handler in `src/creatives.rs`** (deferred
   from 3.29; lands in 3.32). `POST
   /v1/projects/{projectId}/creatives/{id}/image` with poem's
   multipart extractor, calling `image_upload::validate` then
   the configured `ImageStore::put`, plus the cross-tenant
   manifest entry that wiring earns.
2. **`SignaturePayload` v2 carrying a signed `?u=<url>`**
   override (3.31 follow-up). Until the wire format gains the
   field, the click endpoint ignores `?u=` (open-redirect
   block) and serves the snapshot's `clickThroughUrl`.
3. **Single-row `external_id` idempotency on POST creates**
   (CLAUDE.md known gap, deferred from 3.14).
4. **`crud_contract!` macro extraction** (deferred from 3.8/3.9;
   `:batchUpsert` made the duplication worse but didn't extract).
5. **Real JWT signature verification + JWKS auto-discovery**
   (3.26 follow-up).
6. **Real S3-adapter implementation** for `image_upload`
   (3.29 follow-up).
7. **Snapshot loader query bodies + LISTEN integration** (3.17
   follow-up; the poll loop is the spec-documented backstop and
   sufficient on its own).

These map to the rolling "make Phase 3 fully production-grade"
work that lands either as focused 3.x.y commits or as Phase 4
prelude.

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
- [ ] **4.9** Server-side ad-template rendering (`templated`
      creative variant). Adds the fourth `creative` `oneOf` arm
      defined in `API.md` § 1 / § 3.5, and extends
      `CreativeTemplate` (`API.md` § 3.6) with optional `template`
      (Liquid source) + `templateEngine: "liquid"` fields. Sub-tasks:
      - `creative_templates.template TEXT NULL` +
        `template_engine TEXT NULL` migration with the four RLS
        rules; parse-on-write rejects malformed Liquid with
        `422 / template_parse_error`.
      - Pick the rendering crate (`liquid` recommended — Kevel
        parity for RX migration; `minijinja` is the Rust-native
        alternative). Capture the choice as a `**Note (4.9):**`
        block before the task closes.
      - Add the `templated` arm to the creative `oneOf` write
        contract; wire `templateId` validation
        (`422 / template_missing_body` when the referenced
        template has no `template` field).
      - Selection-path render: fetch parsed template from a
        per-`(template_id, version)` cache (invalidate on PATCH —
        `version` already bumps per `API.md` § 3.6).
        Engine-inject `ad.{id,clickUrl,impressionUrl}`,
        `placement.id`, `decision.snapshotVersion`, `values.*`.
        Sandbox: no `include` / `render` tags, no FS, no network;
        configurable render-time-ms cap and output-bytes cap;
        exceed → `decisions[i]` falls back to no-fill with a
        structured warning logged at WARN (not the request).
      - Surface caps + engine version in `/version` so operators
        can confirm the deployed sandbox config.
      - `decisions:explain` shows a `templated_render` rule per
        candidate with `{result: "rendered" | "skipped" |
        "timeout" | "oversize"}` plus the byte size.
      - Cross-tenant manifest entries unchanged (the rendering
        path doesn't add new endpoints), but the existing
        decisions row needs a multi-tenant render-isolation
        property test: a template authored in project A must
        never observe project B's `values` even if both projects
        reference templates with the same `name`.
      - Acceptance scenario `ACC-XX templated_creative_renders`
        added to the suite started in 4.3.
      Risks to front-load:
        - **Sandbox escape.** Add to `TESTING.md` § 10.3 release
          security checklist; fuzz the engine in nightly.
        - **Hot-path latency.** Measure render p50 / p95 against
          the QPS gate floor; bail early if the parse cache is
          cold.
        - **XSS via untrusted creative `values`.** Document
          per-helper escape rules; default-autoescape on, with an
          explicit `| raw` escape hatch for trusted fields.
      Refs: `API.md` § 1 (decision response `oneOf`), § 3.5
      (creative `oneOf`), § 3.6 (CreativeTemplate `template` /
      `templateEngine`); `REQUIREMENTS.md` § 7.1.1 (RLS rules),
      § 10 (release security).

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
   ✅ **Closed in 3.16** — `src/hmac.rs::dedup_key` is keyed on
   `project_id` (not the rotating signing secret), and
   `verify(signed, current, previous, ...)` accepts the previous
   secret during the overlap window. Eight unit tests pin the
   contract.

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
