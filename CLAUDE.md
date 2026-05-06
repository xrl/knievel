# CLAUDE.md

Repo-specific guidance for Claude sessions working on knievel.
Brief; designed to bring a fresh session up to speed in 60 seconds.

## What this repo is

A Rust ad-serving platform inspired by Kevel's domain model.
Multi-tenant, Postgres-native, OpenAPI-first. v0 is early — the
spec corpus is heavy and the code is just starting.

The platform docs are the contract. Read in this order if you've
never seen the repo:

1. `README.md` — when it exists; today it's a stub
2. `PHASES.md` — execution roadmap; **the living progress log**
3. `REQUIREMENTS.md` — working spec
4. `API.md`, `AUTH.md`, `REPORTING.md` — focused area specs
5. `TESTING.md` — test plan and CI gates
6. `DOCUMENTATION_PLAN.md` — meta-plan for the doc surface

Consumer-specific material lives in `MIGRATION_<NAME>.md` (only
`MIGRATION_RX.md` today). Platform docs stay generic per
`REQUIREMENTS.md` § 8.

## How work is structured

Phases 1–5 live in `PHASES.md`. **Each task is one git commit**
prefixed `Phase X.Y: <task name>`. The PHASES.md update marking
the task `[x]` rides along in the same commit.

```bash
# Fast audit trail at any time:
git log --oneline --grep "Phase"
```

When picking up a session:

1. Read `PHASES.md`. Status is current as of the last commit.
2. Find the next `[ ]` task. Its **Refs** point you at the
   spec section to consult.
3. Each `[x]` task may carry a `**Note (X.Y):**` block with
   surprises or deviations from the spec — those are the
   gotchas worth knowing.
4. The `### Notes` block at the end of each phase carries
   post-milestone bug fixes and follow-ups across tasks.

If a task changes scope, edit the task description in the same
commit that does the work. Don't delete completed tasks — the
`[x]` lines are the audit trail.

## Where things live

```
.cargo/config.toml          # `cargo xtask` alias
.github/
  actions/rust-setup/       # composite: toolchain + cache
                            # caller MUST checkout first
  workflows/
    ci.yml                  # per-PR DAG (TESTING.md §12.4)
    nightly.yml             # advisory, scheduled
    release.yml             # on `v*` tag
build.rs                    # captures git SHA + build timestamp
                            # via shell-out (no vergen)
migrations/                 # sqlx migrations, numeric prefix
src/
  config.rs                 # figment loader, ${VAR} interpolation
  observability.rs          # tracing init (json or compact)
  server.rs                 # poem bootstrap + graceful shutdown
  state.rs                  # AppState (PgPool today)
  system.rs                 # /healthz, /readyz, /version via
                            # poem-openapi
  lib.rs                    # exposes openapi_spec_yaml() for xtask
  main.rs                   # entry point
testlib/                    # DB test harness; testlib::db::ephemeral
xtask/                      # repo CLI: linters + codegen
  src/lint_migrations.rs    # 4 RLS rules from REQUIREMENTS §7.1.1
  src/check_cross_tenant.rs # gate (1) — endpoint coverage
  src/openapi.rs            # spec drift gate
tests/                      # integration tests, slice-named
  api_placeholder.rs        # binary anchor for the api/contract slice
  integration_migrations.rs # binary anchor for the integration slice
  cross_tenant_manifest.toml
openapi.yaml                # generated spec, drift-checked in CI
```

## Sandbox limitations seen so far

The execution environment for past sessions has been a sandbox
with these caveats — they may or may not apply to your session.
Verify before assuming:

- **Docker daemon not reachable.** `docker run` fails. Means
  integration tests that need real Postgres can't run locally;
  `tests/integration_migrations.rs` self-skips when
  `DATABASE_URL` is unset and runs against the CI Postgres
  service container instead.
- **Cargo registry IS reachable.** Building external deps works
  fine.
- **`psql` and `cargo` and `git` are installed.** No `nextest`,
  no `kubectl`, no `helm`.
- **Backgrounding processes is unreliable.** `cargo run &`
  followed by `curl` doesn't always work to test handlers; use
  `poem::test::TestClient` for in-process HTTP testing.

## Conventions established (non-obvious)

- **Em-dash, not colon, in workflow YAML strings.** `echo "TODO:
  X"` parses as a YAML mapping value and breaks. Use `"TODO — X"`
  instead.
- **Migration files start with `SET search_path TO knievel,
  public;`** so unqualified DDL lands in the right schema. But
  this isn't enough on its own — see the gotcha below about
  `sqlx::migrate` and `after_connect`.
- **Workspace deps via `[workspace.dependencies]`** with members
  opting in via `dep = { workspace = true }`. Don't pin per-crate
  unless intentional.
- **Module-level `#![allow(dead_code)]`** is the agreed-upon way
  to keep `clippy -D warnings` happy when a config struct is
  parsed today but read in a later phase. Remove the allow
  attribute when consumers land.
- **Spec follow-ups stay in `PHASES.md` Notes**, not as code
  comments. Prevents rot when the spec is updated.
- **`xtask` subcommand pattern**: stub the file in the phase
  that creates the shape, real impl lands in the phase that
  earns it. Each stub names the phase that will replace it.
- **OpenAPI spec is the contract**; `cargo xtask openapi --check`
  fails CI when the committed `openapi.yaml` drifts from what the
  binary emits. Regenerate with `cargo xtask openapi`.

## Gotchas already hit

Future sessions: don't relearn these.

1. **`sqlx::migrate` creates `_sqlx_migrations` BEFORE the
   migration runs.** That means `SET search_path` at the top of
   the migration file is too late — the tracking table is
   already in `public`. Fix: configure `after_connect` on
   `PgPoolOptions` so every new connection sets
   `search_path = knievel, public` before any query. See
   `testlib/src/db.rs` for the working pattern.

2. **`#[derive(Default)]` overrides per-field serde defaults.**
   If a struct has `#[serde(default = "fn_name")]` on a field
   AND `#[derive(Default)]` on the struct, the derived `Default`
   uses Rust's `Default::default()` (e.g. `""` for `String`)
   instead of the named function — but only when the entire
   struct is missing from the input. Fix: implement `Default`
   manually for any struct that has serde-defaulted fields and
   whose enclosing struct uses `#[serde(default)]`.

3. **`EnvFilter::try_new` is permissive.** It accepts strings
   that look bogus (e.g. spaces). To trigger a real parse error
   in tests, use a directive with an invalid level:
   `knievel=POTATO`.

4. **`poem-openapi` 5 emits OpenAPI 3.0.0**, not 3.1, despite
   `REQUIREMENTS.md` § 6 specifying 3.1. Live with 3.0 for now;
   revisit when the library catches up.

5. **`nextest` `binary(/regex/)` parse-fails** when no binary
   matches. `--no-tests=pass` only handles "filter parsed but
   matched no tests at runtime." The filter must resolve to at
   least one binary at parse time. If a slice has no real tests
   yet, drop a comment-only file in `tests/` whose name matches
   the filter (e.g. `tests/api_placeholder.rs` for
   `binary(/^api/)`).

6. **Local composite actions need an explicit checkout first.**
   `uses: ./.github/actions/rust-setup` can't be resolved until
   `actions/checkout@v4` has put the `action.yml` on disk.
   Self-checkout from inside the composite is too late.

7. **`cargo fmt` removes column alignment.** Don't pre-align
   `match` arm `=>` operators to a column; rustfmt will undo it.
   Single-space is the canonical style.

8. **`config_version` is implemented as a SEQUENCE, not a
   table.** The `REQUIREMENTS.md` § 7.2 wording says "row in a
   bookkeeping table"; the SEQUENCE has the same semantics
   (`SELECT last_value`, `SELECT nextval(...)`) but doesn't trip
   the migration linter's "every CREATE TABLE in knievel needs
   RLS" rule. Spec follow-up noted in `PHASES.md`.

## Running the gates locally

```bash
# Full per-PR CI parity (fast):
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
cargo xtask lint-migrations
cargo xtask check-cross-tenant
cargo xtask test-shape
cargo xtask openapi --check

# Integration tests (need DATABASE_URL):
DATABASE_URL=postgres://knievel_app:dev@localhost:5432/knievel \
  cargo test --workspace
```

## Push policy for this repo

The user has explicitly approved **direct pushes to `main`** for
this work. That's a non-default decision logged here so future
sessions don't second-guess it. The pattern is:

- Each task = one commit, prefixed `Phase X.Y: <task name>`
- Push immediately after commit
- PHASES.md update rides along with the work

If you start a session and the user hasn't reaffirmed this,
treat it as scope-specific and ask. The standing per-CLAUDE.md
git rules in the harness still apply (no `--no-verify`, no
force-push to main, etc.).

## What's next when you pick this up

`PHASES.md` is the source of truth. As of the last writing:

- **Phase 1**: complete (foundation rails: workspace, CI, xtask
  linters, first migration, DB harness).
- **Phase 2**: complete (walking skeleton: config, tracing,
  poem server, /healthz, /readyz, /version, /openapi.json).
- **Phase 3**: broad strokes only. **First task at session
  resume:** decompose Phase 3 into 1.x/2.x-level granularity
  (tasks `3.1` through `3.18` are sketched; refine).

Risks to front-load (per `PHASES.md` cross-cutting risks):

1. `poem-openapi` round-trip of `CreativeTemplate.schema` JSON
   Schema documents — spike before Phase 3.5 (CreativeTemplate
   handlers).
2. Aurora-specific behavior (NOTIFY drop on failover, advisory
   lock release semantics) — simulated in code, validate against
   real Aurora before Phase 5 tag.
3. HMAC rotation overlap with stable `dedup_key` — land in
   Phase 3.9 with `proptest` coverage.

## When in doubt

- The spec docs are right; if code disagrees, the code is wrong
  (or the spec needs an update — which is also a commit).
- A feature that needs a doc update gets the doc update in the
  same PR as the code.
- New project-scoped endpoints REQUIRE a paired entry in
  `tests/cross_tenant_manifest.toml` — `xtask check-cross-tenant`
  fails CI without it.
- New migrations REQUIRE the four RLS rules from
  `REQUIREMENTS.md` § 7.1.1 — `xtask lint-migrations` fails CI
  without them.
- When you hit a sandbox limitation, document it in the relevant
  phase's note rather than working around silently.
