# UI.md

Plan for the knievel admin audit UI. Companion to `API.md`,
`AUTH.md`, and `PHASES.md`. Scope is **the admin console** —
operator-facing tooling to audit and manage the state of the ad
server. Self-serve advertiser/publisher consoles are out of scope
for v0 and will be a separate deliverable.

## Goals

- An in-browser console for operators to inspect projects,
  advertisers, campaigns, flights, ads, creatives, sites, zones,
  taxonomy, and (later) reporting and event flow.
- Uses the same OpenAPI surface as every other client. No
  privileged side channels, no admin-only endpoints that bypass
  the public contract. If the UI needs it, the API exposes it.
- Drift-checked against `openapi.yaml` in the same CI run as the
  Rust server. The spec is the contract; the UI's typed client
  is regenerated from it.

## Non-goals (v0)

- A self-serve advertiser/publisher console.
- Mobile-responsive layouts. Desktop-first; tablet works,
  phones are not a target.
- Real-time dashboards beyond what `/events` and reporting
  endpoints expose. No WebSocket push from the API in v0.
- An embedded editor for `creative_template.schema` JSON Schema.
  Raw JSON textarea with validation is acceptable for v0.

## Stack

| Concern | Choice | Why |
| --- | --- | --- |
| Framework | **React 18 + TypeScript** | Largest off-the-shelf component & form ecosystem for an audit-style console. |
| Build | **Vite** | Fast dev server, first-class TS, no Next.js server runtime to operate. |
| Routing | **TanStack Router** | Type-safe routes pair well with the typed API client. |
| Data layer | **TanStack Query** + **`openapi-fetch`** (typed via **`openapi-typescript`**) | Codegen consumes `openapi.yaml` directly; no hand-written types. Cache + invalidation handled by Query. |
| Forms | **react-hook-form** + **zod** | Schemas derived from generated TS types; minimal runtime overhead. |
| Components | **Mantine v7** (tables, modals, forms, dates) | Batteries-included audit UI primitives. shadcn/ui is the fallback if Mantine bloat becomes a problem. |
| Tables | **TanStack Table** under Mantine wrappers | Server-side pagination/sort/filter against our cursor envelope. |
| Lint/format | **eslint** (typescript-eslint, react-hooks) + **prettier** | Match Rust gates' "fail CI on warnings" stance. |
| Tests | **vitest** + **@testing-library/react**; **Playwright** for end-to-end | Vitest mirrors `cargo test` ergonomics. Playwright runs against a real backend in CI. |
| Package manager | **pnpm** | Lockfile determinism; faster than npm in CI. |

Hard constraints:

- **No Next.js, no SSR, no RSC.** This is a single-page admin
  app talking to a Rust API. SSR adds an extra runtime to
  operate and buys us nothing here.
- **No cookies for app auth.** Bearer tokens in
  `Authorization` headers; storage discussed under "Auth"
  below. Keeps CORS simple (`allow_credentials: false`).
- **No third-party telemetry SDKs in v0.** Operator console;
  we already get server-side request IDs via
  `observability.rs`.

## Repo layout

In-tree under `web/admin/`. The admin UI moves at the same
cadence as the OpenAPI surface during Phase 3+; coupling the
codegen step to the same CI run as `xtask openapi --check`
catches drift in one place.

```
web/admin/
  package.json
  pnpm-lock.yaml
  tsconfig.json
  vite.config.ts
  index.html
  src/
    api/
      generated.ts        # `openapi-typescript` output; checked in
      client.ts           # `openapi-fetch` instance, auth header
    auth/
      session.ts          # token storage + login/logout
      RequireAuth.tsx
    routes/
      __root.tsx
      projects/
        index.tsx
        $projectId.tsx
        $projectId.advertisers.tsx
        ...
    components/
      DataTable.tsx       # Mantine + TanStack Table wrapper
      JsonView.tsx        # read-only JSON inspector for audit
      ...
    main.tsx
    app.tsx
  tests/
    e2e/                  # Playwright specs
  README.md
```

When/if the UI graduates to its own deploy cadence (post-Phase
3, once the spec stabilizes and the deploy story diverges),
extract to a sibling repo. Until then, in-tree wins for the same
reasons we keep `testlib/` and `xtask/` in-tree: one PR, one CI
run, no version-skew window.

## OpenAPI codegen

A new xtask subcommand:

```bash
cargo xtask ui-client                # regenerate web/admin/src/api/generated.ts
cargo xtask ui-client --check        # CI gate: fail if generated.ts is stale
```

Implementation (`xtask/src/ui_client.rs`):

1. Shell out to `pnpm --dir web/admin exec openapi-typescript
   ../../openapi.yaml -o src/api/generated.ts`.
2. In `--check` mode, run the same command into a temp file and
   `diff` against the committed copy. Mirror
   `xtask/src/openapi.rs` exactly — same exit-code shape, same
   error messaging.

Wire `ui-client --check` into `.github/workflows/ci.yml` as a
peer of `openapi --check`. Both gates share the same Node setup
step (a new `.github/actions/node-setup/` composite, mirroring
`rust-setup`).

The generated file is **checked in** (not gitignored) for the
same reason `openapi.yaml` is: it's part of the contract surface
and reviewers should see when it changes.

## Auth

The admin UI authenticates with the same opaque bearer tokens as
every other client (`AUTH.md` § 2). For v0:

- **Login screen** accepts a token string (operator pastes from
  `tokens` API output or from their secret-store CLI) and
  validates it by calling `GET /v1/orgs/me` (or whatever the
  smallest "who am I" endpoint becomes — track in `PHASES.md`).
- **Storage**: `sessionStorage`, not `localStorage`. Survives
  tab refresh, dies on tab close. Acknowledge the XSS-exfil
  risk in the UI README; mitigations come with the eventual
  admin-session endpoint (next bullet).
- **Roadmap**: a dedicated admin-session endpoint
  (`POST /v1/admin/sessions`) backed by argon2id-hashed user
  credentials, returning a short-lived bearer token + refresh
  semantics. Token rotation handled client-side via TanStack
  Query's retry-on-401. Track as a Phase 4 task; not blocking
  for the read-only audit UI.

The UI never embeds tokens at build time. There is no "service
account baked into the SPA."

## CORS

Yes — required, because the dev workflow has the UI on
`http://localhost:5173` (Vite) talking to the API on
`http://localhost:8080`, and prod can plausibly run the UI on a
different origin from the API (e.g. `admin.example.com` →
`api.example.com`). Without CORS the browser blocks every
preflighted request (anything with `Authorization` is
preflighted).

### API-side changes

Add to `src/config.rs::ApiConfig`:

```rust,ignore
/// Origins permitted to make cross-origin requests. Empty
/// (default) disables the CORS layer entirely — same-origin
/// only. Each entry is a literal origin like
/// `https://admin.example.com`; wildcards are not supported.
#[serde(default)]
pub allowed_origins: Vec<String>,
```

In `src/server.rs`, when `cfg.api.allowed_origins` is non-empty,
wrap the route with `poem::middleware::Cors`:

- `allow_origins`: from config.
- `allow_methods`: `GET, POST, PATCH, DELETE, OPTIONS`.
- `allow_headers`: `Authorization, Content-Type,
  Idempotency-Key, If-Match, X-Request-Id`.
- `expose_headers`: `ETag, Location, X-Request-Id,
  X-Idempotency-Replayed`. (Anything custom that the UI needs
  to read off responses.)
- `allow_credentials`: **false**. We use bearer tokens, not
  cookies; staying credentials-less keeps the wildcard escape
  hatch open and removes a cookie-CSRF surface.
- `max_age`: 600 seconds. Cuts preflight chatter for the SPA's
  navigation patterns.

### Dev defaults

`config.example.yaml` gains:

```yaml
api:
  allowed_origins:
    - http://localhost:5173    # vite dev
    - http://127.0.0.1:5173
```

Vite's dev server can also proxy `/v1/*` to `localhost:8080`
to side-step CORS during local development; we keep the CORS
layer enabled anyway so the dev path matches prod. (Proxy is
configured in `vite.config.ts` but is opt-in via env var, not
the default.)

### Prod guidance

- Same-origin deploy (UI served behind the same hostname as
  the API via reverse proxy or a poem static-files route):
  leave `allowed_origins` empty. Simplest, no CORS layer
  installed, no preflight overhead.
- Split-origin deploy: list each admin origin explicitly. Do
  not use `*`; bearer tokens shouldn't fly to arbitrary
  origins even with `allow_credentials: false`.

### Test coverage

A new `tests/api_cors.rs` slice asserts:

1. Empty `allowed_origins` → no `Access-Control-Allow-Origin`
   header on responses; OPTIONS returns 404 (poem's default).
2. Configured origin echoes back on `Access-Control-Allow-Origin`
   for matching `Origin:` requests; non-matching origins get no
   header.
3. Preflight (`OPTIONS` with `Access-Control-Request-Method`)
   returns 204 with the expected `Allow-Methods`,
   `Allow-Headers`, `Max-Age`.
4. `Authorization` is in the allowed-headers reflection.

Add the slice's binary to `cargo xtask test-shape` so it runs
in CI alongside the rest of `tests/api_*.rs`.

## Dev workflow

```bash
# One-time
cd web/admin && pnpm install

# Two terminals (or pnpm dev runs both via concurrently)
cargo run                                    # API on :8080
cd web/admin && pnpm dev                     # UI on :5173

# Regenerate the typed client after touching openapi.yaml
cargo xtask ui-client

# Per-PR gates locally
cd web/admin && pnpm lint && pnpm typecheck && pnpm test
cargo xtask ui-client --check
```

## CI integration

New jobs in `.github/workflows/ci.yml`, gated on changes to
`web/admin/**` or `openapi.yaml`:

- `ui-typecheck` — `pnpm typecheck`
- `ui-lint` — `pnpm lint`
- `ui-test` — `pnpm test --run`
- `ui-client-drift` — `cargo xtask ui-client --check`
- `ui-build` — `pnpm build` (catches dead imports, type errors
  the dev server tolerates, and bundle-size regressions via a
  size-limit check)
- `e2e` (nightly only, in `nightly.yml`) — Playwright against
  a Postgres-backed compose stack

A new `.github/actions/node-setup/` composite handles pnpm +
Node version pinning, mirroring `rust-setup`. Same caveat
applies: callers must `actions/checkout@v4` before `uses:
./.github/actions/node-setup` (gotcha #6 in `CLAUDE.md`).

## Phasing

The UI is a new top-level workstream; it doesn't displace any
Phase 3 task. Suggested decomposition once Phase 3 closes the
hot-path rail (3.21):

- **Phase 7.1** — Repo skeleton: `web/admin/` scaffold, vite +
  TS + Mantine, ESLint/Prettier, vitest harness, README. No
  routes yet beyond a placeholder.
- **Phase 7.2** — CORS rail: `ApiConfig.allowed_origins`,
  poem `Cors` middleware, `tests/api_cors.rs`, config example
  update.
- **Phase 7.3** — Codegen rail: `xtask ui-client [--check]`,
  `node-setup` composite action, CI wiring, generated
  `src/api/generated.ts` checked in.
- **Phase 7.4** — Auth: login screen, session storage,
  `RequireAuth`, 401-aware fetch wrapper, smallest "who am I"
  endpoint on the API side if it doesn't exist yet.
- **Phase 7.5** — Org/project browser: list + detail for
  `/v1/orgs` and `/v1/projects/:project_id`. First end-to-end
  slice; proves the typed client + Query + Router stack.
- **Phase 7.6** — Resource audit views: read-only tables for
  advertisers, campaigns, flights, ads, creatives, sites,
  zones, taxonomy. Cursor pagination + filter UI per the API
  envelope.
- **Phase 7.7** — Editing: PATCH/POST forms with
  react-hook-form + zod, idempotency-key handling, optimistic
  invalidation. Feature-flag rollout per resource.
- **Phase 7.8** — Reporting + event-flow inspector: charts on
  rollups, a tail view over `/events` (poll-based for v0).
- **Phase 7.9** — Admin-session endpoint: short-lived tokens,
  refresh handling, retire the paste-a-token login.
- **Phase 7.10** — Playwright e2e in `nightly.yml`, bundle-size
  budgets, accessibility sweep.

Numbering is provisional — the actual phase number lands when
this gets merged into `PHASES.md`. The point is the dependency
order: skeleton → CORS → codegen → auth → views → editing →
reporting → admin-session → polish.

## Open questions

- **Static hosting in prod.** Two options:
  1. Serve the built bundle from poem via a `StaticFilesEndpoint`
     mounted at `/admin/`. One deploy artifact, same-origin,
     no CORS needed. Couples UI deploys to API deploys.
  2. Separate static host (S3 + CloudFront, or a dedicated
     pod). Decoupled cadence, requires CORS, requires a
     separate deploy pipeline.
  Lean toward (1) for v0 to keep the deploy story simple;
  revisit when the UI deploy cadence diverges from the API.
- **`creative_template.schema` editor.** Raw textarea + JSON
  parse for v0. Monaco-based editor with JSON-Schema-aware
  IntelliSense is appealing but adds ~2 MB to the bundle —
  defer until a real operator complaint motivates it.
- **i18n.** Not in v0. English-only. Plan: when needed, lift
  strings into a `messages/` directory and pick a library
  then; don't pre-build a translation pipeline.
- **Dark mode.** Mantine ships it free; enable from day one.
  No additional spec needed.

## Cross-references

- `API.md` — endpoint shapes the UI consumes.
- `AUTH.md` — bearer token semantics; the UI honors them
  unchanged.
- `REQUIREMENTS.md` § 6 — OpenAPI 3.0 reality (gotcha #4 in
  `CLAUDE.md`); `openapi-typescript` handles 3.0 fine.
- `TESTING.md` § 12.4 — CI DAG; new UI jobs slot in as
  conditional peers.
- `PHASES.md` — phase numbering and the canonical task list.
