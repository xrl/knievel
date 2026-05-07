# Contributing to knievel

## Setup

```sh
# Toolchain (matches rust-toolchain.toml).
rustup show

# Cargo registry only — most builds work offline after first cache.
cargo build --workspace

# Postgres for integration tests. Compose is the simplest path:
docker compose -f examples/compose/compose.yaml up -d knievel-postgres
export DATABASE_URL=postgres://knievel_app:dev@localhost:5432/knievel
```

`testlib::db::ephemeral` materializes a per-test temporary database
on top of `DATABASE_URL`, so a single Postgres serves every
integration test. The harness automatically downgrades the role to
`NOSUPERUSER CREATEDB` to keep RLS honest (gotcha 17 in `CLAUDE.md`).

## Branch policy

PR-based, single-track main. No long-lived feature branches.

- Land in main directly when the task is one-commit-or-less and you're
  a maintainer.
- Open a PR for anything else — non-trivial features, schema
  migrations, doc reorganizations.
- No release branches in v0; tags are cut from main.

## Commit conventions

**Subject line shape**: `Phase X.Y: <imperative description>`.
Matches the `[x]` lines in `PHASES.md`.

```
Phase 3.33: cursor pagination — server side
```

The body explains the **why**, not the what. The diff already shows
the what. Wrap at 72 characters. Refer to spec sections explicitly
(`API.md § "Pagination"`, `REQUIREMENTS.md § 7.1.1`) when the change
is policy-driven.

When a task changes scope, edit the task description in `PHASES.md`
in the **same commit** that does the work. Don't delete completed
tasks — `[x]` lines are the audit trail.

## Code review expectations

- **Maintainer +1 minimum.** Two for security-sensitive changes
  (auth, HMAC, RLS rules, tenancy boundaries).
- **CI green before merge.** Per-PR DAG in `.github/workflows/ci.yml`
  runs in 8 minutes on a cold cache. See `TESTING.md` § 12.4 for the
  full gate matrix.
- **No squash-merge for spec changes.** Doc changes that explain the
  spec rationale stay as their own commits in history.

## Wire-format rule

**JSON wire format is `snake_case`. Hard rule, enforced in CI.**

Every JSON property name and every query parameter in
`openapi.yaml` is `snake_case` — `site_id`, not `siteId`;
`next_cursor`, not `nextCursor`; `is_active`, not `isActive`.
The rule applies to both new endpoints and any documentation
example that purports to show wire JSON.

`cargo xtask check-snake-case` walks `openapi.yaml`'s component
schemas and operation parameters and fails CI on any violation.
The check ignores OpenAPI structural metadata
(`operationId`, `additionalProperties`, etc. — those follow
OpenAPI's own conventions) and JSON-Schema-vocabulary fields
inside `creative_template.schema` payloads (`maxLength`,
`additionalProperties`, etc. — those are JSON Schema, not
knievel wire).

Generated SDKs follow each language's idiom:

- Ruby: `obj.is_active` (snake_case method names — generator
  transliterates from the spec).
- Python: `obj.is_active`.
- TypeScript: free to expose `isActive` if the project prefers,
  but the wire JSON it sends and receives stays snake_case.

When you add a new endpoint or schema, the gate is automatic;
when you add a doc example that includes JSON, copy snake_case
from a real spec example or run a quick `cargo xtask openapi` +
spot-check.

## Test expectations

Every change carries proportional test coverage. Specifics:

- **New project-scoped endpoints** require a paired entry in
  `tests/cross_tenant_manifest.toml`. `xtask check-cross-tenant`
  fails CI without it.
- **New migrations** must satisfy the four RLS rules in
  `REQUIREMENTS.md § 7.1.1`. `xtask lint-migrations` fails CI
  without compliance.
- **New API surface** gets API-level tests (`tests/api_*.rs`) plus
  any necessary integration tests (`tests/integration_*.rs`).
  Skipped automatically when `DATABASE_URL` is unset; CI provides it.
- **New invariants worth proptesting** get proptest coverage
  (HMAC rotation, selection priority, idempotency body-hash).

`TESTING.md` § 4–§ 7 has the full test plan.

## Doc expectations

- **Code examples in any `.md` file are tested in CI.**
  `xtask check-doc-fences` parses fenced code blocks per language
  (`rust`, `yaml`, `json`, `sql`). Examples that don't parse fail
  the gate. Tag a block as `rust,ignore` (same as rustdoc) to
  opt out.
- **Quickstart examples** in `README.md` are extracted and replayed
  by `acceptance.rs` (`TESTING.md` § 7.1). A README curl that
  doesn't work fails CI.
- **API tables** in `API.md` are compared against `openapi.yaml` by
  `xtask check-api-doc`. Adding an endpoint to the spec without
  documenting it in the table fails CI.

When a code change adds or removes a feature, the PR description
must list the docs affected. The reviewer enforces by inspection.

## Platform vs. consumer

`REQUIREMENTS.md`, `API.md`, `AUTH.md`, and `REPORTING.md` are the
**platform contract.** They describe knievel as a generic
multi-tenant ad platform and stay free of consumer-specific
identifiers, terminology, and assumptions. Reviewers reject PRs that
leak consumer-specific concepts (RX organizations, scientist.com URL
shapes, etc.) into platform docs.

Consumer-specific recipes live in `MIGRATION_<NAME>.md`. RX is the
first such file (`MIGRATION_RX.md`); future consumers add their
own. The platform docs link to migration files as examples; they
never depend on them.

## What gets pushed where

- **Generated client code** — `knievel-ads/knievel-ruby` is
  regenerated from `openapi.yaml` on every `v*` tag push to this
  repo. Don't hand-edit the generated `*Api.rb` / `*.rb` model
  files; the regen wipes them.
- **Hand-written wrapper layer** in `knievel-ruby` (`lib/knievel/
  resources/`, `lib/knievel/client.rb`) is maintained directly in
  that repo and protected through `.openapi-generator-ignore` (the
  canonical version lives in this repo at
  `.github/ruby-client/.openapi-generator-ignore` and is copied on
  every regen).
- **The OpenAPI spec** at the root (`openapi.yaml`) is generated
  by `cargo xtask openapi`. Don't hand-edit it.
- **Helm chart values** in `charts/knievel/` are hand-maintained.

## Local CI parity

The same gates that block merges run from your shell:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
cargo xtask lint-migrations
cargo xtask check-cross-tenant
cargo xtask test-shape
cargo xtask openapi --check
cargo xtask check-doc-fences
cargo xtask check-api-doc
```

Run all eight before pushing; the CI runs them in a DAG so failures
land per-gate, but locally they're a one-line pre-push hook.

## Reporting issues

GitHub Issues is the front door. Security issues go to the address
in `SECURITY.md` instead — please don't open public issues for
security findings.
