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
src/auth/                   # opaque-token parse, argon2id hash,
                            # Role enum, Principal, BearerAuth
                            # security scheme (Phase 3.2/3.3)
src/handlers.rs             # open_project_tx — common prologue
                            # for every project-scoped handler
                            # (auth + role + tenant binding +
                            # project-existence check)
src/db.rs                   # begin_bound / begin_auth_lookup —
                            # tenant-binding tx openers
src/idempotency.rs          # Idempotency-Key replay store
                            # (24h, body_hash via canonical
                            # serde_json::to_vec + SHA-256)
src/orgs.rs                 # OrgApi (projects)
src/tokens.rs               # TokensApi (mint / list / revoke)
src/<resource>.rs           # one file per project-scoped CRUD
                            # resource (advertisers, campaigns,
                            # flights, ads, creatives,
                            # creative_templates, sites, zones,
                            # taxonomy). Same handler shape;
                            # macro extraction deferred until
                            # 3.14 :batchUpsert lands.
testlib/                    # DB test harness; testlib::db::ephemeral
                            # plus testlib::tenant::begin_bound
xtask/                      # repo CLI: linters + codegen
  src/lint_migrations.rs    # 4 RLS rules from REQUIREMENTS §7.1.1
  src/check_cross_tenant.rs # gate (1) — endpoint coverage
  src/openapi.rs            # spec drift gate
tests/                      # integration tests, slice-named
  api_*.rs                  # binary(/^api/) — full HTTP via
                            # poem::test::TestClient
  integration_*.rs          # binary(/^integration/) — DB-level
                            # tests; self-skip without DATABASE_URL
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
- **Project-scoped handler shape (3.8+):** `auth: BearerAuth,
  state: Data<&AppState>, project_id: Path<String>, body:
  Json<...>` → `crate::handlers::open_project_tx(pool,
  &principal, &path_project_id, Role::Editor)` returns a
  tenant-bound `Transaction` with both `knievel.org_id` and
  `knievel.project_id` set, after running the standard authz
  prologue. Add the operation's row to
  `tests/cross_tenant_manifest.toml` in the same commit — the
  cross-tenant gate fails CI without it.
- **Cross-tenant manifest paths use the OpenAPI spec's literal
  param names.** `:project_id` in `#[oai(path = ...)]` becomes
  `{project_id}` in the spec; the manifest entry must match
  exactly, including the brace-delimited param name.
- **Wire timestamps formatted at the SQL layer** via
  `to_char(... AT TIME ZONE 'UTC',
  'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')`. Avoids needing the
  `sqlx` `time` feature; the `created_at` / `updated_at`
  columns flow into `String` row fields verbatim.
- **Object response types that round-trip via JSON** (idempotency
  cache, etc.) need explicit `#[derive(Object,
  serde::Serialize, serde::Deserialize)]` — `Object` does NOT
  derive serde traits automatically. Same for request types when
  the handler hashes them via `idempotency::body_hash`.
- **Random IDs come from `argon2::password_hash::rand_core::{OsRng,
  RngCore}`.** argon2 is already a dep; no need to add `rand`
  directly. Pattern in `src/orgs.rs::random_pj_id`.
- **Per-resource test files duplicate fixture helpers
  intentionally** (`seed_org_project`, `mint_token`,
  `build_app`). Extracting to a shared `tests/common/mod.rs`
  module is a future refactor — wait until the test patterns
  stabilize across `:batchUpsert` (3.14) and the hot path
  (3.18).

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

9. **`#[derive(ApiResponse)]` rejects two variants sharing the
   same status code** — they collide as duplicate keys in the
   generated OpenAPI YAML. To distinguish flavors at the same
   status (e.g. fresh create vs idempotency replay), use one
   variant carrying `Option<String>` for a differentiator
   header: `Created(Json<T>, #[oai(header = "X-Foo")]
   Option<String>)`. Send `None` for one flavor, `Some("true")`
   for the other.

10. **`clippy::large_enum_variant` trips on `ApiResponse` enums
    where the typed Json variant is a wide struct** (10+ fields).
    Boxing for one alloc per response isn't worth obscuring the
    typed return — add `#![allow(clippy::large_enum_variant)]`
    at module level. Convention used in
    `flights.rs`/`creatives.rs`/`sites.rs`/etc.

11. **`serde_json::Value` works as a `poem-openapi` Object field,**
    surfacing as a free-form JSON `Any` schema in the spec.
    Used for `creative_template.schema` (Phase 3.10 closes
    cross-cutting risk #1: round-trips POST → GET → PATCH
    bit-for-bit).

12. **Postgres FORCE'd RLS default-denies operations without a
    matching policy.** `audit_log` (Phase 3.4) relies on this
    for append-only — only `FOR SELECT` and `FOR INSERT`
    policies exist; UPDATE/DELETE silently affect 0 rows. The
    migration linter's rule 3 (every `CREATE TABLE` in knievel
    needs ENABLE RLS) still applies to partition leaves even
    though the parent's policies cover them — keep the
    redundant `ALTER TABLE ... ENABLE ROW LEVEL SECURITY` on
    each leaf you create explicitly.

13. **Postgres 14 doesn't have `NULLS NOT DISTINCT`.** For
    nullable lookup keys (e.g. `idempotency_keys.project_id`),
    use a unique partial expression-index that coalesces NULL
    to a sentinel: `UNIQUE (a, coalesce(b, ''), c)`. Pattern in
    `migrations/0005_idempotency_keys.sql`.

14. **`api_tokens` RLS auth-bootstrap is a chicken-and-egg fix.**
    The principal extractor needs to read `secret_hash` before
    any tenant binding is known. Solution: a single-row bypass
    via the `knievel.auth_lookup_id` GUC, scoped to one row by
    primary key (`OR id = current_setting('knievel.auth_lookup_id',
    true)` in the USING clause). `db::begin_auth_lookup` sets
    the GUC; the `WITH CHECK` clause for writes deliberately
    omits it so the bypass cannot escalate writes. See
    `migrations/0003_api_tokens.sql` and
    `src/auth/security.rs`.

15. **Migration linter rule 4 was loosened** in Phase 3.4 to
    accept either `knievel.project_id` OR `knievel.org_id`
    (matching `REQUIREMENTS.md` § 7.1.1's "or equivalent
    session-scoped tenant binding" wording). The regex now
    scans the entire `CREATE POLICY` statement (up to its
    terminating `;`) so multi-line USING bodies with nested
    parens and `WITH CHECK`-only INSERT policies parse
    correctly.

16. **Project creation seeds default taxonomy in the same
    transaction.** `create_project` (in `src/orgs.rs`) binds
    `knievel.project_id` mid-transaction with `set_config(...,
    true)` after the project insert succeeds, then calls
    `taxonomy::seed_default_taxonomy`. A crash between the
    project insert and the seed leaves no half-applied state.

17. **Postgres SUPERUSER bypasses RLS even with
    `FORCE ROW LEVEL SECURITY`.** The `postgres:16` docker image
    creates `POSTGRES_USER` (= `knievel_app`) as a SUPERUSER,
    which silently defeats every cross-tenant isolation test —
    `FORCE` only gates the table owner, not superusers. Fixed
    by `testlib::db::ephemeral` running `ALTER ROLE CURRENT_USER
    NOSUPERUSER CREATEDB` against the admin connection at the
    start of every ephemeral fixture; `examples/compose/init.sql`
    does the same for local dev. Idempotent. If you ever
    re-introduce a code path that connects as a superuser to
    run knievel queries, RLS will appear to work in dev and
    silently fail in CI — keep app sessions on the
    NOSUPERUSER role.

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

**Work directly on `main`.** The user has standing approval to
commit + push directly to `main` for the duration of Phase 3
big-feature development. No feature branches, no PRs, no asking.
Each task = one commit prefixed `Phase X.Y: <task name>`, pushed
to `origin/main` immediately. The PHASES.md update marking the
task `[x]` rides along in the same commit.

```bash
git checkout main             # if not already there
# ... edit, run gates ...
git add <files> && git commit -m "Phase X.Y: ..."
git push -u origin main
```

The harness may default a session onto a feature branch
(`claude/...`) — when that happens, do the work on that branch
during the session, then merge fast-forward to `main` and push
at the end. `git checkout main && git merge --ff-only <branch>
&& git push origin main` is the expected close-out. Don't open a
PR; don't wait for review.

This standing-policy override is in effect until the user
explicitly says otherwise. After Phase 3 ships, we'll re-evaluate
(probably moving to PRs for Phase 4 deployable work).

The harness git safety rules still apply unconditionally:
- no `--no-verify`, no `--no-gpg-sign`
- no force-push to main (`git push --force`/`git push +ref`)
- no `git rebase -i` / `git add -i` (interactive flags break the
  non-interactive harness)
- never amend a published commit; always create a new commit
- never `reset --hard` something pushed without checking first

## What's next when you pick this up

`PHASES.md` is the source of truth. As of the last writing:

- **Phase 1**: complete (foundation rails).
- **Phase 2**: complete (walking skeleton).
- **Phase 3**: complete (29 tasks across tenancy/auth, CRUD,
  hot-path, and cursor pagination). `xtask check-cross-tenant`
  reports all project-scoped endpoints covered.
- **Phase 4**: deployment rails complete (compose stack,
  containers, helm, acceptance suite, ad templating, ruby gem
  codegen, repo rehome). Only 4.11 (kind-helm e2e) is open.
- **Phase 5**: docs done; bench harness extended to a full
  three-signal suite (criterion + iai-callgrind + dhat) plus a
  macro-load script, all run from cloud sessions via
  `cargo xtask bench-all`. Schema lives in
  `bench/results/SCHEMA.md`. Next is **5.8** (release-tagging
  workflow → first checklist-gated cut, `v0.1.7`).

Risks to front-load (per `PHASES.md` cross-cutting risks):

1. ~~`poem-openapi` round-trip of `CreativeTemplate.schema`~~
   **Closed in 3.10** — `serde_json::Value` round-trips
   bit-for-bit through the typed handler surface. See
   `creative_template_json_schema_round_trips`.
2. Aurora-specific behavior (NOTIFY drop on failover, advisory
   lock release semantics) — simulated in code, validate against
   real Aurora before Phase 5 tag.
3. HMAC rotation overlap with stable `dedup_key` — land in
   Phase 3.16 (renumbered from 3.9) with `proptest` coverage.

**Open known gaps** (documented as PHASES.md notes; resolve when
the corresponding handler feature lands):

- **External-id idempotency on POST creates.** `API.md` says
  POST `/projects` is "idempotent on externalId" — the existing
  row should come back at 200, not 409. Today the handler
  returns 409 `external_id_conflict`. Moved to **Phase 6.1**
  (post-v0); `:batchUpsert` already round-trips cleanly.
- **Cursor pagination.** ✅ Closed for the 8 demand+inventory
  list endpoints in **3.33** (cursor is
  `base64url(JSON{kind, last_id})`, default limit 50, max 500;
  `kind`-validated to catch cross-resource replay). The 3
  taxonomy endpoints stay non-paginated (bounded-small
  per-project sets). `listAdLibraryItems` + `listTokens` deferred
  to **Phase 6.5** because their TEXT primary keys need a
  `(created_at, id)` tuple cursor.
- **`If-Match` etag validation on PATCH.** Etag is bumped on
  every PATCH, but PATCH doesn't read the `If-Match` header
  yet. Future task.
- **Site URL/aliases uniqueness across the union.** The unique
  constraint covers `(project_id, url)` only; aliases are
  application-layer for v0. Partial expression-index when write
  throughput needs it.
- **`Creative` is a flat union, not a typed `oneOf`.** `API.md`
  § 3.5 documents a `oneOf` discriminated by `type`; the v0
  wire shape uses a single `kind` column with per-kind nullable
  fields. Spec follow-up noted in 3.10's PHASES.md note.

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
