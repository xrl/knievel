# Knievel Documentation Plan

What we publish, who reads it, and how the docs hand off to each
other so a reader can decide — confidently and quickly — whether
knievel fits.

This file is the working spec for the doc surface. It is not user
documentation; it is the plan that produces user documentation.
Companion to `REQUIREMENTS.md`, `API.md`, `AUTH.md`, `REPORTING.md`,
and `TESTING.md`.

## 1. Goals

1. **A reader can decide in 30 seconds whether to keep reading.**
   The README's first screen tells them what knievel is, who it's
   for, and what it isn't. No waterfall of badges or marketing copy.
2. **A reader can decide in 10 minutes whether knievel fits their
   architecture.** `ARCHITECTURE.md` covers the data flow, the
   storage model, the multi-tenancy story, and the named tradeoffs
   without dragging the reader into endpoint shapes.
3. **An operator can decide in 30 minutes whether they can run it.**
   `DEPLOYMENT.md` is the operator's checklist: prereqs, sizing,
   secrets, upgrades, alerts, runbooks.
4. **An integrator can ship a working call within a workday.** The
   README quickstart plus `API.md` plus the generated client gem
   should be enough; docs do not duplicate the OpenAPI contract.
5. **Every example we publish runs.** Examples are extracted and
   tested in CI (§ 11). Doc rot is a CI failure, not a stale page.
6. **The platform-vs-consumer split (REQUIREMENTS.md § 8) is
   preserved.** Platform docs stay generic; consumer-specific
   material lives in `MIGRATION_<NAME>.md`.

## 2. Audiences and the Decision Funnel

The evaluator's journey, with the doc that serves each step:

| Stage | Time | Question | Doc |
|---|---|---|---|
| 1. Discover | 30 s | "Is this for me at all?" | `README.md` (top of file) |
| 2. Skim | 3 min | "Does the shape fit?" | `README.md` (full) |
| 3. Architect | 10 min | "Will it work in my stack?" | `ARCHITECTURE.md` |
| 4. Try | 15 min | "Can I get it running?" | `README.md` quickstart + compose |
| 5. Operate | 30 min | "Can I run it in prod?" | `DEPLOYMENT.md` |
| 6. Integrate | hours | "What's the wire?" | `API.md`, generated client docs |
| 7. Migrate | days | "How do I move from X?" | `MIGRATION_<NAME>.md` |

Stages 1–5 must be answerable from the platform docs alone. Stage
6 may reference language-specific client material (e.g. the Ruby
gem's README). Stage 7 is consumer-specific.

A reader should never need to read `REQUIREMENTS.md` to make a
decision. That document is the working spec; it answers "why is
knievel built the way it is" once a reader has decided to engage
deeply, not "should I engage."

## 3. Doc Inventory

### 3.1 Existing (platform-core)

| File | Role | Audience |
|---|---|---|
| `REQUIREMENTS.md` | Working spec / platform contract | Maintainers, deep evaluators |
| `API.md` | Endpoint reference | Integrators |
| `AUTH.md` | Authentication and authorization detail | Operators, integrators |
| `REPORTING.md` | dbt + reporting integration | Data teams |
| `TESTING.md` | Test plan and CI gates | Maintainers |

### 3.2 Existing (consumer adapter)

| File | Role |
|---|---|
| `MIGRATION_RX.md` | RX-specific Kevel-to-knievel migration guide |

### 3.3 New (this plan creates them)

| File | Role | Owns |
|---|---|---|
| `README.md` | Landing page, elevator pitch, quickstart | First-touch decision |
| `ARCHITECTURE.md` | Visual + textual system overview | Stage 3 above |
| `DEPLOYMENT.md` | Operator install + run guide | Stage 5 above |
| `CONTRIBUTING.md` | Dev environment, branch policy, PR conventions | Contributors |
| `SECURITY.md` | Vulnerability reporting, supported versions | Anyone with a finding |
| `CHANGELOG.md` | Per-release human changelog (keep-a-changelog format) | Upgraders |
| `RELEASE_CHECKLIST.md` | Items required at tag time (§ 7.1.1 of REQUIREMENTS) | Maintainers |
| `RELEASE_PLAYBOOK.md` | What to do when a release goes sideways | Release engineer |
| `LICENSE` | MIT (per `Cargo.toml`) | All |

### 3.4 Supporting directories

| Path | Contents |
|---|---|
| `examples/compose/` | Reference single-node compose stack used by README quickstart and acceptance tests (§ 7 of TESTING.md) |
| `examples/helm/` | Sample `values.yaml` per deployment shape (single-tenant, multi-tenant, K8s SA tokens) |
| `examples/dbt/` | Skeleton dbt project per `REPORTING.md` |
| `examples/curl/` | Hand-runnable curl walkthroughs per `API.md` resource |
| `bench/results/` | Per-release benchmark artifacts (§ 9 of REQUIREMENTS) |
| `docs/diagrams/` | Source files for any non-ASCII diagram (mermaid `.mmd` or `.dot`) |

## 4. `README.md` — Outline

Length target: **400 lines or fewer.** The README is the busiest
piece of doc surface; brevity is enforced.

Section order:

1. **Tagline** — one line, matches `Cargo.toml`'s `description`.
2. **What it is** — one paragraph, three sentences. Names the
   shape (Rust, multi-tenant, Postgres-native ad serving with
   OpenAPI 3.1) and the audience (teams running ad delivery for
   one or many publishers).
3. **Status** — small table: version, license, supported Postgres
   versions, supported architectures, "v0 — surface stabilizing"
   honesty line.
4. **Why knievel** — 3–5 bullets, each a comparison the reader is
   already making in their head:
   - vs. Kevel (proprietary; you can self-host knievel)
   - vs. building from scratch (we ship the boring parts: auth,
     partitions, snapshot loader, generated client)
   - vs. a generic ad framework (knievel is opinionated about
     Postgres + OpenAPI + multi-tenant)
   - vs. an OpenRTB stack (knievel is publisher-side direct ad
     serving, not an exchange)
5. **What's in v0** — bulleted feature list grouped by chain
   (Decision API / Management API / Events / Auth / Multi-tenancy /
   Observability). Two-line max per item.
6. **What's deliberately not in v0** — short bullets, link to
   `REQUIREMENTS.md` § 11. Setting expectations early prevents
   later disappointment.
7. **Quickstart (5 minutes)** — docker compose up; mint a token;
   create one of each resource; issue a decision; observe the
   `events_raw` row. Tested by `examples/compose/` plus ACC-01..14
   (`TESTING.md` § 7.1).
8. **Hello-world decision** — one curl request, one redacted
   response. Links to `API.md` for the full surface.
9. **Architecture in one diagram** — the system-shape ASCII from
   `REQUIREMENTS.md` § 3, condensed to fit one screen. Caption
   links to `ARCHITECTURE.md`.
10. **Deployment** — 4-line summary: helm chart, compose manifest,
    Postgres 14+, S3-compatible object store. Links to
    `DEPLOYMENT.md`.
11. **Documentation map** — table of the platform docs with
    one-line descriptions, so a reader landing here knows where to
    go next.
12. **Project status and stability** — what "v0" means; the
    additive-forever compatibility policy (`REQUIREMENTS.md` §
    6.4) summarized.
13. **License + acknowledgements** — MIT; "domain model inspired by
    Kevel"; `poem`, `sqlx`, etc.

Conventions:

- No badges above the tagline. Badges (CI, version) live just below
  status if at all; they don't earn the top inch.
- Code examples are runnable, copy-pasteable, and tested. The
  quickstart's commands are extracted and replayed in CI as part
  of ACC-01..14.
- Every external link is to a doc in this repo or to a stable
  third-party reference. No links to draft RFCs, no links to
  internal company resources.
- Emoji use: zero. Matches the rest of the platform docs.

## 5. `ARCHITECTURE.md` — Outline

Length target: **800 lines or fewer.** Aimed at a reader who has
finished the README and is asking "will it work in my stack?"
Heavy use of diagrams; light use of code.

Section order:

1. **Where knievel sits** — block diagram of the calling app,
   knievel, Postgres, object store, OTel collector, Sentry. Names
   the trust boundary (server-to-server in v0).
2. **Component map** — the in-process pieces:
   - HTTP server (`poem` + `poem-openapi`)
   - In-memory snapshot
   - Snapshot loader (LISTEN + 5 s poll backstop)
   - Event channel + flusher (`COPY`)
   - Partition manager (leader-elected via advisory lock)
   - Auth layer (opaque + JWT)
   - Idempotency cache
3. **Hot path: a decision request, end to end** — sequence
   diagram. Token validated → tenant resolved → snapshot lookup →
   filter/priority/weight → HMAC-mint URLs → enqueue
   `decision` event → respond. Annotated with where the
   sub-millisecond budget goes.
4. **Cold path: an event ping, end to end** — sequence diagram.
   HMAC verify → enqueue → `COPY` to partitioned `events_raw` →
   eventual rollup.
5. **Configuration lifecycle** — management write → DB commit →
   `NOTIFY` → snapshot diff-pull → atomic swap. Includes the poll
   backstop. Calls out the bound: 5 s worst-case staleness.
6. **Storage model** — one schema, RLS as defense in depth,
   partitioning policy, retention. Diagram of `events_raw`'s
   leaf-partition layout. Links to `REQUIREMENTS.md` § 7 for the
   detailed rules.
7. **Multi-tenancy** — Org → Project → resources. Three deployment
   shapes (single-project, project-per-environment,
   project-per-tenant). When to pick which.
8. **Auth at a glance** — opaque tokens vs. JWTs vs. K8s SA
   tokens, summarized in a 6-row table; links to `AUTH.md` for
   detail.
9. **Observability stack** — what's a metric, what's a log, what's
   a span, what's a Sentry breadcrumb. Per-tenant data lives in
   logs and traces, not Prometheus (default-low cardinality).
10. **Failure model** — short summary of `REQUIREMENTS.md` § 10.9.
    The two cross-cutting principles ("reads degrade later than
    writes," "failures surface, not silently drop") get top
    billing.
11. **Capacity model** — what the SLO targets are (with the
    "TARGET (unverified)" caveat), what scales out (knievel pods),
    what scales up (Postgres tier), the connection budget.
12. **Named tradeoffs** — short prose section, one paragraph per
    decision:
    - Postgres-only in v0 (vs. Redis/Cassandra/etc.)
    - In-memory snapshot (vs. per-request DB lookup)
    - Server-to-server only in v0 (vs. browser-direct)
    - Opaque tokens + JWT both supported (vs. one-or-the-other)
    - No required Postgres extensions beyond `pgcrypto`
    - In-process partition manager (vs. `pg_partman`)
13. **Where to read more** — single closing table linking each
    deeper topic to its source doc.

Diagrams: ASCII first (matches existing platform docs).
Mermaid (`.mmd`) is allowed for sequence diagrams that exceed ~25
lines of ASCII; rendered output checked into `docs/diagrams/`
alongside the source so GitHub renders the SVG without round-tripping
through a build step.

## 6. `DEPLOYMENT.md` — Outline

Length target: **900 lines or fewer.** The operator's checklist.
Prescriptive where the spec allows; opinionated where it doesn't.

Section order:

1. **Prerequisites**
   - Postgres 14+ (cluster writer endpoint reachable; `pgcrypto`
     available; partitions managed in-process so no `pg_partman`).
   - S3-compatible object store (AWS S3, MinIO, R2, GCS-via-S3).
   - OpenTelemetry collector (optional but recommended).
   - Sentry project (optional).
2. **Sizing guidance** — per `REQUIREMENTS.md` § 9. One pod CPU /
   memory ask, connection budget (~12 per pod), expected throughput
   per pod with the "TARGET (unverified)" caveat.
3. **Database setup** — schema, role, grants. `pgcrypto` extension.
   The `knievel_app` role's `search_path`. The `knievel_reader`
   role for downstream analytics (per `REPORTING.md`). Backup
   responsibility (operator-owned).
4. **Helm install** — the recommended path. Walkthrough of a
   `values.yaml` from `examples/helm/` covering: image, replicas,
   resources, database, events retention, hmac secret bootstrap,
   sentry, otel, ingress, service-account, security context.
5. **Compose install** — single-binary + bring-your-own-Postgres
   for local dev and reference single-node deployments. Same
   compose stack used by acceptance tests (§ 7 of TESTING.md).
6. **Bare metal / systemd** — short. The container image is the
   blessed path; bare-metal users get the static binary + a sample
   unit file and a one-line "you own backups, monitoring, and
   process supervision."
7. **Secrets management** — DB password, HMAC default secret,
   Sentry DSN. How they're projected into the container (env vars
   referenced by `${VAR}` interpolation in `config.yaml`). What
   the operator owns vs. what the chart provides.
8. **Migrations** — `auto_migrate: true` on startup vs. running
   `knievel-cli migrate` out-of-band. When to pick which. Schema
   versioning and the additive-forever policy (`REQUIREMENTS.md`
   § 6.4) for the SQL surface.
9. **Upgrades** — rolling restart story. The advisory-lock leader
   re-elects automatically; the snapshot loader catches up via
   poll. Migration order (apply migrations before rolling pods).
   What an Aurora failover looks like in the metrics during an
   upgrade.
10. **Multi-region** — single-region in v0; the chart exposes
    `affinity` and `topologySpreadConstraints` for multi-AZ. Cross-
    region active-active is out of scope; an active-passive
    operator pattern is documented as a recipe.
11. **Observability setup** — Prometheus scrape config, OTel
    collector pipeline, Sentry DSN handling. The default-low
    cardinality policy and how to enable per-project metrics for an
    investigation.
12. **Alerts and dashboards** — per `REQUIREMENTS.md` § 9.3, the
    six operator-actionable thresholds. Sample PromQL alerts
    checked into `examples/observability/`.
13. **Runbooks (links)** — common incidents:
    - DB writer unreachable
    - Snapshot stale
    - Event channel saturated
    - Leader maintenance failure
    - JWKS endpoint unreachable
    - Connection-pool exhaustion
14. **Troubleshooting** — short FAQ keyed off `error.code` values
    callers are likely to see.
15. **Decommissioning** — what to drop and in what order. Schema +
    role + backups. The operator's checklist for a clean removal.

The deployment doc must be **self-contained for the happy path**.
A reader following it should never need to open `REQUIREMENTS.md`
to get a working install. Deep dives are linked, not inlined.

## 7. Supporting Docs

### 7.1 `CONTRIBUTING.md`

- Dev environment setup (Rust toolchain pin, `cargo nextest`,
  Postgres via testcontainers or compose).
- Branch policy (PR-based, single-track main, no long-lived
  branches).
- Commit conventions (subject line shape; body explains the why).
- Code review expectations (at least one maintainer; security-
  sensitive changes get two).
- Test expectations — link to `TESTING.md`. New project-scoped
  endpoints require a paired cross-tenant test (§ 6.5 of TESTING).
- Doc expectations — examples that ship in any doc must be tested
  in CI (§ 11 below).
- The platform-vs-consumer split: where consumer-specific examples
  are appropriate vs. where they aren't.

### 7.2 `SECURITY.md`

- How to report a vulnerability (email address; PGP key optional).
- Supported versions (the latest minor, plus the previous minor for
  6 months).
- Security model summary, three paragraphs:
  1. Trust boundary (server-to-server caller; no browser-direct in v0).
  2. Tenant isolation (RLS + query layer + CI gates;
     `REQUIREMENTS.md` § 7.1.1).
  3. Auth (`AUTH.md` summary; HMAC for events).
- Out-of-scope items (operator-owned: TLS termination, network
  policy, S3 bucket policy, OS hardening, backup encryption).
- Disclosure timeline (90-day default).

### 7.3 `CHANGELOG.md`

- [keep-a-changelog](https://keepachangelog.com) format.
- Per-release sections; oldest at the bottom.
- Generated from the release-tagging PR's release-notes section,
  not hand-edited per commit.
- Cross-references the OpenAPI spec version and the schema
  version.

### 7.4 `RELEASE_CHECKLIST.md`

- The checklist from `REQUIREMENTS.md` § 7.1.1 gate (3),
  rendered as a PR template.
- Enforced by CI per `TESTING.md` § 10.3.

### 7.5 `RELEASE_PLAYBOOK.md`

- What to do when a tag build fails halfway (image pushed, gem
  not).
- How to yank a bad release (gem yank; image immutable, push a
  patched tag).
- Rollback procedure for operators who already pulled.
- Referenced from `TESTING.md` § 12.9 and `DEPLOYMENT.md`.

## 8. Cross-Doc Conventions

These apply to every file in § 3.

### 8.1 Terminology

- **Capitalize** Org, Project, Advertiser, Campaign, Flight, Ad,
  Creative, CreativeTemplate, Site, Zone, Channel, Priority,
  AdType, AdLibraryItem when referring to the entity. Lowercase
  when used adjectivally ("the project's flights").
- **`organization` (RX-side concept) is never confused with
  `Organization` (knievel's top-level tenant).** When the doc
  needs to discuss both — only ever in `MIGRATION_*.md` files —
  qualify explicitly: "RX Organization" vs. "knievel Org".
- **`project_id`, `org_id`** in code blocks; "Project ID" / "Org
  ID" in prose. Snake-case in JSON; camelCase in TypeScript-shape
  examples. JSON wire format is camelCase per `API.md`.

### 8.2 Example values

Reused across every doc so a reader recognizes them:

| Concept | Canonical example |
|---|---|
| Org external ID | `scientist-com-prod` (in `MIGRATION_RX.md`); `acme-marketplace` (in platform docs) |
| Org ID | `org_AbCd...` |
| Project external ID | `tenant-acme` |
| Project ID | `pj_AbCd...` |
| Advertiser external ID | `advertiser-acme` |
| Site URL | `https://example.com` |
| HMAC default secret name | `KNIEVEL_HMAC_DEFAULT_SECRET` |
| OpenAPI tag (Decision) | `decisions` |
| Sample token | `kvl_prod_org_AbCd_8f2a...` |

`MIGRATION_<NAME>.md` files use consumer-specific names; platform
docs use generic ones (`acme-marketplace`, `pj_AbCd...`). This is
the platform-core vs. consumer-specific policy from
`REQUIREMENTS.md` § 8.

### 8.3 Cross-references

- Use `§ N.M` for section references within a doc.
- Use backticked filenames for cross-doc references:
  ``REQUIREMENTS.md § 7.1.1``.
- Hyperlinks are for **external** references (third-party tools,
  RFCs); do not hyperlink inside the repo. Plaintext path + section
  is more diff-friendly and survives a doc rename better than a
  rotting link.

### 8.4 Tone

- Spec-style: present tense, declarative.
- Honest about unverified numbers ("TARGET (unverified)" caveat;
  `REQUIREMENTS.md` § 9.1).
- Honest about gaps. The "What's deferred" / "What's not in v0" /
  "What tests don't catch" sections are mandatory in any doc that
  could mislead by omission.
- No marketing copy. The README's "why knievel" bullets are
  evidentiary, not promotional.

### 8.5 Code in docs

- Fenced code blocks always carry a language hint (`rust`, `sql`,
  `yaml`, `json`, `bash`, `dockerfile`).
- Examples are runnable. If a code block is illustrative-only,
  the prose says so explicitly.
- Long examples link to a file in `examples/` rather than inlining
  the full thing in the doc.

### 8.6 Diagrams

- ASCII for top-level component and data-flow diagrams (matches
  every existing platform doc).
- Mermaid `.mmd` for diagrams that exceed ~25 lines of ASCII;
  source committed to `docs/diagrams/`, rendered SVG checked in
  alongside so GitHub renders without a build step.
- Diagrams accompany prose, never replace it. A reader skimming
  text-only output (an accessibility tool, a terminal) must still
  get the message.

## 9. The OpenAPI Spec is the Wire Contract

`API.md` is a human map of the wire surface; `/openapi.json`
generated from the binary is the source of truth.

- The OpenAPI spec is committed at `openapi.yaml`, regenerated by
  `cargo xtask openapi`, drift-checked in CI (`TESTING.md` § 12.7).
- Generated client documentation (the Ruby gem's RDoc, etc.)
  derives from the spec and ships with each gem release.
- Examples in `API.md` are extracted and validated against the
  spec's schemas in CI (`TESTING.md` § 11.3) so the doc never
  shows a body the server would reject.

`API.md` consciously does **not** repeat what the spec encodes
mechanically (full schemas, response codes for trivial errors). It
focuses on the conventions a reader needs to integrate: pagination
shape, idempotency semantics, write-contract atomicity, error-body
shape. The OpenAPI spec covers the field-by-field detail.

## 10. What Each Doc Doesn't Do

A short anti-table to keep scope drift visible.

| Doc | Does not do |
|---|---|
| `README.md` | Schema reference, deployment depth, full feature matrix |
| `ARCHITECTURE.md` | Endpoint shapes, helm values, runbooks |
| `DEPLOYMENT.md` | Rationale for design choices, integration code samples |
| `API.md` | Field-level schema (the OpenAPI spec is canonical) |
| `AUTH.md` | Endpoint-level role minimums beyond what's needed for the auth model |
| `REPORTING.md` | dbt tutorials beyond knievel-specific patterns |
| `TESTING.md` | The actual workflow YAML (lives under `.github/`) |
| `REQUIREMENTS.md` | Beginner-friendly framing — it is the working spec |
| `MIGRATION_*.md` | Anything generic; consumer-specific only |

## 11. Maintenance and CI Gates

The doc surface stays honest because CI enforces it.

### 11.1 Existing gates that touch docs

- **OpenAPI spec drift** — `cargo xtask openapi --check`
  (`TESTING.md` § 12.7). The committed `openapi.yaml` and the
  binary's generated spec must match.
- **OpenAPI example validation** — example bodies in `API.md` are
  parsed as JSON against the spec's schemas (`TESTING.md` § 11.3).
- **Helm values lint** — `helm lint` and `kubeconform` on every
  `values.yaml` shipped under `examples/helm/`.
- **Acceptance suite** — ACC-01..14 walk the README quickstart
  end to end. A broken README quickstart fails CI.

### 11.2 New gates this plan adds

- **Markdown link checker** — [`lychee`](https://lychee.cli.rs)
  on every `.md` file in the repo, run in `xtask-lints`. External
  URLs are checked nightly only (rate-limited APIs make per-PR
  hostile); intra-repo paths are checked per PR.
- **Code-fence syntax check** — a small `xtask check-doc-fences`
  binary that walks every `.md` file, extracts every fenced code
  block by language tag, and runs the matching parser:
  - `rust` → `syn::parse_file` (must parse; not type-checked).
  - `yaml` → `serde_yaml::from_str::<serde_yaml::Value>`.
  - `json` → `serde_json::from_str::<serde_json::Value>`.
  - `sql` → `pg_query::parse` (syntactic only).
  - `bash` → `bashlex` (Python; vendored once or skipped).
  Blocks tagged `rust,ignore` or `yaml,ignore` are skipped — same
  semantics as `rustdoc`. The gate makes silent doc rot loud.
- **Doc-table coverage** — for every endpoint in the OpenAPI
  spec, a corresponding row must exist in `API.md`'s resource
  tables. Enforced by `xtask check-api-doc`.
- **Quickstart freshness** — the README quickstart is extracted
  and replayed as part of acceptance ACC-01..14 (`TESTING.md` §
  7.1). If a step breaks, the README is wrong.

### 11.3 Out of CI but in the discipline

- **"Last reviewed" markers** — explicitly *not* added. Per-doc
  staleness markers rot more visibly than the docs themselves and
  give false confidence. Doc freshness is a property of the CI
  gates above; if a doc passes, it's current enough to ship.
- **Doc PR template** — for any PR that adds or removes a feature,
  the PR description must list the docs affected. Reviewer enforces
  by inspection, not by automation.

## 12. Rollout Order

Docs are produced in the order a reader meets them, not the order
they're easiest to write. Each step lands in a separate PR.

1. `README.md` (this plan's primary deliverable). Quickstart
   compose stack lands at the same time so the README's commands
   work on day one.
2. `ARCHITECTURE.md`. Reuses the diagrams from `REQUIREMENTS.md`
   § 3 and § 7, refactored for an evaluator audience.
3. `DEPLOYMENT.md`. Lands with `examples/helm/` and
   `examples/compose/` populated; the runbook section initially
   just links to short stubs that get fleshed out as incidents are
   actually seen.
4. `CONTRIBUTING.md`, `SECURITY.md`. Short; they unblock external
   contributions.
5. `RELEASE_CHECKLIST.md`. Required for the first tagged release.
6. `CHANGELOG.md`. Created at `v0.1.0`; `Unreleased` section
   maintained from then on.
7. `RELEASE_PLAYBOOK.md`. After the first real-world release
   reveals which sections matter.
8. `xtask check-doc-fences`, `xtask check-api-doc`, `lychee`
   integration. Land alongside the docs they protect.

A README without a working quickstart is worse than no README; a
quickstart without acceptance coverage rots in weeks. The order
above keeps the published docs honest at every step.

## 13. Out of Scope

- **Tutorial-style "build a real app" walkthroughs.** Knievel is
  the platform; a tutorial is a consumer concern. The Ruby gem
  may ship one in its own repo.
- **Video docs / asciinema casts.** Maintenance overhead too high
  for a v0.
- **A separate static doc site** (Hugo / mdBook / Docusaurus).
  GitHub renders the existing `.md` files well; a static site
  adds a build step for marginal gain. Revisit when the doc count
  exceeds ~25 files.
- **Localization.** English-only at v0.
- **Per-language client docs in this repo.** Each generated
  client owns its own README; this repo links out.

## References

- [keep-a-changelog](https://keepachangelog.com) — `CHANGELOG.md` format
- [GitHub-flavored Markdown](https://github.github.com/gfm/) — rendering target
- [`lychee`](https://lychee.cli.rs) — link checker
- [`syn`](https://docs.rs/syn/) — Rust source parsing for fence checks
- [`pg_query`](https://docs.rs/pg_query/) — PostgreSQL parser for SQL fence checks
- [Mermaid](https://mermaid.js.org) — diagram source format for non-ASCII diagrams
- [OpenAPI 3.1 specification](https://spec.openapis.org/oas/v3.1.0)
