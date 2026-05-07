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
- [x] **3.3** First handler — `POST /v1/orgs/{org_id}/projects`
      and `GET /v1/orgs/{org_id}/projects/{project_id}`. `OrgApi`
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
      /v1/orgs/{org_id}/tokens`. `src/tokens.rs::TokensApi`:
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
      `POST/GET/PATCH /v1/projects/{project_id}/advertisers[/{id}]`.
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

      **Note (3.14):** Three follow-ups deferred to **Phase 6**
      (post-v0 polish; v0 ships with batch as-is).
      (1) Single-row `external_id` idempotency on POST creates →
      Phase 6.1.
      (2) `crud_contract!` macro extraction →  Phase 6.2.
      (3) Two-pass validate-then-execute for batch errors —
      today diagnostics stop at the first failing row by design
      (Postgres aborts the tx on the first bad statement); a
      two-pass pattern would surface every offending row at the
      cost of doubled DB round-trips → Phase 6.3.
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
      /v1/projects/{project_id}/decisions`. Wires snapshot + 3.15 +
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
      /v1/projects/{project_id}/decisions:explain`. Three-control
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
      POST/GET/PATCH `/v1/orgs/{org_id}/ad-library/items[/{item_id}]`.
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
      commit — moved to **Phase 6.4** (post-v0; the upsert
      pattern from 3.14 generalizes cleanly). (3)
      **`/v1/orgs/{org_id}/ad-library/items/{item_id}/references`
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
      /v1/projects/{project_id}/creatives/{id}/image` (multipart),
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
      /v1/projects/{project_id}/creatives/{id}/image` endpoint
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

- [x] **3.32** Multipart upload handler in `src/creatives.rs`.
      `POST /v1/projects/{project_id}/creatives/{id}/image`
      accepts a multipart body via `poem-openapi`'s `Multipart`
      derive, runs the body through `image_upload::validate`,
      writes through the configured `ImageStore`, and updates
      the creative's `image_url` in the same transaction. Min
      role: editor (matches `createCreative`). Returns the
      updated creative on 200; 413 / 415 / 404 / 403 / 400 /
      500 envelopes per the response shape on
      `image_upload::UploadError`. Server bootstrap injects an
      `InMemoryStore` as the v0 default; the S3-compat adapter
      is the 3.29 follow-up. The `random_object_uuid` helper
      keeps us off `uuid` as a workspace dep — argon2's
      `password_hash::rand_core` already provides `OsRng`.
      Cross-tenant manifest grows by 1 entry; cross-tenant
      gate now reports 47 endpoints, all covered.
      Refs: `REQUIREMENTS.md` § 7.9, `API.md` § 3.5.

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

- [x] **3.33** Cursor pagination — server side. Every paginated
      list endpoint accepts `?limit=N&cursor=<opaque>` per
      `API.md` § "Pagination" (default 50, max 500). Cursor is
      `base64url(JSON{kind, last_id})`; server validates `kind`
      matches the endpoint to catch cross-resource cursor
      replay (`400 invalid_cursor`). `?limit=0` and
      `?limit > 500` return `400 invalid_limit`. Implementation
      lives in `src/pagination.rs` (~80 lines + 13 unit tests),
      wired into the 8 demand+inventory list handlers
      (`advertisers`, `campaigns`, `flights`, `ads`,
      `creatives`, `creative_templates`, `sites`, `zones`) via
      a uniform shape: `Query<Option<i64>>` + `Query<Option<String>>`
      params, `pagination::resolve` + `pagination::next_cursor`
      around an `id`-DESC keyset query that fetches `LIMIT N+1`
      to peek "are there more pages?" without a separate COUNT.
      `tests/api_pagination.rs` covers the contract:
      default-limit, cursor walk across multiple pages,
      last-page-null-cursor, `invalid_limit` (zero + overcap),
      `invalid_cursor` (corrupt + cross-resource replay).
      Refs: `API.md` § "Pagination," CLAUDE.md "Open known
      gaps."

      **Note (3.33):** Five list endpoints are intentionally
      not cursor-paginated for v0:
      `listChannels`/`listPriorities`/`listAdTypes` (taxonomy
      tables are bounded-small per project, ~5 rows each, and
      `listPriorities` is sorted by `tier` semantically rather
      than by `id`); `listAdLibraryItems` and `listTokens`
      have TEXT primary keys, so they need a
      `(created_at, id)` tuple cursor instead of the bigserial-
      `id` shape this commit ships — moved to **Phase 6.5**.
      All five still return `nextCursor: null` so wrappers
      degenerate to a single page cleanly. The vendor
      extensions (`x-knievel-paginated*`) API.md once promised
      are deferred to **Phase 6.6** — poem-openapi 5 doesn't
      expose generic operation-level extensions, and rather
      than carry a `cargo xtask openapi` post-processor as a
      maintenance liability we'll upstream extension support
      to poem-openapi first. The hand-written Ruby wrapper
      from 4.10 doesn't need extensions to know its paginated
      set; they earn their keep when a second consumer (Python
      / Go binding, doc-site renderer) shows up.

**Milestone:** Every endpoint in `API.md` returns the documented
shape. Full API-contract suite + cross-tenant suite green for every
project-scoped endpoint.

### Notes

**Phase 3 close-out summary (after 3.14–3.32 landed):**

- 90 unit tests in the lib (84 at 3.29 close + 3 from 3.30
  + 3 from 3.31), all green.
- 47 project-scoped endpoints under `cargo xtask
  check-cross-tenant`, all covered by manifest entries
  (3.32 added image upload).
- `openapi.yaml` ~107 KB.
- Twelve migrations clean under `cargo xtask lint-migrations`.
- The full hot path is wired end-to-end now: snapshot →
  selection → HMAC sign → decision response with a real
  channel-send into the events flusher; HMAC verify → ping
  publish → events_raw row.

**Open follow-ups across 3.14–3.32** (not blockers, but pulled
out as their own commits):

1. **`SignaturePayload` v2 carrying a signed `?u=<url>`**
   override (3.31 follow-up). Until the wire format gains the
   field, the click endpoint ignores `?u=` (open-redirect
   block) and serves the snapshot's `clickThroughUrl`.
2. ~~Single-row `external_id` idempotency on POST creates~~ →
   moved to **Phase 6.1**.
3. ~~`crud_contract!` macro extraction~~ → moved to
   **Phase 6.2**.
4. **Real JWT signature verification + JWKS auto-discovery**
   (3.26 follow-up).
5. **Real S3-adapter implementation** for `image_upload`
   (3.29 / 3.32 follow-up; the v0 in-process `InMemoryStore`
   is fine for tests but won't serve uploads beyond a single
   instance's lifetime).
6. **Snapshot loader query bodies + LISTEN integration** (3.17
   follow-up; the poll loop is the spec-documented backstop and
   sufficient on its own).

These map to the rolling "make Phase 3 fully production-grade"
work that lands either as focused 3.x.y commits or as Phase 4
prelude.

---

## Phase 4 — Make it deployable

**Goal:** Anyone can `docker compose up` and get a working knievel
or `helm install` it into a real cluster. Container image
published to `ghcr.io/xrl/knievel`, multi-arch, signed.
Acceptance suite running in CI. Generated client libraries
follow once the runtime substrate is real.

**Spec references:**

- `REQUIREMENTS.md` § 8 (Deliverables), § 8.1 (Helm chart).
- `MIGRATION_RX.md` "Local Development for RX Engineers"
  (compose layout pinned to `ghcr.io/xrl/knievel:latest`).
- `TESTING.md` § 7 (E2E Acceptance), § 11.1 (`seed-demo`).
- `DOCUMENTATION_PLAN.md` § 6 (DEPLOYMENT.md).

**Order of operations.** The substrate (compose, container,
Helm) lands first so every later task — acceptance suites,
chaos rigs, even the Ruby gem's smoke test — has a real
deployable to point at. Ruby-gem generation moves to the end
because the OpenAPI spec is the contract; everything else
flows from a working binary in a real container.

**Tasks (broad strokes):**

- [x] **4.1** `examples/compose/` reference stack — `docker
      compose up` boots Postgres + knievel against a locally-
      built image. Pinned to `ghcr.io/xrl/knievel:latest` once
      4.3 publishes; until then the compose file uses a `build:`
      directive against the in-tree `Dockerfile`. `knievel-cli
      seed-demo` runs as a one-shot sidecar (stubbed until 4.2
      lands).
      Refs: `MIGRATION_RX.md` "Local Development for RX
      Engineers," `TESTING.md` § 11.1.

      **Note (4.1):** Three pieces:
      - **Dockerfile** at the repo root — multi-stage build
        with a dependency-prefetch layer (stub `src/main.rs`
        + `cargo build` so the registry is warm before the
        real source comes in), then a release build that
        `strip`s the binary. Final image is
        `gcr.io/distroless/cc-debian12:nonroot`; `cc`
        because rustls-tls links against `libgcc_s`, `nonroot`
        for UID 65532 with no shell. The cache layer means
        an iterative source change rebuilds only the binary,
        not 300+ deps.
      - **Compose stack** under `examples/compose/`. Service
        names + volume names mirror the `MIGRATION_RX.md`
        canonical example so a contributor reading either
        file recognizes the structure. The `KNIEVEL_IMAGE`
        env var lets you point at a pinned digest
        (`@sha256:...`) without editing `compose.yaml`.
        `knievel-seed` is a `curlimages/curl` placeholder
        that polls `/readyz` and exits 0 — Phase 4.2 swaps
        it for the real `seed-demo` invocation.
      - **`auto_migrate` wired** in `server::build_state`.
        New `src/migrate.rs` carries
        `static MIGRATOR = sqlx::migrate!("./migrations")`
        plus a `run` helper that does the
        `CREATE SCHEMA / pgcrypto / migrate` sequence
        idempotently. The pool's `after_connect` hook sets
        `search_path = knievel, public` (mirroring the
        `testlib` pattern) so the `_sqlx_migrations` table
        lands in `knievel`, not `public`. `sqlx`'s `migrate`
        feature is now on at the workspace level.

      Sandbox note: docker isn't reachable in the dev
      sandbox, so end-to-end verification (`docker compose
      up` against the image) ran as a release-mode `cargo
      check` only. Phase 4.3's CI workflow will exercise the
      full container build path on every PR.
- [x] **4.2** `knievel-cli seed-demo` implementation. Admin
      CLI binary alongside the server binary; `seed-demo`
      populates org/project/advertisers/campaigns/flights/ads/
      creatives/sites/zones via direct DB access (auth
      chicken-and-egg dictates DB-direct, not HTTP) so a
      contributor can issue meaningful decisions against the
      compose stack. Idempotent — re-runs find every row by
      `external_id` and a deterministic hash-derived id for the
      org / project. Bearer is written to a file on a configured
      path so the compose `knievel-seed` sidecar can drop it on
      a host volume.
      Refs: `REQUIREMENTS.md` § 8 item 4, `AUTH.md` "Local
      Development."
- [x] **4.3** Multi-arch container image build, **published
      to `ghcr.io/xrl/knievel`** on semver tags only — every
      published image is a deliberate release, not a moving
      target. `docker buildx` for `linux/amd64` +
      `linux/arm64`, distroless base
      (`gcr.io/distroless/cc:nonroot`). Tag policy:
      `ghcr.io/xrl/knievel:vX.Y.Z` (semver, immutable) plus
      `ghcr.io/xrl/knievel:latest` re-pointed to the freshest
      semver release. **No image is built or pushed for
      main-branch commits or PRs** — pre-release deployments
      pin to a digest from a published `vX.Y.Z-rc.N` tag, or
      build locally via the in-tree `Dockerfile` (the compose
      stack from 4.1 supports `KNIEVEL_BUILD=1`). `cosign`
      keyless signing via Sigstore Fulcio; provenance
      attestation included. Build runs in
      `.github/workflows/release.yml` under `on: push: tags:
      ['v*']`. The compose file in 4.1 and the Helm chart in
      4.4 reference this image directly.

      **`knievel-cli` release attachments.** On every `v*`
      tag, cross-build the `knievel-cli` binary for
      `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
      `x86_64-apple-darwin`, and `aarch64-apple-darwin` (cargo
      cross / GitHub-hosted runners). Strip + tar.gz +
      sha256sum each artifact (`knievel-cli-vX.Y.Z-<target>.tar.gz`
      + matching `.sha256`). Sign each artifact's digest with
      `cosign sign-blob` keyless. Upload all artifacts to the
      GitHub Release via `softprops/action-gh-release` (or
      `gh release upload`); a `checksums.txt` aggregates the
      per-artifact sha256 for one-line verification. The
      release-notes block links the image digest, the cosign
      certificate, and the per-OS install one-liner. This
      makes `knievel-cli` installable without docker —
      operators on macOS / Linux can `curl | tar` the matching
      binary, and the SDK / scripting consumers Phase 4.2
      enables (`seed-demo`) work on bare hosts.

      Refs: `REQUIREMENTS.md` § 8 item 5, `MIGRATION_RX.md`
      compose example, `TESTING.md` § 12.9, § 10.3 (release
      security checklist — cosign attestation lives here).
- [x] **4.4** `charts/knievel` Helm chart; `helm lint` +
      `kubeconform` gate. Default `values.yaml` pins
      `image.repository: ghcr.io/xrl/knievel` and `image.tag:
      ""` (defaults to `Chart.AppVersion`; operator overrides
      per environment, including digest pinning via a `tag`
      starting with `sha256:`).
      Refs: `REQUIREMENTS.md` § 8.1.
- [x] **4.5** Acceptance scenarios ACC-01..30. The test binary
      `tests/acceptance.rs` lands all 30 scenarios as named
      `#[tokio::test]` functions (`acc_01_*` through
      `acc_30_*`), so a reader can grep TESTING.md and find the
      runnable test instantly. Six scenarios exercising
      flows that work end-to-end through the in-process
      `poem::test::TestClient` harness are full tests today;
      the remaining 24 are `#[ignore]`'d skeletons whose
      `#[ignore]` reason names the dependency that unblocks
      them (ad-library decision-time wiring, snapshot loader
      direct-invocation, MinIO container, JWKS mock, chaos rig,
      etc.). The skeletons stay so (1) Phase 4.6's shard matrix
      sees a stable test count, (2) the grep-by-name from
      TESTING.md works on day 1, and (3) activating each one is
      a focused PR — flip `#[ignore]`, fill in the body.
      Refs: `TESTING.md` § 7.1.
- [x] **4.6** Acceptance sharding in CI (4-way nextest
      partition). Each shard runs in its own job with its own
      Postgres service container; shard $N$ runs `nextest run
      -E 'kind(test) & binary(acceptance)' --partition
      count:$N$/4`. Spec calls for docker-compose orchestration
      per shard (TESTING.md § 12.6); we run in-process via
      `poem::test::TestClient` + ephemeral Postgres so the
      compose dance isn't needed today (the `Dockerfile` and
      Phase 4.3 image-publish workflows still produce the
      compose-ready image that future chaos-rig shards will
      `docker load` from). Live tests today distribute
      2/2/1/1 across the four shards; ignored skeletons stay so
      flipping `#[ignore]` is enough to add coverage to the
      matrix.
      Refs: `TESTING.md` § 12.6.
- [x] **4.7** Chaos suite skeleton paired 1:1 with
      `REQUIREMENTS.md` § 10.9. Nine `tests/chaos_<scenario>.rs`
      binaries — one per row of `TESTING.md` § 9 — each
      carrying a single `#[ignore]`'d `#[tokio::test]` whose
      `#[ignore]` reason names the injection mechanism
      (`iptables`, `tc qdisc`, `pg_terminate_backend`,
      `docker compose pause`, etc.). Activated in
      `nightly.yml` (job `chaos`) via
      `cargo nextest run -E 'binary(/^chaos_/)' --run-ignored=all`
      — bodies fill in over time; flipping `#[ignore]` is enough
      to add coverage to the nightly. Issues open via
      `peter-evans/create-issue-from-file` per
      `TESTING.md` § 12.8 (advisory, doesn't block tags).
      Refs: `TESTING.md` § 9.
- [x] **4.8** Server-side ad-template rendering (`templated`
      creative variant). Adds the fourth `creative` `oneOf` arm
      defined in `API.md` § 1 / § 3.5, and extends
      `CreativeTemplate` (`API.md` § 3.6) with optional `template`
      (Liquid source) + `template_engine: "liquid"` fields. Sub-tasks:
      - `creative_templates.template TEXT NULL` +
        `template_engine TEXT NULL` migration with the four RLS
        rules; parse-on-write rejects malformed Liquid with
        `422 / template_parse_error`.
      - Pick the rendering crate (`liquid` recommended — Kevel
        parity for RX migration; `minijinja` is the Rust-native
        alternative). Capture the choice as a `**Note (4.8):**`
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
        added to the suite started in 4.5.
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
      `template_engine`); `REQUIREMENTS.md` § 7.1.1 (RLS rules),
      § 10 (release security).
- [x] **4.9** Rehome the repo to the `knievel-ads` GitHub org.
      Mechanical move + a sweep of the hardcoded paths the
      `${{ github.repository_owner }}` workflow expressions
      can't cover. Splits cleanly into three commits:

      1. **Pre-transfer sweep.** `sed -i
         's|xrl/knievel|knievel-ads/knievel|g;
         s|ghcr.io/xrl|ghcr.io/knievel-ads|g'` across:
         - `Cargo.toml` (`workspace.package.repository`,
           `homepage`)
         - `examples/compose/compose.yaml` (default image),
           `examples/compose/README.md`
         - `charts/knievel/{Chart.yaml,README.md,values.yaml}`
           (`image.repository`, OCI install one-liners,
           `home` / `sources`)
         - `MIGRATION_RX.md` compose snippet,
           `REQUIREMENTS.md` § 8.1 example, `TESTING.md`
           § 12.9 example
         - `PHASES.md` references (cosmetic but consistent
           with the spec-is-the-contract principle)
         Commit + push to `xrl/knievel/main` so the next
         transfer-side push doesn't fight uncommitted state.
      2. **GitHub transfer.** `Settings → Transfer ownership
         → knievel-ads`. GitHub redirects URLs and webhooks;
         existing `ghcr.io/xrl/knievel:*` tags keep resolving
         until explicitly deleted, so cluster pulls don't
         break mid-cutover. Update local remote:
         `git remote set-url origin
         git@github.com:knievel-ads/knievel.git`.
      3. **Post-transfer rebuild.** Tagging a `v*` release
         under the new org fires `release.yml` and publishes
         `ghcr.io/knievel-ads/knievel:vX.Y.Z` (plus
         `:latest` re-pointed to that tag). Cosign cert-
         identity regex auto-rehomes via the workflow's
         `${{ github.repository }}` interpolation. CLI
         release artifacts attach under the new path.
         Document the cutover date in `CHANGELOG.md` (Phase
         5.4 lands the file; until then a one-line
         `Note (4.9)` here suffices). No `main`-side rebuild
         is needed since main pushes don't publish images
         (Phase 4.3 tag-only policy).

      **Auto-adapts** (don't touch):
      - `.github/workflows/release.yml` — image name and
        cosign cert identity both derive from
        `${{ github.repository[_owner] }}`.
      - The `Dockerfile` itself — image-tag-agnostic.

      **Harness setup for the next session.** Whatever
      Claude Code session opens after the transfer needs
      `knievel-ads/knievel` in its GitHub-MCP repo
      allowlist (the system-prompt "Repository Scope"
      block); otherwise the GitHub tools refuse calls.
      `CLAUDE.md` and `PHASES.md` carry every other piece
      of project context across the transfer untouched.

      Refs: `REQUIREMENTS.md` § 8 item 5 (image registry
      pinning), `MIGRATION_RX.md` compose example.

- [x] **4.10** `openapi-generator-cli` wired into CI; Ruby gem
      with `Resource` wrappers + `Enumerable` pagination;
      gem-smoke job runs against the compose stack from 4.1.
      Refs: `REQUIREMENTS.md` § 8 item 3, `API.md` "Pagination."

      Generator CI shipped early as v0.1.1 → v0.1.5 (see Phase
      4 Notes "4.10 (generator CI, early)" below). Wrappers +
      Enumerable pagination shipped in knievel-ruby `f6938cb`
      (gem 0.1.5) — `Knievel::Resources::Base` is the cursor
      walk + page_size validation; eight subclasses (one per
      paginated resource) + a `Knievel::Client` facade that
      parses full URLs. 24 rspec examples cover the wrapper
      contract (cursor walk, `lazy.first(n)` short-circuit,
      `take_while`, page_size bounds, filter forwarding). The
      `x-knievel-paginated*` vendor extensions API.md
      originally promised got redirected to **Phase 6.6** —
      hand-written wrappers know their own paginated set, and
      we're upstreaming extension support to poem-openapi
      rather than carrying a `cargo xtask openapi`
      post-processor. The compose-based gem-smoke job runs as
      a step in `release-ruby-gem.yml` between gem build and
      tag-push, gating every release on a real Knievel server
      + the freshly built gem actually round-tripping the
      Enumerable contract end-to-end.

**Milestone:** `docker compose up` boots a working knievel against
Postgres + MinIO + wiremock; `helm install` against a real
cluster pulls the published `ghcr.io/knievel-ads/knievel` image;
the acceptance suite + chaos rig run against the same image; a
third party can integrate from the gem alone once 4.10 lands.

### Notes

**Note (4.9):** Order-of-operations differed from the plan
text. The GitHub `Settings → Transfer ownership` step (commit
2) happened first — the repo was moved from `xrl/knievel` to
`knievel-ads/knievel` ahead of any in-tree sweep, and the
post-transfer rebuild (commit 3) was already implicit by the
time `v0.1.x` releases started landing under the new org. This
commit is the deferred sweep (commit 1): `Cargo.toml`
`workspace.package.{repository,homepage}`, `REQUIREMENTS.md`
§ 8.1 example, `TESTING.md` § 12.9 example, `MIGRATION_RX.md`
compose snippet, `examples/compose/{compose.yaml,README.md}`,
`charts/knievel/{Chart.yaml,values.yaml,README.md}` (image
repository, OCI install one-liners, cosign cert-identity
regex). Historical PHASES.md mentions of `xrl/knievel`
deliberately left in place — the `[x]` lines are the audit
trail and rewriting them would obscure when the move actually
happened.

**Phase 4.0 (reorder):** Tasks were renumbered to put the Docker
substrate first (compose → image+ghcr → Helm) so every later
deliverable points at a real image. Ruby-gem generation moved
from old-4.8 to new-4.9 (last) because the gem's smoke test
needs a deployable to integration-test against. `ghcr.io/xrl/
knievel` is now the explicit image registry per
`REQUIREMENTS.md` § 8 and `MIGRATION_RX.md`'s compose example;
4.3's task description pins tag policy (`vX.Y.Z` semver +
`latest` re-pointed at the freshest semver release; **no
main-branch publishes**) and the cosign signing mechanism.

**Phase 4.0 follow-up (renumber, post-4.8):** Rehoming the repo
to the `knievel-ads` GitHub org earned its own slot — landed as
new-4.9 (the org migration), pushing the Ruby gem from old-4.9
to new-4.10. The migration is mechanical and well-bounded but
deserves a documented stage so the find/replace surface
(`Cargo.toml`, `examples/compose/`, `charts/knievel/`, the spec
docs) doesn't get reinvented. The gem's "third party can
integrate" milestone language now references 4.10. The image
registry path becomes `ghcr.io/knievel-ads/knievel` once 4.9
lands; the `${{ github.repository[_owner] }}` workflow
interpolations carry both halves automatically.

**Note (4.10, generator CI, early):** The generator-CI half of
4.10 landed ahead of Phase 4 proper because the empty
`knievel-ads/knievel-ruby` repo needed scaffolding for early
client work. `.github/workflows/release-ruby-gem.yml` on
knievel triggers on `v*` tags, mints an installation token via
the `knievel-pipelines` GitHub App
(`KNIEVEL_PIPELINES_APP_ID` / `KNIEVEL_PIPELINES_PRIVATE_KEY`
secrets, scoped to `knievel` + `knievel-ruby`), uses
`openapi-generators/openapitools-generator-action` to
regenerate the Faraday-based gem from the committed
`openapi.yaml`, runs `bundle install` + `rake build` +
load-test as a fail-fast smoke check, then commits and tags
the matching version on `knievel-ruby` using the App token.
The generator config + ignore live in `.github/ruby-client/`
and are copied into `knievel-ruby` on every run (canonical
config follows the spec). The downstream chain — App tokens
trigger workflow runs on push (unlike `GITHUB_TOKEN`) — is
exactly what makes the second hop work:
`.github/workflows/publish-rubygems.yml` on knievel-ruby fires
on the same `v*` tag, rebuilds the gem, and `gem push`es to
RubyGems via `RUBYGEMS_ORG_API_KEY` (a knievel-ruby secret).
First end-to-end run published `knievel 0.1.1` (squat at
`0.1.0`).

**Note (4.10, openapi tags):** Initial generator output
collapsed every endpoint into a 3970-line `Knievel::DefaultApi`
because no operation carried a `tags:` array. Fixed by adding
`src/api_tags.rs` (a `#[derive(Tags)]` enum with one variant
per resource module — `System`, `Orgs`, `Tokens`, `AdLibrary`,
`Advertisers`, `Campaigns`, `Flights`, `Ads`, `Creatives`,
`CreativeTemplates`, `Sites`, `Zones`, `Taxonomy`,
`Decisions`, `Explain`) and a `#[OpenApi(tag = "ApiTags::…")]`
attribute on each of the 15 `#[OpenApi]` impl blocks. The
poem-openapi 5 syntax inherits the impl-level tag onto every
operation in the block, so this was a one-attribute change per
resource. Variant doc comments flow through to tag descriptions
in the spec. The Ruby gem now exposes 15 focused API classes
(`Knievel::AdvertisersApi`, `Knievel::CampaignsApi`, …)
instead of one. Bumped to `0.1.2` since this is a Ruby-surface
breaking change, well within the `gem 0.1.* ↔ server 0.1.*`
compatibility window from REQUIREMENTS.md § 4.

Deferred to Phase 4.10 proper: hand-written `Resource`
wrappers, `Enumerable` pagination keying off
`x-knievel-paginated`, the gem-smoke job against the compose
stack, plus the spec-side polish (root `servers:` block — today
the generated client defaults to `http://localhost`).

**Phase 4.1 follow-up — RLS bypass via Postgres SUPERUSER.**
The Phase 3.30+ wiring exposed a long-standing test-harness
bug: the `postgres:16` docker image creates `POSTGRES_USER`
(`knievel_app` in CI and `examples/compose`) as a SUPERUSER,
and Postgres superusers bypass RLS unconditionally — even with
`FORCE ROW LEVEL SECURITY` set. The cross-tenant isolation
tests (`integration_tenants`, `integration_audit_log`,
`integration_demand`, plus the API-level
`cross_tenant_advertisers_get` / `ad_inline_create_round_trip
_and_cross_tenant`) had been passing on superficial assertions
but silently failed all the `count == 1` / `403` checks once
they actually ran end-to-end. Fixed by
`testlib::db::ephemeral` running `ALTER ROLE CURRENT_USER
NOSUPERUSER CREATEDB` against the admin connection at the
start of every ephemeral DB; `examples/compose/init.sql`
mirrors the downgrade for local dev. Idempotent and matches
`MIGRATION_RX.md`'s production recipe (`knievel_app` is a
non-superuser there). Documented as gotcha 17 in `CLAUDE.md`.

**Note (4.2):** `seed-demo` connects to Postgres directly rather
than going through the OpenAPI client because there's no token in
existence on a clean install (auth chicken-and-egg). The org row's
`id` is derived deterministically from `external_id` via SHA-256
[:12] so the lookup-then-insert pattern doesn't need a tenant
binding the caller hasn't established yet — `INSERT … ON CONFLICT
(id) DO UPDATE … RETURNING (xmax = 0)` handles both the
fresh-install and re-run cases without a SELECT-by-external-id
that RLS would hide. Project ids are derived the same way (hash
of `org_id + '/' + external_id`); the lower-level resources
(advertiser, campaign, flight, ad, creative, site, zone) all use
auto-`bigserial` ids and are upserted via `(project_id,
external_id)` lookups under a proper `(org_id, project_id)`
binding.

The compose `knievel-seed` sidecar now invokes
`knievel-cli seed-demo` against the live Postgres and drops the
bearer at `./tmp/knievel-dev-token` on the host. The Dockerfile
builds and ships both `knievel` and `knievel-cli` in
`/usr/local/bin/`. Open follow-up: when the OpenAPI-generated
client lands in 4.9, `seed-demo`'s post-bootstrap operations
(everything past org + project) can move to HTTP so we exercise
the same handler path RX hits.

**Note (4.3):** Single workflow. `.github/workflows/release.yml`
fires only on `v*` tags and produces, in order: (a) the per-PR
CI gate (`workflow_call` into `ci.yml`); (b) the multi-arch
image with semver tags `vX.Y.Z`, `X.Y`, `X`, and `latest`
re-pointed to the freshest semver release, signed with cosign
keyless via GitHub OIDC and attested via
`actions/attest-build-provenance`; (c) a 4-target build matrix
for `knievel-cli` (`x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl` via `cross`; `x86_64-apple-darwin`
on `macos-13`; `aarch64-apple-darwin` on `macos-14`), each
stripped + tar.gz'd with a `.sha256` sidecar and a cosign
sign-blob bundle; (d) the GitHub Release with a body that pins
the image digest, copies the `cosign verify` invocation, and
provides curl-pipe install one-liners for every CLI target. A
goreleaser-style `checksums.txt` aggregates the sidecar hashes.

**No image is built or pushed for main-branch commits or PRs.**
CI on PRs still **builds** the image via the `Dockerfile` in
ci.yml's per-PR DAG to catch Dockerfile rot, but doesn't push.
Pre-release deployments either build locally (`KNIEVEL_BUILD=1
docker compose build` against the in-tree Dockerfile) or pin a
digest from a published `vX.Y.Z-rc.N` tag.

The Helm and gem publish steps stay stubbed
(`if: false` until 4.4 / 4.10 land their respective artifacts).
Sandbox limitation: I couldn't run the workflow end-to-end here
(no docker daemon, no GitHub OIDC), so trust-but-verify on the
first `v*` tag — known-good action versions (`docker/build-push-
action@v6`, `sigstore/cosign-installer@v3`,
`softprops/action-gh-release@v2`) keep the surprise surface low,
and the YAML parses clean (`python3 -c 'import yaml;
yaml.safe_load(...)'`).

**Note (4.4):** Chart layout under `charts/knievel/` follows the
canonical `helm create` shape — `Chart.yaml`, `values.yaml`,
`templates/_helpers.tpl`, `templates/{deployment,service,
configmap,serviceaccount,ingress,servicemonitor,NOTES}.yaml`.
The ConfigMap renders the figment-shaped `config.yaml`
(REQUIREMENTS.md § 8.1's exact value surface). Secrets are
projected into the pod as env vars (`KNIEVEL_DB_USER`,
`KNIEVEL_DB_PASSWORD`, `KNIEVEL_SENTRY_DSN`) and the rendered
config references them via `${VAR}` interpolation so plaintext
never lands in the ConfigMap. The Deployment carries a
`checksum/config` annotation = `sha256sum` of the rendered
ConfigMap so a values-only `helm upgrade` rolls the pods.

`image.tag` honors a digest form: a value starting with
`sha256:` renders as `repository@sha256:...` (immutable pin),
anything else as `repository:tag`. Operators pin a digest from a
published `vX.Y.Z` (or `vX.Y.Z-rc.N`) release to keep deploys
reproducible.

CI gate (`.github/workflows/ci.yml` `helm-lint` job, no longer
gated on `if: false`): installs `helm v3.16.2` + `kubeconform
v0.6.7`, runs `helm lint`, then `helm template` with all
optional toggles on (Ingress + ServiceMonitor) piped through
`kubeconform -strict -kubernetes-version 1.30.0` against the
default schema set plus the
`datreeio/CRDs-catalog` for the
`monitoring.coreos.com/v1` ServiceMonitor schema. Locally I get
`6 resources found … Valid: 6, Invalid: 0`.

The chart `publish` step in `release.yml` stays gated until
Phase 5.8 (it'll `helm package` + push to an OCI registry,
ideally `ghcr.io/xrl/charts/knievel`). The chart is usable from
the working tree today (`helm install knievel ./charts/knievel
-f my-values.yaml`).

**Note (4.6):** The 4-way shard exposed a TOCTOU race in
`testlib::db::ephemeral`'s SUPERUSER downgrade — many parallel
test processes all `SELECT rolsuper` then `ALTER ROLE
NOSUPERUSER`, but only the first ALTER has privilege; the rest
fail with 42501 (or other SQLSTATE depending on PG version) and
emit a confusing error. Fixed by gating the SELECT/ALTER pair
behind a `pg_advisory_xact_lock(hashtext('knievel_testlib_role_
setup'))` — first lock-holder does the work in a transaction,
subsequent holders see `rolsuper = false` and no-op. Lock
auto-releases on commit, so there's no cleanup cost. 10 stress
runs of shard-1 are clean.

**Note (4.7):** Skeleton binaries land in `tests/chaos_*.rs`
rather than `tests/chaos/` because Cargo's `tests/` directory
doesn't recurse — one binary per scenario keeps the
`binary(/^chaos_/)` filter natural. Nine binaries today,
matching the nine rows of `TESTING.md` § 9. The nightly job's
final command ends in `|| true` so the workflow stays green
while bodies are empty; that drops once the first scenario is
wired (one-line PR change). The compose harness with the
`chaos-injector` sidecar (`NET_ADMIN` for iptables / tc) and
the `wiremock` JWKS service is documented in
`tests/chaos/README.md` but not yet checked in — the first
scenario activation lands the harness alongside its body.

**Note (4.8):** What landed:

- **Migration `0013_templated_creatives.sql`**: adds optional
  `creative_templates.template` + `template_engine` columns
  with a CHECK constraint enforcing the `(NULL, NULL)` or
  `(some, 'liquid')` pair. Drops + re-adds
  `creatives_kind_check` to admit `'templated'` as a fourth
  value. Purely additive — no backfill, existing rows stay
  valid. RLS unchanged (creative_templates already
  project-bound). Migration linter clean.
- **CreativeTemplate handler** (`src/creative_templates.rs`):
  POST + PATCH accept `template` + `template_engine`. Liquid
  source parses on write via the `liquid` crate
  (`ParserBuilder::with_stdlib().build().parse(&src)`); a bad
  source returns `422 / template_parse_error`. Shape errors
  (`template_engine_required`, `template_engine_unsupported`)
  also return 422. PATCH with explicit nulls clears the pair.
  Schema-or-template content change bumps `version`.
- **Creatives handler** (`src/creatives.rs`): `'templated'`
  joins `image | html | native` on the kind check, sharing
  validation with `native` (both require `template_id` +
  `values`). For `templated` writes the handler additionally
  verifies the referenced template carries a non-null
  `template` column — `422 / template_missing_body` when not.
  Unknown / wrong-tenant template ids return
  `422 / template_not_found`.
- **Tests** (`tests/api_templated.rs`): 6 `#[tokio::test]`
  cover the round-trip + every 422 case
  (`template_parse_error`, `template_engine_required`,
  `template_engine_unsupported`, `template_missing_body`).
- **OpenAPI** regenerated (`108653 bytes`); spec drift gate
  clean.

What's deferred (open follow-ups under 4.8):

- **Decision-time rendering.** The `templated` creative variant
  is admitted at the write side, but the decisions handler
  doesn't render the `body` field yet — the snapshot needs to
  carry parsed Liquid templates per `(template_id, version)`,
  and the snapshot loader function is itself unwired (cross-
  cutting blocker for ACC-02 too). Once the snapshot ships, the
  decisions handler grows a `render_templated_body` step that
  pulls the parsed template, renders against `values + ad.* +
  placement.id + decision.snapshotVersion`, and emits the
  rendered string in the typed `creative` `oneOf` arm.
- **Sandbox limits.** The render-time-ms cap and output-bytes
  cap (per the Phase 4.8 task body's "front-load these risks")
  land with the rendering step — they only matter when render
  actually runs.
- **Audit / observability surface.** `decisions:explain` will
  grow a `templated_render` rule per candidate
  (`{result: "rendered" | "skipped" | "timeout" | "oversize"}`)
  alongside the rendering step.

The `liquid` crate (DotLiquid-compatible) was picked for Kevel
parity per `MIGRATION_RX.md` — RX engineers can lift their
existing `Ad Template` source verbatim. `minijinja` remains a
viable swap if Kevel parity becomes a non-goal (the
`template_engine` column was designed for exactly that — admit
new engines additively). Phase 4.7's chaos rig fixture will
gain a `templated_sandbox_escape` scenario once the renderer
ships.

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

- [x] **5.1** `README.md` — landing page with working quickstart.
      Refs: `DOCUMENTATION_PLAN.md` § 4.
- [x] **5.2** `ARCHITECTURE.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 5.
- [x] **5.3** `DEPLOYMENT.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 6.
- [x] **5.4** `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 7.
- [x] **5.5** `RELEASE_CHECKLIST.md`, `RELEASE_PLAYBOOK.md`.
      Refs: `DOCUMENTATION_PLAN.md` § 7.4, § 7.5.
- [x] **5.6** `xtask check-doc-fences`, `check-api-doc`, lychee
      link checking in CI.
      Refs: `DOCUMENTATION_PLAN.md` § 11.2.
- [x] **5.7** First benchmark run; `bench/results/v0.1.md`
      committed.
      Refs: `REQUIREMENTS.md` § 9.2, `TESTING.md` § 8.

      **Note (5.7-followup, snake_case wire-format rule):** The
      camelCase ↔ snake_case drift surfaced in 5.1 (`API.md`
      examples were camelCase; `openapi.yaml` was snake_case)
      hardened into a hard rule: **JSON wire format is
      `snake_case` everywhere**. New gate
      `cargo xtask check-snake-case` walks the spec's component
      schemas + operation parameters and fails CI on any
      violation (383 property names + 17 parameter names
      verified at landing). Doc sweep brought every platform
      doc (`API.md`, `AUTH.md`, `REQUIREMENTS.md`,
      `REPORTING.md`, `TESTING.md`, `MIGRATION_RX.md`,
      `README.md`, `ARCHITECTURE.md`, `DEPLOYMENT.md`,
      `DOCUMENTATION_PLAN.md`) into agreement; path-template
      params normalized to `{org_id}` / `{project_id}` /
      `{token_id}` / `{item_id}` / `{user_id}`.
      JSON-Schema-vocabulary inside `creative_template.schema`
      payloads (`maxLength`, `additionalProperties`, etc.)
      intentionally stays camelCase — that's JSON Schema, not
      knievel wire. Documented in `CONTRIBUTING.md`
      "Wire-format rule" and `DOCUMENTATION_PLAN.md` § 8.1.
- [ ] **5.8** Release-tagging workflow — first
      checklist-gated cut. Multi-arch image published, gem
      published, GitHub Release created.
      Refs: `TESTING.md` § 12.9, `RELEASE_CHECKLIST.md`,
      `RELEASE_PLAYBOOK.md`.

      **Note (5.8 versioning):** The literal `v0.1.0` was
      squatted on RubyGems before any code shipped (see
      CHANGELOG `[0.1.0]`). `v0.1.1` … `v0.1.6` published as
      the gem version ratcheted forward without bumping
      `Cargo.toml` (every tag was a gem-ratchet, not a real
      checklist-gated release). The first cut through
      `RELEASE_CHECKLIST.md` is therefore **`v0.1.7`** —
      formalized via `cargo xtask release-tag 0.1.7` (the
      one-shot wrapper that bumps `Cargo.toml`, refreshes
      `Cargo.lock`, rolls `CHANGELOG.md` `[Unreleased]` →
      `[0.1.7]`, commits, tags, and prints the push commands).
      Subsequent releases use the same wrapper.

**Milestone:** `v0.1.0` tagged. Container image, Helm chart, and
gem published.

### Notes

(none yet)

---

## Phase 6 — Bulk operations & API idempotency follow-ups

**Goal:** Round out the write-side surface that was scoped out
of v0. The `:batchUpsert` implementation shipped in 3.14 stays
in place; this phase carries the deferred polish around it
(POST idempotency parity, macro extraction to kill the
duplication batch made worse, two-pass batch diagnostics, and
the one resource that didn't get the batch surface in 3.14).

Treated as post-v0 because v0 ships with per-row CRUD +
`:batchUpsert` working, and the items below are quality / ergonomics
on top — none block a deployable. Real consumer demand should
re-prioritize within the phase.

**Spec references:**

- `API.md` § 2.1, § 2.2 ("Write contract"), § 3.x POST rows
  ("Idempotent on externalId").
- `REQUIREMENTS.md` § 4 ("Generated client compatibility").
- 3.14 `:batchUpsert` follow-up notes.

**Tasks:**

- [ ] **6.1** Single-row `external_id` idempotency on POST
      creates. Today every POST handler returns
      `409 external_id_conflict` on a re-POST of the same
      `externalId`, contradicting the API.md POST rows that
      label them "Idempotent on externalId" and inconsistent
      with the `:batchUpsert` path that already round-trips
      cleanly. Adds an `Existing(200)` flavor to every
      `ApiResponse` enum that today only carries `Created(201)`,
      and rewrites the existing 409 tests across
      `api_advertisers.rs`/`api_campaigns.rs`/etc. accordingly.
      `crud_contract!` (6.2) should land on top of this, not
      under it — the macro needs to bake in the new shape.
      Refs: `API.md` § 3.x, CLAUDE.md "Open known gaps,"
      3.14 follow-up note (1).
- [ ] **6.2** `crud_contract!` macro extraction. Deferred from
      3.8 / 3.9 / 3.14. Per-resource handlers (`advertisers`,
      `campaigns`, `flights`, `ads`, `sites`, `zones`,
      `creatives`, `creative_templates`, `taxonomy`) currently
      duplicate ~80% of body — the standard prologue
      (`open_project_tx`, `Idempotency-Key` wrapper, etag
      bump, error envelope mapping) is identical and the
      `:batchUpsert` arrival made it worse. Extracts the shared
      shape into a macro stamped from per-resource type lists.
      Lands after 6.1 so the macro encodes the new
      POST/200/`Existing` flavor.
      Refs: 3.8 / 3.9 notes, 3.14 follow-up note (2),
      cross-cutting open follow-up #3 (now superseded by this
      phase).
- [ ] **6.3** Two-pass validate-then-execute for batch errors.
      Today `:batchUpsert` stops at the first failing row by
      design — once a Postgres tx aborts on one statement,
      every subsequent statement returns the same
      "transaction is aborted" error. A two-pass pattern
      (cheap validation pass under the same tx with savepoints,
      OR a dry-run REST surface, OR client-side row partitioning)
      would surface every offending row at the cost of doubled
      DB round-trips or extra transaction complexity. Pick a
      shape only when a real bulk-sync consumer asks for it.
      Refs: 3.14 final note ("revisit if the gem-side
      bulk-sync flow asks for it").
- [ ] **6.4** `:batchUpsert` for ad-library items. The 3.7
      ad-library commit deferred this — the upsert pattern from
      3.14 generalizes cleanly but the endpoint wasn't on
      3.14's resource list. Single tx with per-row diagnostics
      matching the established shape; FK semantics differ
      slightly because ad-library is org-scoped, not
      project-scoped (so `open_project_tx` is the wrong
      prologue — `db::begin_bound` on the org id with role
      check inline).
      Refs: `API.md` § 3.7 ("Ad library"), 3.7 commit note.

- [ ] **6.5** Cursor pagination for TEXT-id list endpoints
      (`listAdLibraryItems`, `listTokens`). Phase 3.33 shipped
      cursor pagination keyed on a bigserial `id`, but
      `ad_library_items.id` and `api_tokens.id` are TEXT
      primary keys — they need a `(created_at, id)` tuple
      cursor (timestamp as the sort key, id as the unique
      tiebreaker). Generalize `crate::pagination` to take a
      strategy or expose a second `resolve_timestamp` /
      `next_cursor_timestamp` pair, then wire the two
      handlers. Tests piggyback on the
      `tests/api_pagination.rs` shape. Both endpoints today
      return `nextCursor: null` and a hard `LIMIT 500`, so
      consumers won't break — they just gain real pagination
      when this lands.
      Refs: `API.md` § "Pagination" (non-paginated v0
      footnote), `PHASES.md` § 3.33 note.

- [ ] **6.6** Vendor-extension support — upstream poem-openapi
      first, then surface `x-knievel-paginated*` natively.
      poem-openapi 5 has no operation-level extension API
      (`MetaOperation` hardcodes its serialized fields plus a
      special-case `x-code-samples`); generic extensions fight
      the derive-macro model. Plan: land an extension API in
      poem-openapi upstream — likely
      `#[oai(extension("x-foo", json!({...})))]` or a
      registry-side `MetaExtensions` map serialized via the
      `x-` prefix convention — then come back here and tag
      every paginated `#[OpenApi]` operation with
      `x-knievel-paginated: true` /
      `x-knievel-paginated-items: items` /
      `x-knievel-paginated-cursor: nextCursor`. The Ruby
      wrapper from 4.10 doesn't need them (hand-written, knows
      its own paginated set), but a future Python/Go binding
      or a doc-site generator (Redoc-style) would. Until the
      upstream PR lands, the spec stays extension-free —
      `cargo xtask openapi`-side post-processing was
      considered and rejected as a maintenance liability we'd
      have to keep porting forward.
      Refs: `API.md` § "Pagination" (deferred-extensions
      footnote), `PHASES.md` § 3.33 note,
      `https://github.com/poem-web/poem` (upstream).

**Milestone:** `:batchUpsert` is consistent across every
resource that declares it; POST creates are truly idempotent;
handler bodies are short again. Every list endpoint in
`API.md` is cursor-paginated. Vendor extensions ship natively
through poem-openapi.

### Notes

(none yet)

---

## Phase 7 — Admin audit UI

**Goal:** Operator-facing browser console for auditing the state
of the ad server. SPA built on the same OpenAPI surface every
other client uses; no admin-only side channels. Read-only audit
views first, editing later. See `UI.md` for the full plan
(stack, repo layout, codegen, auth, CORS, deploy).

Treated as a standalone post-Phase-3 workstream because the UI
is gated on a stable OpenAPI surface — most of Phase 3's task
list churns it. Phases 7.x can run in parallel with Phase 4
(deployable) and Phase 6 (bulk follow-ups) once Phase 3 closes.

**Spec references:**

- `UI.md` (entire document — the canonical plan).
- `API.md` — endpoints the UI consumes; no new shapes.
- `AUTH.md` § 2 — bearer-token semantics; UI honors them
  unchanged.

**Pre-staged in advance of this phase:**

- `ApiConfig.allowed_origins` (`src/config.rs`) and the
  matching example in `config.example.yaml`. Default empty;
  consumed by 7.2 below. Landed early so dev configs can
  declare the Vite origin before the middleware install.

**Tasks:**

- [x] **7.1** Repo skeleton: `web/admin/` with Vite + React +
      TypeScript, ESLint + Prettier, vitest harness, Mantine v7
      installed, TanStack Router + Query wired with a
      placeholder route. No real views yet; this commit just
      proves `pnpm install && pnpm dev && pnpm test && pnpm
      build` work clean. Include a README pointing at `UI.md`.
      Refs: `UI.md` "Stack," "Repo layout."
- [x] **7.2** CORS middleware install. Wraps the poem route
      with `poem::middleware::Cors` when
      `cfg.api.allowed_origins` is non-empty: methods `GET,
      POST, PATCH, DELETE, OPTIONS`; allow-headers
      `Authorization, Content-Type, Idempotency-Key, If-Match,
      X-Request-Id`; expose-headers `ETag, Location,
      X-Request-Id, X-Idempotency-Replayed`;
      `allow_credentials: false` (bearer tokens only);
      `max_age: 600`. New `tests/api_cors.rs` slice covers
      empty-config / matching-origin / non-matching-origin /
      preflight. Add the slice's binary to `cargo xtask
      test-shape`. Empty-default behavior must not install the
      middleware at all (no preflight overhead, no
      `Access-Control-Allow-Origin` on responses).
      Refs: `UI.md` "CORS"; pre-staged config field.
- [x] **7.3** OpenAPI codegen rail. New
      `xtask/src/ui_client.rs` shelling out to `pnpm --dir
      web/admin exec openapi-typescript ../../openapi.yaml -o
      src/api/generated.ts`; `--check` mirrors
      `xtask/src/openapi.rs` exactly. New
      `.github/actions/node-setup/` composite (pnpm + Node
      pinned) mirroring `rust-setup`. CI gains `ui-client-drift`
      peer of `openapi-drift`. The generated file is checked
      in, not gitignored.
      Refs: `UI.md` "OpenAPI codegen"; `xtask/src/openapi.rs`.
- [x] **7.4** Auth in the UI — **OIDC Authorization Code +
      PKCE primary, paste-a-token fallback**. Wires
      `react-oidc-context` (over `oidc-client-ts`) against
      Keycloak's admin-UI client (public client, PKCE
      required, no client secret in the bundle); adds
      `/oidc/login` + `/oidc/callback` + `/oidc/logout`
      routes, `RequireAuth` guard, and a 401-aware fetch
      wrapper that runs a silent refresh before redirecting
      to login. The fetch wrapper reads `X-Request-Id` from
      every response and attaches it to the TanStack Query
      result so error toasts/panels can surface it for
      support correlation (`UI.md` "Error handling").
      Tokens live in memory inside `UserManager` with
      `sessionStorage` persistence; closing the tab clears
      them. The paste-a-token form remains behind
      `admin_ui.oidc.require_oidc: false` for dev,
      bootstrap, Keycloak outages, and CI fixtures —
      identical fetch path, just a `kvl_*` Bearer instead of
      a JWT. Adds the `admin_ui:` config block on the API
      side plus `GET /admin/config.json` so one bundle works
      across envs (issuer + public client_id served from
      runtime config, not baked at build time). Smallest
      "who am I" endpoint lands here if it doesn't exist yet
      and validates both Bearer flavors identically. The
      error-state matrix from `UI.md` "Error handling" lands
      with this task — 400/401/403/404/409/422/429/5xx/
      network-error each get the documented UX shape.
      Refs: `UI.md` "Auth", "Error handling"; `AUTH.md`
      "Keycloak Setup — Human Admin UI (PKCE)."
- [ ] **7.5** Org/project browser. List + detail for `/v1/orgs`
      and `/v1/projects/{project_id}`. First end-to-end slice
      that exercises the typed client + Query + Router stack.
      Cursor pagination wired even though `next_cursor` is
      still `null` server-side (envelope is real today).
- [ ] **7.6** Resource audit views (read-only). Tables for
      advertisers, campaigns, flights, ads, creatives, sites,
      zones, taxonomy, creative templates, ad library. Server-
      side pagination/sort/filter against the cursor envelope.
      Detail panes inspect raw JSON for fields the table
      doesn't surface.
- [ ] **7.7** Editing surface. PATCH/POST forms with
      react-hook-form + zod, idempotency-key handling
      (UUIDv4 minted client-side per submit), optimistic
      invalidation. Roll out per resource behind a feature
      flag so each surface gets a real review.
      **Token-mint show-once UX:** introduces a
      `<MintRevealModal>` component used by every endpoint
      that returns a server-only secret (token mint +
      HMAC secret rotation, future mint endpoints). Modal
      shows the plaintext exactly once, dismissal gated
      behind an explicit "I've stored this" checkbox (no
      X-close, no Esc, no clickaway), wipes from React
      state and Query cache on close. Per `UI.md` "Auth"
      / "Token-mint show-once UX"; `AUTH.md` "Opaque
      Tokens."
      **Creative image upload:** drag-and-drop picker
      backed by `POST /v1/projects/{p}/creatives/{id}/image`
      (`src/image_upload.rs`), client-side validation
      mirrors `images.upload.max_bytes` +
      `allowed_mime_types` from the runtime config so
      operators see the limit before submit, server still
      enforces. Shows progress + success state; failure
      surfaces the API error envelope (per the 7.4 error
      matrix).
- [ ] **7.8** Reporting + event-flow inspector. Charts on
      rollups, a tail view over `/events` (poll-based for v0;
      revisit if/when push lands). Time-bucket controls match
      `REPORTING.md`.
- [ ] **7.9** OIDC hardening. Wires Keycloak's
      `end_session_endpoint` into `signoutRedirect()` so
      logout invalidates the SSO session (not just the local
      tokens), adds an idle-warning modal driven by
      `oidc-client-ts` events with a grace-refresh path,
      and surfaces role-claim-driven UI gating that hides
      admin-only surfaces from `editor` / `reader` claim
      values. UI gating is **not** a security boundary —
      knievel still enforces every authz check server-side;
      this is purely cosmetic. Includes a documented
      Keycloak admin-UI client setup (public client + PKCE
      + group-membership claim mapper) verified end-to-end
      against a real realm; the runbook lands in `AUTH.md`'s
      "Keycloak Setup — Human Admin UI (PKCE)" section in
      the same commit. (The original "custom admin-session
      endpoint backed by argon2id user credentials" was
      retired when OIDC became the primary flow — Keycloak
      owns user identity; knievel doesn't.)
      Refs: `AUTH.md` "Keycloak Setup — Human Admin UI
      (PKCE)"; `UI.md` "Auth."
- [ ] **7.10** Polish: Playwright e2e in `nightly.yml`,
      bundle-size budgets, accessibility sweep (axe in CI for
      the main routes).
- [ ] **7.11** Single-image ghcr publish for the admin UI.
      The Node build runs in **GitHub Actions** (not as a
      Docker stage) so pnpm's store is cached natively via
      `actions/setup-node` `cache: pnpm`; the Dockerfile
      gains exactly one new line (`COPY web/admin/dist
      /var/lib/knievel/admin`) plus a
      `KNIEVEL_ADMIN_UI__STATIC_DIR` env var. `release.yml`
      and the per-PR `ui-build` job grow a Node/pnpm setup
      step + `pnpm --dir web/admin install --frozen-lockfile
      && pnpm --dir web/admin build` ahead of the existing
      `docker/build-push-action` call; the build context
      already contains the populated `dist/`. Lands a new
      `admin_ui:` config block (`static_dir`, plus the OIDC
      sub-block from 7.4) consumed by `src/server.rs` to
      mount poem's `StaticFilesEndpoint` at `/admin/` with
      `.index_file("index.html").fallback_to_index()` for
      SPA history routing; enables the `static-files`
      feature on the `poem` workspace dep. Serves
      `GET /admin/config.json` registered **before** the
      static nest so it isn't shadowed by a bundle file.
      Empty `static_dir` → mount not installed, `/admin/*`
      returns 404 (same image runs as a headless API).
      New `cargo xtask build-image [--skip-ui]` wrapper
      drives `pnpm build && docker build` for local devs.
      The existing `release.yml` keeps publishing
      `ghcr.io/<owner>/knievel` unchanged. New
      `tests/api_admin_ui.rs` slice covers: unset
      `static_dir` returns 404 on `/admin/`; set returns
      `index.html`; deep paths fall back to `index.html`
      for SPA routing; `/admin/config.json` round-trips
      the OIDC block and isn't shadowed by a same-named
      bundle file.
      Refs: `UI.md` "Deployment"; `AUTH.md` "Knievel-side
      configuration."
- [ ] **7.13** Decision tester / debugging surface. A
      project-scoped form at `/reports/test` that lets an
      operator construct a real
      `POST /v1/projects/{p}/decisions` request from a
      typed builder (zone, channel, ad-type, custom
      targeting JSON, optional `force.*` overrides), fires
      it, and renders the served ad alongside the response
      from `:explain` so the operator can see *which*
      flights/ads matched and the per-flight reason
      strings. The single most-valuable surface for
      "why isn't my campaign serving?" debugging — common
      in Kevel-style admin consoles, missing from the
      original plan. Honors the `force.*` three-control
      gate (`API.md` § 1; `decisions.force_overrides_enabled`
      kill-switch + per-project `allow_force_decision` flag
      + caller's role); the UI hides the `force.*` controls
      entirely when the role/flag combination forbids them
      (server still enforces). Lives in the Reports rail
      section; reuses 7.7's react-hook-form + zod machinery
      and the 7.4 fetch wrapper. (7.12 was vacated when
      fly.io was dropped; numbering jumps to 13 to leave
      the audit trail visible.)
      Refs: `API.md` § 1 "Decisions"; `UI.md` "Information
      architecture" + "Phasing."

**Milestone:** Operators can log in, browse every project-
scoped resource, edit the editable ones, inspect rollups
+ event flow, and live-test decisions with full explain
output — all over the public OpenAPI surface, no admin
side channels. UI + API ship as a single
`ghcr.io/<owner>/knievel` image; the same image runs as a
headless API when `KNIEVEL_ADMIN_UI__STATIC_DIR` is unset.

### Notes

**Note (7.4):** Landed across three commits per the
"sub-commit prefixed `Phase X.Y (partial):`" convention used
elsewhere (Phase 4.10): one for the API-side handshake
(`/admin/config.json` + `/v1/whoami` + `admin_ui:` config),
one for the UI-side OIDC plumbing (UserManager + routes +
RequireAuth + paste-token form + typed fetch wrapper), and a
third for silent refresh + the unified error-notification
helper.

Three subtleties worth not relearning:

- **`oidc-client-ts` access tokens are sync-readable only via
  a cache.** `UserManager.getUser()` is async, but the fetch
  wrapper needs the bearer synchronously per request. The
  fix: `userManager.ts` keeps a module-level `cachedUser`
  primed at boot and updated via `addUserLoaded` /
  `addUserUnloaded` / `addAccessTokenExpired`. Reads stay
  zero-allocation.

- **`openapi-fetch`'s discriminated union confuses TS's
  narrowing.** `const r = await client.GET(...)` is
  `{ data, error?: never, response } | { data?: never, error,
  response }`. After `if (r.error || !r.data)` the compiler
  collapses `r` to `never` and `r.response.status` errors as
  "property does not exist on never." Workaround: pull
  `result.response.status` BEFORE the conditional (where TS
  still sees the full union). Documented in
  `src/auth/PasteTokenLogin.tsx`.

- **Silent-refresh retry is GET-only.** POST / PATCH bodies
  may have been consumed by the time the 401 arrives, so
  cloning the original Request to retry doesn't generalize.
  Non-safe methods fail with 401 and the next request picks
  up the refreshed token; the user sees one error toast
  before TanStack Query re-fetches. Documented in
  `src/api/client.ts`.

The error-notification helper (`src/api/errors.ts`) covers
all nine UI.md "Error handling" status branches via Mantine
notifications with `X-Request-Id` appended for support
correlation. Inline 403 panels and field-level mapping for
400/422 land in 7.5+ once the views with real forms exist.

**Note (7.3):** `xtask ui-client --check` writes the fresh
codegen to `target/xtask-ui-client-check.ts` (gitignored
under `target/`) instead of bringing in a `tempfile` dep on
xtask — same drift behavior, one less crate. The output
path passed to `pnpm exec openapi-typescript` is
absolutized before the `cwd → web/admin` shell-out, so
both the canonical `web/admin/src/api/generated.ts` and the
drift-check temp resolve correctly regardless of where the
caller pointed them.

`generated.ts` is checked in (130 KB, ~4400 lines of typed
bindings) for the same reason `openapi.yaml` is — it's part
of the contract surface; reviewers see when it changes.
Excluded from prettier (`.prettierignore`) and ESLint
(`eslint.config.js` ignores) since linting auto-generated
code is just rot.

CI gains five UI jobs gated on `prime`: `ui-client-drift`
(peer of `openapi-drift`), `ui-typecheck`, `ui-lint` (also
runs `pnpm format:check`), `ui-test`, `ui-build`. They share
the new `.github/actions/node-setup/` composite which mirrors
`rust-setup` (caller checks out first; pnpm action setup +
`actions/setup-node` `cache: pnpm` + `pnpm install
--frozen-lockfile` against `web/admin/pnpm-lock.yaml`).
Path-filtering on `web/admin/**` was considered and dropped:
the cache hit makes a no-op job cheap, and the cycle
complexity isn't worth saving ~20 s on Rust-only PRs.

**Note (7.2):** poem 3.1's `Cors` requires the
`MakeRouter`-style `.with(cors)` chaining. The conditional
install pattern uses `EndpointExt::boxed()` because
`routes.with(cors)` returns a different type than the bare
`routes`, so the `match` arms have to converge on
`BoxEndpoint<'static>`. The boxing is one heap allocation
per request lifetime (cheap) and the alternative — always
installing Cors with empty allow lists — would still fire
the preflight handler on OPTIONS, which contradicts the
"empty config = no middleware" goal.

`tests/api_cors.rs` doesn't need `DATABASE_URL` — the test
hits `/healthz` (system endpoint, served regardless of DB).
`knievel::server::cors_layer(&cfg)` is exposed publicly so
tests rebuild the same shape against fixture configs
without copy-pasting the methods/headers/max_age list.

For the non-matching-origin test, poem returns
`CorsError::OriginNotAllowed` which surfaces as 401 (per
its `error_response` impl, not 403 as I'd assumed); the
test checks "no `Access-Control-Allow-Origin` header in
response and status is not 2xx" rather than pinning the
exact code, since the security-relevant assertion is the
absence of the ACAO echo.

**Note (7.1):** Pinned versions: React 18.3, TypeScript 5.7,
Vite 6, Vitest 3 (Vitest 2 conflicts with Vite 6's `Plugin`
type — Vitest 2 bundles its own Vite 5 internals, so
`defineConfig` from `vitest/config` resolves a different
`Plugin<any>` than `@vitejs/plugin-react`'s; bump to Vitest
3 to align), Mantine 7, TanStack Router 1.x (file-based
routing via `@tanstack/router-plugin/vite`), TanStack Query
5. ESLint 9 flat config; Prettier 3.

`src/routeTree.gen.ts` is generated by the router plugin on
every dev/build run, so it's gitignored. `pnpm typecheck`
and `pnpm lint` run `tsr generate` first (via a shared
`pnpm routes` script) so they don't see a stale-or-missing
file. `@tanstack/router-cli` is added as a dev dep purely
for the CLI; the Vite plugin still drives generation in
dev/build.

PostCSS config lives in `postcss.config.mjs` (not `.cjs`) —
the package is `"type": "module"`, so `.cjs` would need a
separate ESLint env block to silence `no-undef` on `module`.
ESM is one less moving part.

pnpm 10 blocks install scripts by default; `pnpm.onlyBuiltDependencies:
["esbuild"]` in package.json approves esbuild's postinstall
(needed for Vite's bundler binary). No other deps need
build-script approval today.

**Note (7.12 dropped):** Originally planned a fly.io
sample-app deploy (knievel + Postgres + Keycloak federated
to GitHub) so evaluators could spin up the whole stack with
one script. Removed because fly.io's free trial is bounded
to 7 days — a perpetual demo target doesn't fit. The
reusable pieces (Keycloak realm export, demo-data seed
script, GitHub-as-social-IdP setup) stay valid; they belong
in 7.4 / 7.9 / 7.11. If a hosted demo target comes back
later (Render, Railway, a self-hosted box, or a
docker-compose recipe), slot it in as a fresh task with the
new provider's specifics.

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
