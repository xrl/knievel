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
| Auth | **`react-oidc-context`** (wraps **`oidc-client-ts`**) | Authorization Code + PKCE against Keycloak; silent refresh; no client secret in the SPA. Paste-a-token fallback for dev / no-Keycloak environments. |
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
      AuthProvider.tsx    # `react-oidc-context` <AuthProvider>
      RequireAuth.tsx
      PasteTokenLogin.tsx # fallback when oidc.require_oidc is false
      runtimeConfig.ts    # fetches /admin/config.json on boot
    routes/
      __root.tsx
      projects/
        index.tsx
        $project_id.tsx
        $project_id.advertisers.tsx
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

**Primary flow: OIDC Authorization Code + PKCE against Keycloak.**
Humans sign in to Keycloak; the SPA receives a short-lived
access token (a JWT carrying the `knievel` claim); every API
call presents it as `Authorization: Bearer <jwt>`. The API
already validates these JWTs unchanged via the existing JWKS
machinery — no admin-only endpoints, no custom session backend,
no cookies. See `AUTH.md` "Keycloak Setup — Human Admin UI
(PKCE)" for the canonical contract; this section covers the SPA
side.

### Library + flow

- **`react-oidc-context`** (wraps `oidc-client-ts`) drives the
  dance. `<AuthProvider>` at the route root; `useAuth()` hook
  exposes `{ user, signinRedirect, signoutRedirect, ... }`.
- **Public client + PKCE.** No client secret in the bundle; the
  `code_verifier` is generated per-login and held in
  `sessionStorage` only until the callback completes.
- **Token storage.** Access token + refresh token live in
  memory inside `UserManager`, with `sessionStorage`
  persistence so a tab reload doesn't force re-auth. Closing
  the tab clears them; explicit logout calls
  `signoutRedirect()` which hits Keycloak's
  `end_session_endpoint`.
- **Silent refresh.** `oidc-client-ts` auto-refreshes the
  access token via the refresh token before expiry; on
  refresh failure (revoked session, Keycloak unreachable) the
  fetch wrapper redirects to `/oidc/login`.
- **`RequireAuth` route guard** wraps every protected route;
  unauthenticated → `signinRedirect()`. The fetch wrapper
  retries once on `401` after a silent refresh attempt.
- **Role-claim-driven UI gating.** The `knievel.role` from the
  JWT is read in the SPA to hide admin-only surfaces from
  `editor` / `reader` users. **Not a security boundary** —
  knievel still enforces every authz check server-side; this
  is purely cosmetic.

### Routes

- `/oidc/login` — initiates `signinRedirect()` and returns null.
- `/oidc/callback` — completes `signinRedirectCallback()`,
  redirects to the deep link the user originally requested.
- `/oidc/logout` — calls `signoutRedirect()`.

### Runtime config (one bundle, multiple envs)

The SPA can't bake the Keycloak issuer / client-id into the
build because we want a single artifact to deploy across
staging / prod / dev. On boot the SPA fetches
`GET /admin/config.json`, served by the API from the same
origin:

```json
{
  "oidc": {
    "issuer":    "https://keycloak.scientist.com/realms/scientist",
    "client_id": "knievel-admin-ui-prod",
    "scopes":    ["openid", "profile", "knievel"],
    "require_oidc": true
  }
}
```

The API constructs this payload from the new `admin_ui:` config
section (planned in Phase 7.4 alongside the
`StaticFilesEndpoint` mount; see `AUTH.md` "Knievel-side
configuration"). Empty `oidc.issuer` means OIDC is disabled and
the UI falls through to the paste-a-token form.

### Paste-a-token fallback

Kept as a deliberate first-class fallback, not a dev-only
hack:

- **Dev / `docker compose up`** — the seed sidecar mints an Org
  Editor opaque token (`AUTH.md` "Local Development"), no
  Keycloak in the picture. Paste-token login uses it
  unchanged.
- **Bootstrap** — bring up a knievel cluster before Keycloak
  is provisioned.
- **Keycloak outage / DR** — operators with a break-glass
  opaque token in their secret store stay unblocked.
- **CI smoke tests** — deterministic credential, no IdP
  dependency.

The fallback is hidden when `admin_ui.oidc.require_oidc: true`
(default in prod). The form accepts a `kvl_*` token, validates
it against a "who am I" endpoint, and stores it in
`sessionStorage` exactly as the JWT path does. From the API's
perspective the request is identical to any other Bearer call;
the UI just constructs the header from a different source.

### What's intentionally not here

- **No BFF / no admin-session endpoint.** The original sketch
  had `POST /v1/admin/sessions` backed by argon2id user
  credentials; replaced by Keycloak-as-the-IdP, which deletes
  custom auth code we'd have to maintain and gets MFA / SSO /
  password rotation / session policy free.
- **No cookies for app auth.** Keeps CORS at
  `allow_credentials: false` and removes the cookie-CSRF
  surface. Refresh tokens go through `oidc-client-ts` directly
  to Keycloak's token endpoint, never via knievel.
- **No tokens baked into the bundle.** Build artifacts contain
  no secrets. The runtime config only exposes public OIDC
  metadata (issuer URL + public client ID).

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
- **Phase 7.4** — Auth (OIDC PKCE primary, paste-token
  fallback): `react-oidc-context` wired against Keycloak,
  `/oidc/login` + `/oidc/callback` + `/oidc/logout` routes,
  `RequireAuth`, 401-aware fetch wrapper with silent refresh,
  paste-a-token form behind a runtime-config flag.
  Adds the `admin_ui:` config block on the API side and
  `GET /admin/config.json` to surface OIDC metadata to the
  bundle. Smallest "who am I" endpoint added on the API side
  if it doesn't exist yet (validates both JWT and opaque
  Bearer paths identically). Refs: `AUTH.md` "Keycloak Setup
  — Human Admin UI (PKCE)."
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
- **Phase 7.9** — OIDC hardening: end-session integration with
  Keycloak's `end_session_endpoint`, session-timeout UX
  (idle-warning modal + grace refresh), role-claim-driven UI
  gating (hide admin-only surfaces from `editor` / `reader`
  claim values; not a security boundary — knievel still
  enforces server-side), and a documented Keycloak admin-UI
  client setup verified end-to-end against a real realm.
  Refs: `AUTH.md` "Keycloak Setup — Human Admin UI (PKCE)."
- **Phase 7.10** — Playwright e2e in `nightly.yml`, bundle-size
  budgets, accessibility sweep.

Numbering is provisional — the actual phase number lands when
this gets merged into `PHASES.md`. The point is the dependency
order: skeleton → CORS → codegen → auth → views → editing →
reporting → OIDC hardening → polish.

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
