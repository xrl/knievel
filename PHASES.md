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
- [ ] **1.7** `xtask lint-migrations` real implementation. The 4
      rules from `REQUIREMENTS.md` § 7.1.1 gate (2). Six fixtures
      under `xtask/tests/fixtures/migrations/` from
      `TESTING.md` § 10.1. Wire into CI.
      Refs: `TESTING.md` § 10.1.
- [ ] **1.8** `xtask check-cross-tenant` skeleton: walks the
      OpenAPI spec (initially empty), expects every
      `/v1/projects/{p}/...` operation to have a paired test
      registered in `tests/cross_tenant_manifest.toml`. Passes
      vacuously today; gate is real once endpoints land.
      Refs: `TESTING.md` § 6.5, `REQUIREMENTS.md` § 7.1.1 gate (1).
- [ ] **1.9** `testlib::db::ephemeral` — wrapper around `sqlx::test`
      / testcontainers for tests that need a real Postgres. First
      integration test exercises 1.6's migration round-trip.
      Refs: `TESTING.md` § 5.1.

**Milestone:** `cargo nextest run` passes, `cargo xtask
lint-migrations` passes, the CI DAG is green against an empty
business surface. Rails are real before any train rides them.

### Notes

(none yet)

---

## Phase 2 — Walking skeleton handlers

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

- [ ] **2.1** figment config loader + typed `Config` struct
      against `config.example.yaml`. Layer: defaults → file → env.
      `${VAR}` and `${VAR:default}` interpolation; missing required
      vars are a hard error at startup.
      Refs: `REQUIREMENTS.md` § 10.1.
- [ ] **2.2** `tracing-subscriber` JSON output + `EnvFilter`. Pulls
      level/format from config. OTel and Sentry initialization is
      stubbed (booleans honored, no exporters wired yet —
      Phase 3 work).
      Refs: `REQUIREMENTS.md` § 10.2, § 10.3, § 10.4.
- [ ] **2.3** `main.rs` binds `poem` at the configured `bind_addr`,
      installs SIGTERM/SIGINT handlers, drains in-flight requests
      with a bounded shutdown timeout. No handlers wired yet — bare
      poem app with a single `not_found` route.
      Refs: `REQUIREMENTS.md` § 10.7.
- [ ] **2.4** `/healthz` handler. `200` if process is up. No
      authentication. Integration test via `poem::test::TestClient`.
      Refs: `API.md` § 5, `REQUIREMENTS.md` § 10.6.
- [ ] **2.5** `/readyz` handler. `200` only if DB is reachable
      (snapshot/flusher/leader checks land in later phases as TODOs
      flagged in the readiness output). Integration test that
      asserts 503 when DB is unreachable.
      Refs: `API.md` § 5, `REQUIREMENTS.md` § 10.6, § 10.9.
- [ ] **2.6** `/version` handler. Build metadata from `vergen`
      (git SHA, build time). Schema version. The `auth` block is
      a stub (modes `[]`, no issuers) until Phase 3 lands real
      auth.
      Refs: `API.md` § 5, `AUTH.md` "Effective-policy visibility."
- [ ] **2.7** `poem-openapi` setup. `/openapi.json` serves the
      generated spec. `/healthz`, `/readyz`, `/version` are
      registered as OpenAPI operations so the spec reflects them.
      Refs: `REQUIREMENTS.md` § 3, `API.md` "Path Structure."
- [ ] **2.8** `xtask openapi --check` — generates the spec from
      the binary, diffs against committed `openapi.yaml`, fails on
      mismatch. Wire into CI. Initial commit also adds the
      generated `openapi.yaml`.
      Refs: `TESTING.md` § 6.3, § 12.7.
- [ ] **2.9** Phase milestone: confirm full CI DAG green; update
      this file's Phase 2 status; refine any tasks/notes for
      Phase 3 based on what we learned.

**Milestone:** A `cargo run` starts a server that responds to
`/healthz`, `/readyz`, `/version`, and `/openapi.json` with honest
values. CI is fully green: `cargo fmt`, `clippy`, the test suite,
all four `xtask` linters, and the OpenAPI drift check.

### Notes

(none yet)

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

**Tasks (broad strokes, refined when Phase 2 closes):**

- [ ] **3.1** Tenant model migrations: organizations, projects, RLS
      policies, the `current_setting('knievel.project_id')` binding.
      `audit_log` table.
      Refs: `REQUIREMENTS.md` § 4, § 7.1, § 7.3.
- [ ] **3.2** Opaque token mint + verify (argon2id), `Principal`
      extractor, role enum.
      Refs: `AUTH.md` "Opaque Tokens," "Authorization."
- [ ] **3.3** First handler: `POST /v1/orgs/{orgId}/projects` —
      with the cross-tenant negative test. Proves the auth +
      tenant-resolution + persistence loop end to end.
      Refs: `API.md` § 2.1.
- [ ] **3.4** Inventory + demand-chain migrations and CRUD
      (Advertisers, Campaigns, Flights, Ads, Creatives,
      CreativeTemplates, Sites, Zones; read-only Channel, Priority,
      AdType).
      Refs: `API.md` §§ 3.1–3.9, `REQUIREMENTS.md` § 5.
- [ ] **3.5** `crud_contract!` macro — uniform per-resource test
      contract.
      Refs: `TESTING.md` § 6.4.
- [ ] **3.6** `:batchUpsert` — single transaction, per-row diagnostics.
      Refs: `API.md` "Write contract."
- [ ] **3.7** Idempotency middleware (24 h replay window).
      Refs: `API.md` "Idempotency."
- [ ] **3.8** Snapshot loader: cold load, LISTEN, 5 s poll backstop,
      Aurora-failover reconnect.
      Refs: `REQUIREMENTS.md` § 7.2.
- [ ] **3.9** Decision API — `POST /v1/projects/{p}/decisions`.
      `selection::filter`, `priority`, `weighted_random`. HMAC
      mint + verify with rotation overlap.
      Refs: `API.md` § 1, `REQUIREMENTS.md` § 6.1.
- [ ] **3.10** Decision explainer — `POST .../decisions:explain`.
      Three-control gate for `force.*` + audit log row.
      Refs: `API.md` § 1, `AUTH.md` "Endpoint → minimum role."
- [ ] **3.11** Event channel + COPY flusher. `events_raw` parent
      + first leaf partition. Dedup logic.
      Refs: `REQUIREMENTS.md` § 7.3, § 7.6, `API.md` "Replay,
      dedup, and counts."
- [ ] **3.12** Event endpoints `/e/i/{signed}` + `/e/c/{signed}`.
      Refs: `API.md` § 4.
- [ ] **3.13** Partition manager + leader election (advisory lock
      + watchdog).
      Refs: `REQUIREMENTS.md` § 7.4, § 7.5.
- [ ] **3.14** `events_rollup` + watermark + leader-elected rollup
      computation.
      Refs: `REQUIREMENTS.md` § 7.3, `REPORTING.md` § "Schema for
      Reporters."
- [ ] **3.15** JWT validator + JWKS cache + `claim_mapping`. Boot-
      time auth lint.
      Refs: `AUTH.md` "JWTs," "Startup Linting."
- [ ] **3.16** `/version` real auth block.
      Refs: `AUTH.md` "Effective-policy visibility."
- [ ] **3.17** Ad Library (org-scoped) + reference vs inline ad
      `oneOf`.
      Refs: `REQUIREMENTS.md` § 5.1, `API.md` § 2.4, § 3.4.
- [ ] **3.18** S3-compatible image upload.
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
