# Knievel Authentication

Knievel supports two coexisting credential types on its Management and
Decision endpoints. Either or both can be enabled per deployment.

| Mode | Format | When to use |
|---|---|---|
| **Opaque token** | `kvl_<env>_<scope>_<random>` | Bootstrap, deployments without an IdP, dev environments, fallback when the admin UI can't reach Keycloak. |
| **JWT** | Three base64url segments separated by `.` | When a Keycloak / OIDC provider is in place. Service-to-service via `client_credentials`; human admins via Authorization Code + PKCE from the admin UI. Identity stays in the IdP; knievel just validates. |

Detection is trivial: the prefix `kvl_` is reserved for opaque tokens;
anything else is parsed as a JWT.

Event endpoints (`/e/...`) remain unauthenticated; they're protected by
HMAC signatures in the URL. System endpoints (`/healthz`, `/readyz`,
`/metrics`, `/openapi.json`, `/version`) are unauthenticated by default
and typically restricted via reverse proxy.

## Opaque Tokens

Recap of what's described in `REQUIREMENTS.md` §4.3:

- Format: `kvl_<env>_<scope>_<random>`, e.g. `kvl_prod_org_AbCd_8f2a...`.
- Stored argon2id-hashed; never recoverable after creation.
- Scoped to an Org or a Project, with a role (`org-owner`, `org-admin`,
  `admin`, `editor`, `reader`).
- Minted via `POST /v1/orgs/{org_id}/tokens`. Revocable. Last-used
  timestamp tracked.

Use opaque tokens when: there's no IdP available; you're bootstrapping
a new deployment before Keycloak is wired up; you need fine-grained
revocation without coordinating with an IdP.

## JWTs

Knievel validates JWTs **statelessly** against issuer JWKS endpoints.
No introspection round-trip per request; no DB lookup. The standard
OAuth2/OIDC trade-off: revocation is bounded by token TTL.

### Validation rules

1. Header must specify a `kid` and an algorithm in the per-issuer
   allow-list (default: `RS256`, `ES256`, `PS256`). `alg: none` and
   HMAC algorithms (`HS256` etc.) are rejected unconditionally.
2. Signature must verify against the JWK matching `kid` from the
   issuer's JWKS.
3. Standard claims:
   - `iss` must match a configured issuer.
   - `aud` must contain the configured audience for that issuer.
   - `exp` must be in the future (with a 30 s clock-skew tolerance).
   - `nbf` and `iat`, when present, must not be in the future
     (same tolerance).
4. The `knievel` claim (or operator-configured equivalent) must be
   present and well-formed.

Failures return `401` with `code: invalid_token` and a per-failure
detail (`signature`, `expired`, `audience`, `issuer`, `claim_missing`,
`claim_malformed`).

### The `knievel` claim

A single namespaced custom claim carries authorization context:

```json
{
  "iss": "https://keycloak.scientist.com/realms/scientist",
  "aud": "knievel",
  "sub": "service-account-rx-prod",
  "iat": 1717000000,
  "exp": 1717003600,
  "knievel": {
    "scope":      "org",
    "org_id":     "scientist-com-prod",
    "role":       "editor"
  }
}
```

Fields:

| Field | Required | Notes |
|---|---|---|
| `scope` | yes | `org` or `project`. |
| `org_id` | yes | Knievel Org ID or `external_id`. |
| `project_id` | when `scope=project` | Knievel Project ID or `external_id`. |
| `role` | yes | One of `org-owner`, `org-admin`, `admin`, `editor`, `reader` (see role table below). |

The claim path is configurable (`auth.jwt.issuers[].claim`). Default
is `knievel`. Some IdPs prefer flat claim namespaces (`knievel_scope`,
`knievel_org_id`, …); knievel supports a flat-claim mode by setting
`claim_format: flat` on the issuer.

### Role mapping

The `role` value in the JWT maps directly to knievel's existing role
enum (`REQUIREMENTS.md` §4.3). `scope: org` requires an org-level
role; `scope: project` requires a project-level role:

| `scope` | Allowed `role` values |
|---|---|
| `org` | `org-owner`, `org-admin` (full org auth) — or `admin` / `editor` / `reader` (applied as a project-level role to every project in the org). |
| `project` | `admin`, `editor`, `reader`. |

An org-scoped `editor` JWT is exactly equivalent to an Org Editor
opaque token: it can address any Project in the Org via the path,
performs CRUD on resources, cannot manage members or tokens.

### JWKS handling

For each configured issuer, knievel:

1. Discovers `jwks_uri` from
   `{issuer}/.well-known/openid-configuration` (skipped if explicitly
   overridden in config).
2. Fetches the JWKS, indexes by `kid`, caches with TTL (default 1 h).
3. On validation, looks up the key by `kid`. **Cache miss triggers a
   refresh** — supports key rotation without downtime, since Keycloak
   adds new keys before retiring old ones.
4. Re-fetches on TTL expiry regardless of cache hits, so retired keys
   eventually fall out of cache.

Algorithm allow-list per issuer prevents downgrade attacks; HS256 is
never accepted (would require shipping a shared secret).

### Configuration

```yaml
auth:
  modes: [opaque, jwt]    # either, or both
  jwt:
    issuers:
      - issuer:    https://keycloak.scientist.com/realms/scientist
        audience:  knievel
        # JWKS auto-discovered via /.well-known/openid-configuration.
        # Override only when discovery isn't available:
        jwks_url:  ""
        algorithms: [RS256, ES256]
        # Where the knievel claim lives in the JWT.
        claim:        knievel
        claim_format: object   # or "flat"
        cache_ttl_seconds: 3600
        clock_skew_seconds: 30
```

Multiple issuers are supported for federation (different envs,
different realms, gradual migration between IdPs). The first issuer
whose `iss` claim matches is used.

## Keycloak Setup — Service Accounts (`client_credentials`)

For a Keycloak server at `https://keycloak.scientist.com`. This
section covers **service-to-service** authentication: a backend
client (e.g. a Rails app) obtaining tokens to call knievel.
Human admins authenticating into the admin UI use a separate
client described in the next section.

### 1. Create a client (per knievel environment)

Per knievel environment (prod, staging), create a Keycloak client:

- **Client ID**: `knievel-prod` (etc.)
- **Client authentication**: ON (confidential client).
- **Authentication flow**: leave OIDC defaults.
- **Service account roles**: ON.
- **Standard flow** / **direct access grants**: OFF (we only want
  service accounts via `client_credentials`).

Knievel's calling app obtains tokens via:

```
POST /realms/scientist/protocol/openid-connect/token
grant_type=client_credentials
client_id=knievel-prod
client_secret=<from keycloak>
audience=knievel
```

### 2. Add a hardcoded-claim protocol mapper

On the same client, create a *Hardcoded claim* mapper to inject the
`knievel` claim:

- **Mapper Type**: Hardcoded claim
- **Token Claim Name**: `knievel`
- **Claim value**: a JSON object —
  ```json
  {"scope": "org", "org_id": "scientist-com-prod", "role": "editor"}
  ```
- **Claim JSON Type**: JSON
- **Add to access token**: ON
- **Add to ID token**: OFF
- **Add to userinfo**: OFF

This bakes the org and role into every token the client issues, so
knievel never has to guess.

### 3. Set the audience

Keycloak's default audience for client_credentials tokens isn't always
`knievel`. Add an *Audience* protocol mapper:

- **Mapper Type**: Audience
- **Included Custom Audience**: `knievel`
- **Add to access token**: ON

### 4. Map Keycloak realm/client roles → knievel roles (optional)

For more dynamic role assignment than a hardcoded mapper, define
Keycloak client roles (`knievel-editor`, `knievel-reader`) and a
*Realm role mapping* that injects the appropriate `role` value into
the `knievel` claim based on which roles the service account holds.
Useful when one Keycloak realm serves multiple knievel orgs and you
want to grant access without editing client mappers.

### 5. Knievel-side configuration

```yaml
auth:
  modes: [jwt]    # opaque tokens off; Keycloak is the only source
  jwt:
    issuers:
      - issuer:    https://keycloak.scientist.com/realms/scientist
        audience:  knievel
        algorithms: [RS256]
        claim:     knievel
```

That's it. Knievel discovers the JWKS, validates incoming JWTs, and
maps the `knievel` claim to its internal `Principal`.

## Keycloak Setup — Human Admin UI (PKCE)

For human operators authenticating into the admin UI (`UI.md`).
Uses **OpenID Connect Authorization Code with PKCE** against a
**public client** — no client secret in the SPA, no UI-side
backend, all in-browser. The same JWKS validation the backend
already runs for service-account JWTs handles the resulting
access tokens unchanged; only the upstream issuance flow differs.

The admin UI is the only first-party human surface in v0; if you
add another browser-side client (a separate ops console, etc.)
follow the same pattern with its own client ID.

### 1. Create the admin-UI client (per knievel environment)

Per environment, create a second Keycloak client distinct from
the service-account one above:

- **Client ID**: `knievel-admin-ui-prod` (etc.).
- **Client authentication**: OFF (public client — PKCE replaces
  the client secret, since a SPA can't keep one).
- **Authentication flow**: Standard flow ON, Direct access grants
  OFF, Service accounts OFF, Implicit flow OFF.
- **Proof Key for Code Exchange (PKCE)**: required, method
  `S256`. Set "Advanced → Proof Key for Code Exchange Code
  Challenge Method" to `S256`.
- **Valid redirect URIs** (the SPA mounts under `/admin/`, so the
  callback route resolves under that basepath):
  - `https://admin.knievel.example.com/admin/oidc/callback`
  - `http://localhost:5173/admin/oidc/callback` (dev)
- **Valid post-logout redirect URIs**: the admin UI's root URL,
  including the `/admin/` mount (e.g. `https://admin.knievel.example.com/admin/`).
- **Web origins**: same hosts as the redirect URIs (Keycloak
  uses this for its own CORS allow-list on the token endpoint).
- **Access token lifespan**: 15 minutes (knievel's default
  validation tolerance is fine with this).
- **Refresh tokens**: ON. SSO Session Idle / Max set per your
  org policy; the UI uses the refresh token for silent renewal
  via `oidc-client-ts`.

### 2. Map user identity → the `knievel` claim

The service-account flow uses a hardcoded-claim mapper because
every token from that client represents the same principal. For
humans, the `knievel` claim has to vary per user. Two patterns,
pick one:

**a. Group-membership mapper (multi-org Keycloak realms).**
Define groups whose path encodes org and role:

```
/knievel/scientist-com-prod/editor
/knievel/scientist-com-prod/admin
/knievel/example-co-staging/reader
```

Add a *Script Mapper* (or *Group Membership* + a script) on the
`knievel-admin-ui-prod` client that emits the `knievel` claim by
parsing the first matching group path. Result on the access
token:

```json
{
  "knievel": {
    "scope":  "org",
    "org_id": "scientist-com-prod",
    "role":   "editor"
  }
}
```

A user in multiple `/knievel/*` groups gets the highest role
across them; cross-org membership is allowed but the claim
carries one org per token (the active one — selectable in the UI
via a Keycloak `prompt=login` re-auth, or `acr_values` if you
need it deterministic).

**b. User-attribute mapper (single-org installs).**
Set `knievel_org_id` and `knievel_role` as user attributes; a
*User Attribute* mapper assembles them into the `knievel` claim
object. Simpler than the group approach, but every user has to
be tagged manually.

Either way, the claim shape on the wire is the same one knievel
already validates — see "The `knievel` claim" above. No
knievel-side changes from the service-account path.

### 3. Add the audience

Same as the service-account flow: an *Audience* protocol mapper
to ensure `aud` contains `knievel`. Without it, knievel rejects
the token at boot-validated audience check.

### 4. Knievel-side configuration

No new config on the API side. The same `auth.jwt.issuers[]`
entry that validates service-account tokens validates human
tokens — same issuer, same JWKS, same algorithms, same claim
path. The only addition is a section the **admin UI** reads at
runtime:

```yaml
admin_ui:
  oidc:
    issuer:    https://keycloak.scientist.com/realms/scientist
    client_id: knievel-admin-ui-prod
    scopes:    [openid, profile, knievel]
    # When false (default false in dev, true in prod), the UI
    # hides the paste-a-token fallback login.
    require_oidc: true
```

This block is served to the SPA at boot via a small
`GET /admin/config.json` endpoint (one bundle, multiple envs;
see `UI.md` "Auth"). It's not consumed by the API itself.

### 5. UI flow

`oidc-client-ts` (wrapped by `react-oidc-context`) drives the
dance:

1. Unauthenticated user hits any route → `RequireAuth` redirects
   to `/oidc/login`, which calls `userManager.signinRedirect()`.
2. Browser → Keycloak `/protocol/openid-connect/auth?...&code_challenge=...&code_challenge_method=S256`.
3. User authenticates (with whatever Keycloak has set up — MFA,
   SSO, social, all upstream concerns).
4. Keycloak → `/oidc/callback?code=...`. `userManager.signinRedirectCallback()`
   exchanges the code + PKCE verifier for `{access_token, id_token,
   refresh_token}`.
5. The fetch wrapper attaches `Authorization: Bearer <access_token>`
   to every API call.
6. Silent refresh: when the access token approaches expiry,
   `oidc-client-ts` posts the refresh token to Keycloak's token
   endpoint and replaces the access token in memory.
7. Logout: UI calls `userManager.signoutRedirect()`, which hits
   Keycloak's `end_session_endpoint` and returns to the
   post-logout redirect URI.

Tokens (access + refresh) live in memory inside the
`UserManager`, with optional `sessionStorage` persistence so a
tab refresh doesn't force re-auth. **No long-lived storage**;
closing the tab or `signoutRedirect` clears them. XSS exposure
is the standard SPA tradeoff — mitigated by short access-token
TTL (15 min), strict CSP on the admin bundle, and the option to
move to a BFF cookie pattern later if the threat model
tightens. Knievel doesn't see refresh tokens; they only flow
between the SPA and Keycloak.

### 6. Paste-a-token fallback

When `admin_ui.oidc.require_oidc` is `false` (or the OIDC
metadata fetch fails — Keycloak unreachable, misconfigured), the
UI shows a "Paste an opaque token" form alongside the OIDC
login button. Operator pastes a `kvl_*` token minted via
`POST /v1/orgs/{org_id}/tokens`; the rest of the UI uses it
identically to a JWT (it's just a Bearer credential to the API).

Use cases:

- Bootstrap: spin up a knievel cluster before Keycloak exists.
- Disaster recovery: Keycloak outage shouldn't lock operators
  out of knievel.
- Local dev: `docker compose up` doesn't include Keycloak
  (`Local Development` section above); paste-token flow works
  unchanged against the seeded Org Editor token.
- CI smoke tests: deterministic credential, no IdP dependency.

In production this fallback is typically disabled
(`require_oidc: true`) so SSO/MFA/audit policy can't be
sidestepped; operators with a legitimate need keep one
break-glass opaque token in their secret store.

### 7. Operational verification

Phase 7.9 hardens the SPA's OIDC flow with end-session
integration, an idle-warning modal, and role-claim-driven UI
gating. Verify against a real realm before declaring the
admin UI live for an environment:

1. **Sign-in round-trip.** Visit the admin UI's URL,
   confirm the Keycloak login page renders, sign in, and
   land on the post-login deep link (the `?return_to=`
   carried through `state.return_to`). The
   `Authorization: Bearer …` header attached by the SPA's
   fetch wrapper should be a JWT (not a `kvl_*` opaque
   token); confirm `GET /v1/whoami` returns the right
   `org_id` + `role`.
2. **Group → claim mapping.** Sign in as a user in
   `/knievel/<org-id>/editor`; confirm `whoami.role` is
   `editor`. Sign in as a user in
   `/knievel/<org-id>/admin`; confirm `admin`. The SPA's
   role-gating shows / hides the Settings rail and the
   "New advertiser" button accordingly.
3. **End-session.** Click "Sign out" in the SPA. The
   browser should bounce through Keycloak's
   `end_session_endpoint` (passing `id_token_hint`) and
   land back at the admin UI's root. Re-clicking any
   protected link should re-prompt for sign-in (proving the
   SSO session was actually invalidated, not just the local
   token).
4. **Idle warning.** Configure Keycloak's access-token
   lifespan to a short value (e.g. 2 minutes) for the test.
   Sign in, leave the SPA idle for ~1 minute. The
   "Session expiring" modal should appear; click "Stay
   signed in" to confirm `signinSilent()` refreshes the
   token without a redirect. Reset the lifespan to your
   production value when done.
5. **Silent-refresh on 401.** With Keycloak still on the
   short lifespan, leave the tab open until the access
   token expires, then trigger a `useQuery` refetch (e.g.
   navigate between resource lists). The fetch wrapper
   should attempt `signinSilent()`, get a fresh token, and
   succeed on retry — visible only as a brief loading
   flicker, no error toast.
6. **Cross-tenant gate.** Sign in as a user in org A, then
   manually navigate to `/orgs/<org-b>/projects`. The API
   returns `403 wrong_tenant`; the SPA renders the error
   toast. (Spot-check that the SPA itself didn't leak the
   forbidden org's data — it shouldn't, since it never
   fetched it.)

If any of (3)–(5) misbehaves, the most common cause is an
incorrect `post_logout_redirect_uri` or
`automaticSilentRenew` setting on the public client; check
Keycloak's "Client → Settings" tab against the values in
"1. Create the admin-UI client" above.

### Why public client + PKCE, not BFF

Two architectures could carry OIDC for the SPA:

- **Public client + PKCE, all browser-side (chosen).** No UI
  server. Tokens in memory + optional `sessionStorage`. Works
  with the same-origin static-files deploy from `UI.md`.
- **Backend-for-Frontend (BFF) with httpOnly session cookies.**
  Stronger XSS posture (tokens never touch JS), but requires
  standing up a Node/Rust BFF that holds Keycloak client
  credentials, and means knievel grows a stateful cookie/session
  surface — both contradict `UI.md` ("No Next.js, no SSR" and
  "Bearer tokens, not cookies").

PKCE is the industry default for SPAs in 2026, including for
sensitive admin consoles, and the trade-off is acceptable given
the short access-token TTL and the option to add BFF later
without changing knievel's auth surface (the BFF would just
present the same JWT to knievel that the SPA does today).

## Kubernetes ServiceAccount Tokens

When knievel is reached from pods inside a Kubernetes cluster, the
zero-trust answer is to use the pod's projected ServiceAccount token
as the Bearer credential. No static secrets, automatic rotation,
identity is the SA itself.

### How it works

Modern Kubernetes (1.21+) issues bound ServiceAccount tokens as
JWTs, signed by the cluster's API server. Each pod can mount a
projected token with a deployment-specified audience. The cluster
exposes a JWKS at `https://kubernetes.default.svc/openid/v1/jwks`,
reachable from any in-cluster workload.

Knievel validates these tokens with the same JWKS machinery used for
Keycloak — just a different `issuer` entry in
`auth.jwt.issuers[]`.

### The catch: SA tokens have no `knievel` claim

A typical SA token's payload looks like:

```json
{
  "iss":  "https://kubernetes.default.svc.cluster.local",
  "sub":  "system:serviceaccount:rx-prod:knievel-client",
  "aud":  ["knievel"],
  "exp":  1717003600,
  "kubernetes.io": { "namespace": "rx-prod",
                     "serviceaccount": { "name": "knievel-client", "uid": "..." } }
}
```

No `knievel` claim, so we can't read scope/role/org_id directly.
Instead, knievel supports a per-issuer **claim mapping** that derives
the principal from a deterministic claim (typically `sub`):

```yaml
auth:
  jwt:
    issuers:
      - issuer:    https://kubernetes.default.svc.cluster.local
        audience:  knievel
        algorithms: [RS256]
        # No `claim` — fall back to claim_mapping rules.
        claim_mapping:
          # First matching rule wins. Match keys can be any top-level
          # claim (sub, kubernetes.io.namespace, etc.).
          rules:
            - match:
                sub: system:serviceaccount:rx-prod:knievel-client
              principal:
                scope:   org
                org_id:  scientist-com-prod
                role:    editor
            - match:
                sub: system:serviceaccount:rx-staging:knievel-client
              principal:
                scope:   org
                org_id:  scientist-com-staging
                role:    editor
```

Multiple issuers coexist — knievel picks the right one from the
JWT's `iss` claim. So the same knievel can simultaneously trust
Keycloak (for humans and out-of-cluster service-to-service) and the
Kubernetes API server (for in-cluster pods).

### Pod side: projected token

In the deployment manifest, mount a projected SA token with
`audience: knievel`:

```yaml
spec:
  serviceAccountName: knievel-client
  containers:
    - name: app
      volumeMounts:
        - name: knievel-token
          mountPath: /var/run/secrets/knievel
          readOnly: true
  volumes:
    - name: knievel-token
      projected:
        sources:
          - serviceAccountToken:
              path: token
              audience: knievel
              expirationSeconds: 600       # auto-rotated by kubelet
```

The application reads `/var/run/secrets/knievel/token` and uses it
as the Bearer credential. Re-read periodically (or each request);
the kubelet rotates the file before expiry.

### Why this is the right default for in-cluster deployments

- **No static secrets.** No long-lived API key in a Kubernetes
  Secret to leak or rotate manually.
- **Automatic rotation.** Tokens expire in 10 minutes by default;
  the kubelet re-projects before expiry. The app reads from disk;
  the disk file is current.
- **Principal is the SA.** Audit lines naturally contain
  `system:serviceaccount:<ns>:<name>` — no separate "which client
  is this" lookup.
- **Federation-free.** No Keycloak round-trip, no token exchange,
  no extra service in the path. Per-request auth is one signature
  verification against an in-cluster JWKS.

### EKS: in-cluster issuer vs. external OIDC URL

EKS clusters publish their ServiceAccount tokens via **two**
discoverable issuers, and they identify *the same tokens* — just
differ in how knievel reaches the JWKS:

- **`https://kubernetes.default.svc.cluster.local`** — the standard
  in-cluster issuer. JWKS at
  `https://kubernetes.default.svc/openid/v1/jwks`. Reachable only
  from pods inside the cluster. **This is what you want when
  knievel runs in the same EKS cluster as the calling app.**
- **`https://oidc.eks.<region>.amazonaws.com/id/<cluster-id>`** —
  EKS's public per-cluster OIDC endpoint, primarily used by IRSA
  (IAM Roles for Service Accounts). Reachable from outside the
  cluster. Configured per-cluster; visible in the EKS console under
  "OpenID Connect provider URL." Requires OIDC association to be
  enabled on the cluster (default for any cluster that's been used
  with IRSA).

Pick by where knievel lives:

| Scenario | Issuer to trust |
|---|---|
| Knievel and calling app in the same EKS cluster. | `https://kubernetes.default.svc.cluster.local` |
| Knievel runs elsewhere (different cluster, EC2, on-prem) and is called by EKS pods. | `https://oidc.eks.<region>.amazonaws.com/id/<cluster-id>` |
| Knievel called by pods from multiple EKS clusters. | One issuer entry per cluster's public OIDC URL. |

Don't mix them — pick one per cluster you trust. Using the public
URL for in-cluster traffic adds a needless public-endpoint
dependency (DNS, NAT egress, key-cache hit on every cold start);
using the in-cluster URL from outside the cluster doesn't resolve
at all.

The token contents are identical regardless of which issuer URL the
JWKS is fetched from — the API server signs the same JWT either
way. The only difference is the `iss` claim that ends up baked into
the token, which has to match whichever URL knievel has configured.

(Same nuance applies, in spirit, to GKE Workload Identity Federation
and AKS — both expose external OIDC endpoints alongside the
in-cluster one. We don't enumerate them here because RX is on EKS;
the pattern transfers verbatim.)

### Alternatives (heavier)

- **Token exchange via Keycloak.** Pod presents SA token to
  Keycloak's `token-exchange` endpoint; Keycloak (federated with the
  cluster's OIDC issuer) returns a Keycloak access token with the
  full `knievel` claim. Useful if you want claim mapping managed in
  Keycloak's UI rather than in knievel config. Adds one hop per
  token refresh.
- **SPIFFE / SPIRE.** Industry-standard workload identity; SPIRE
  issues JWT-SVIDs that knievel validates as any other JWT. Worth
  the operational investment only if you have many services that
  all want the same primitive.

Both are additive — the JWKS code path is shared.

## Authorization

Authentication tells knievel **who** the caller is. Authorization
decides **what they can do**. The model is small, explicit, and
identical regardless of how the principal was identified — same role
enum, same enforcement, whether the Bearer was an opaque token, a
Keycloak JWT, or a Kubernetes SA token.

### The model

Five roles, two scopes:

| Role | Scope | Capabilities |
|---|---|---|
| `org-owner` | Org | Manage org, billing, projects, members, all tokens. Implicit Project Admin on every project. |
| `org-admin` | Org | Same minus billing and ownership transfer. Implicit Project Admin on every project. |
| `admin` | Project (or "everywhere" via Org token) | Full CRUD on resources; manage project tokens / members. |
| `editor` | Project (or "everywhere" via Org token) | CRUD on Advertisers, Campaigns, Flights, Ads, Creatives, Templates, Sites, Zones, Ad Library items. The integration role. |
| `reader` | Project (or "everywhere" via Org token) | `GET` everything in scope, including `POST /decisions` and `POST /decisions:explain`. |

Ordering for "at least":

```
reader < editor < admin < org-admin < org-owner
```

A token's `(scope, role)` is fixed at issuance — it doesn't change
mid-session, doesn't escalate via headers, doesn't downgrade except
by the operator revoking and reminting.

### How a request is gated

For every authenticated request, in order:

1. **Principal extraction.** The auth layer (covered above) yields a
   `Principal { token_type, scope, org_id, project_id?, role }`.
   `project_id` is `Some` only for Project-scoped tokens.
2. **Org match.** Compare the principal's `org_id` to the org
   implied by the request path:
   - `/v1/orgs/{org_id}/...` → must equal `principal.org_id`.
   - `/v1/projects/{project_id}/...` → look up the project's parent
     org from the snapshot; must equal `principal.org_id`.
   Mismatch → `403 forbidden / wrong_tenant`.
3. **Project match** (only for project-scoped paths).
   - Project-scoped tokens: path `{project_id}` must equal the
     token's `project_id`. Mismatch → `403 forbidden / wrong_project`.
   - Org-scoped tokens: any project in the org is fine (org_id check
     in step 2 already covered this).
4. **Role check.** The endpoint declares a minimum role. Project
   endpoints need a project-level role (or higher org role).
   Org endpoints need the appropriate org-level role.
   - Org-Admin/Owner tokens implicitly satisfy any Project Admin
     requirement.
   - An Org token with `role=editor` is treated as Project Editor
     on every project in the org (and as a non-mutating org reader
     for org-level GETs like listing projects or the Ad Library).
   - `org-owner` is **not** automatically project-superuser beyond
     `org-admin`; it adds billing/transfer powers, nothing else.
   Insufficient role → `403 forbidden / role_insufficient`.

All checks happen before any handler logic runs. A failed authz
check looks identical from the outside whether the principal's scope
or role was wrong; the response code (`forbidden`) is opaque, the
detail string is what differentiates for log debugging.

### Endpoint → minimum role

#### Decision and Decision Explainer (project-scoped)

| Endpoint | Min role | Notes |
|---|---|---|
| `POST /v1/projects/{p}/decisions` | `reader` | The query path. |
| `POST /v1/projects/{p}/decisions:explain` | `reader` | Reveals nothing the caller can't already see via flight/ad/creative GETs. |
| Decision request with **any** `force.*` field set | `admin` | Three-control gate: (1) project-level `allow_force_decision` flag must be enabled; (2) caller must hold Project Admin or higher; (3) every forced call writes a row to `knievel.audit_log`. Knievel-side `decisions.force_overrides_enabled: false` is a global kill-switch that disables forced paths cluster-wide regardless of per-project state. |

#### Project resources (project-scoped, all under `/v1/projects/{p}/`)

Resources: Advertisers, Campaigns, Flights, Ads, Creatives,
CreativeTemplates, Sites, Zones.

| Operation | Min role |
|---|---|
| `GET` (list, get) | `reader` |
| `POST` create | `editor` |
| `PATCH` update | `editor` |
| `:batchUpsert` | `editor` |
| `POST /creatives/{id}/image` | `editor` |

Read-only inventory (Channels, Priorities, AdTypes):

| Operation | Min role |
|---|---|
| `GET` | `reader` |

#### Org resources (under `/v1/orgs/{o}/`)

| Endpoint | Min role |
|---|---|
| `GET /projects` | `reader` (any token in org) |
| `GET /projects/{id}` | `reader` |
| `POST /projects` | `org-admin` |
| `POST /projects:batchUpsert` | `org-admin` |
| `PATCH /projects/{id}` | `org-admin` |
| `GET /tokens` | `org-admin` |
| `POST /tokens` | `org-admin` |
| `DELETE /tokens/{id}` | `org-admin` |
| `GET /members` | `org-admin` |
| `POST /members` (invite) | `org-owner` |
| `PATCH /members/{u}` (role change) | `org-owner` |
| `DELETE /members/{u}` | `org-owner` |
| `GET /ad-library/items` | `reader` (any token in org) |
| `GET /ad-library/items/{id}` | `reader` |
| `POST /ad-library/items` | `editor` (org-scoped) |
| `PATCH /ad-library/items/{id}` | `editor` |
| `POST /ad-library/items:batchUpsert` | `editor` |
| `GET /ad-library/items/{id}/references` | `reader` |

#### Public and system endpoints

| Endpoint | Auth |
|---|---|
| `GET /e/i/{signed}` | None — HMAC in URL. |
| `GET /e/c/{signed}` | None — HMAC in URL. |
| `GET /openapi.json` | None by default (operator can gate via reverse proxy). |
| `GET /healthz`, `/readyz`, `/metrics`, `/version` | None by default. |

### Cross-cutting rules

- **Project Editor + Ad Library references.** A Project Editor can
  create an Ad that references `ad_library_item_id` (the library is
  org-scoped, but referencing it from a project Ad is just a
  validated foreign key). The library content itself remains
  read-only to project-only tokens; mutation requires an org-scoped
  Editor or higher.
- **Ad Library item deletion (via `is_active: false`)** with
  references still present succeeds (soft delete) but emits a
  warning header (`X-Knievel-Warning: dangling_references`) listing
  the project Ads that reference it. References continue to resolve
  via the snapshot until they're updated.
- **Org Owner self-removal** is rejected unless ownership is
  transferred first. The endpoint returns `409 conflict /
  last_owner`.
- **Member's per-project roles** (when humans land via the admin UI)
  are stored on the membership row, not the token. Tokens always
  carry their full role set baked in at issuance.

### Implementation

The plan is per-handler role guards via a `poem-openapi` extractor:

```rust,ignore
#[handler]
async fn upsert_advertiser(
    Principal(p): Principal<RequireProject<Editor>>,
    // ... other extractors ...
) -> Result<Json<Advertiser>> { ... }
```

`Principal<R>` is generic over a `RequireRole` trait that encodes the
minimum role + scope. The extractor:

1. Pulls and validates the Bearer (opaque or JWT).
2. Resolves project IDs in the path against the principal's scope.
3. Compares the principal's effective role (with org-token
   inheritance) against the endpoint's required role.
4. Either yields the validated `Principal` or returns `401`/`403`.

The OpenAPI spec advertises the requirement on each operation via
`x-knievel-required-role: editor` (or similar) so generated clients
can surface it in docs and test fixtures.

### Audit and observability

Every authz outcome is fed to the structured-log layer:

- `Allow` → `INFO` with `principal_id`, `principal_role`,
  `endpoint`, `request_id`. Decision-endpoint allows are sampled at
  the same rate as the rest of the endpoint logging
  (`logging.decisions_sample_rate`); other endpoints log full.
- `Deny` → always logged at `WARN`, with the reason code
  (`wrong_tenant` / `wrong_project` / `role_insufficient`). Also
  captured to Sentry as a breadcrumb (not a thrown error — denials
  are expected operational events) so a sudden spike in denials
  shows up in dashboards.

### What's deferred

- **Site Group scoping.** Project members/tokens scoped to a subset
  of Sites within a project. Roadmap; data model leaves room.
- **Granular per-resource permissions.** A token that can read
  decisions but not flights, etc. Out of scope; the four roles
  cover the v0 use cases.
- **Per-role rate limits.** Currently rate limits are per-token.
  Per-role caps may emerge if abuse patterns demand it.
- **Project-scoped token mint endpoints** (`POST
  /v1/projects/{p}/tokens`). Currently all token mint flows go
  through the org level. Adds value when the admin UI ships and
  Project Admins want to mint scoped tokens without involving the
  org admin.
- **Attribute-based access control** (e.g. "this token can only
  read Advertisers tagged `region=eu`"). Out of scope until a real
  use case appears.

## Startup Linting and Effective-Policy Visibility

Auth misconfiguration is silent and catastrophic — a too-permissive
algorithm allow-list, a missing audience, an issuer that maps every
token to god-mode. Knievel boots fail-closed: if the auth config
doesn't pass the lint pass, the process exits before serving a
request.

### Boot-time validation rules

The following are checked at startup and produce a hard error
(non-zero exit, structured log line with the offending config path):

- **Algorithm allow-list.** Each issuer's `algorithms` list must
  contain only asymmetric algorithms (`RS256`, `RS384`, `RS512`,
  `PS256`, `PS384`, `PS512`, `ES256`, `ES384`, `ES512`, `EdDSA`).
  `none` and any HMAC algorithm (`HS*`) are rejected unconditionally.
  An empty list is rejected.
- **Issuer / audience completeness.** Every entry in
  `auth.jwt.issuers[]` must have non-empty `issuer` and `audience`
  fields. JWKS auto-discovery is verified at boot — knievel fetches
  `{issuer}/.well-known/openid-configuration` once before declaring
  itself ready, so a typo in the issuer URL fails at boot, not on
  first request.
- **Claim handling.** Each issuer must specify either `claim:
  <name>` (single rich-claim mode) **or** `claim_mapping: {rules:
  [...]}` (derived-principal mode), not both, not neither.
- **`claim_mapping` schema.** Each rule must have a non-empty
  `match` block (at least one claim to match on) and a complete
  `principal` block (`scope`, `org_id`, plus `project_id` for
  `scope: project`, plus `role`). Malformed rules fail the boot.
- **`claim_mapping` coverage.** Issuers using `claim_mapping` must
  have at least one rule. An issuer with `claim_mapping: { rules:
  [] }` would silently pass through every JWT from that issuer with
  no principal mapped — knievel rejects this configuration outright
  to prevent the footgun.
- **Mode coherence.** `auth.modes` must be a non-empty subset of
  `[opaque, jwt]`. If `jwt` is listed, at least one issuer entry
  must be present and pass the checks above. If `opaque` is the only
  mode, the JWT block is permitted to be absent.

Failures are surfaced as a single startup error block, listing
every offending entry with its config path and the failure
category. Operators see all problems at once instead of fixing
them iteratively.

### Effective-policy visibility

Once boot passes the lint, knievel publishes the effective auth
policy in two operator-visible places:

- **Startup INFO log entry.** A single structured log line
  enumerating: enabled modes; per-issuer issuer URL, audience,
  algorithms, claim source (`claim` or `claim_mapping` with rule
  count), and JWKS URL discovered. Secrets and signing keys are
  never logged.
- **`GET /version` response.** The `auth` block in the version
  payload mirrors the startup log. Example:

  ```json
  {
    "knievel": "0.4.2",
    "schema": "0.4",
    "auth": {
      "modes": ["jwt"],
      "issuers": [
        {
          "issuer": "https://keycloak.scientist.com/realms/scientist",
          "audience": "knievel",
          "algorithms": ["RS256"],
          "claim_source": { "kind": "claim", "name": "knievel" },
          "jwks_url": "https://keycloak.scientist.com/realms/scientist/protocol/openid-connect/certs"
        },
        {
          "issuer": "https://kubernetes.default.svc.cluster.local",
          "audience": "knievel",
          "algorithms": ["RS256"],
          "claim_source": { "kind": "claim_mapping", "rule_count": 2 }
        }
      ]
    }
  }
  ```

Operators inspecting `/version` can answer "what auth is this pod
actually doing?" without reading config files or restarting in
debug mode. Useful during incidents and routine audits alike.

## Trade-offs

### Revocation is by expiry

A revoked Keycloak service account keeps working until its current
token's `exp` passes. Mitigations:

- **Short token lifetimes.** 15-minute access tokens are typical;
  Keycloak issues a fresh one each request via the calling app's HTTP
  client. The calling app caches and refreshes; knievel validates
  whatever it sees.
- **Don't add introspection per-request.** It defeats the stateless
  validation that makes JWT cheap; live revocation can be done by
  rotating the JWKS signing key in Keycloak (forces all outstanding
  tokens to fail signature) for genuine emergencies.
- **For sensitive operations** (member management, token mint) we
  could add introspection later behind a flag. Not in v0.

### Audit trail moves outside knievel

With opaque tokens, knievel knows the token's display name and
creator. With JWTs, the principal is whatever the IdP says — typically
`sub` (a service account ID) and `azp` (the client). Knievel logs
`(iss, sub, azp)` on every authenticated request and includes them in
Sentry scope; correlating that to a human or service is a
Keycloak-side question.

### Operator owns the protocol mapper

The `knievel` claim is injected by a Keycloak protocol mapper that
knievel cannot validate from its side. Misconfiguration manifests as
`401 invalid_token / claim_missing` or `claim_malformed`. The error
detail tells the operator what was missing or wrong.

### JWKS fetch needs egress

Knievel pods must be able to reach the issuer's
`/.well-known/openid-configuration` and JWKS endpoints. Default
operator-friendly: use the issuer's public hostname; they're already
exposed to anything that needs to authenticate against the IdP.

## Mixing Modes During Cutover

Both modes can run simultaneously. Practical migration shape:

1. Deploy knievel with `modes: [opaque]`. Mint opaque tokens for the
   calling app. Ship and stabilize.
2. Provision the Keycloak client and mapper. Verify token issuance
   from a curl manually.
3. Flip config to `modes: [opaque, jwt]`. The calling app starts
   using JWTs; opaque tokens still work for tooling that hasn't
   migrated.
4. Once everything is on JWT, flip to `modes: [jwt]` and revoke the
   opaque tokens.

Knievel doesn't preference one mode over the other — whichever the
client presents wins.

## Local Development

Dev mode is **opaque-tokens-only**, self-bootstrapping via
`knievel-cli seed-demo`. JWT issuers (Keycloak, K8s API server) stay
disabled by default — neither is meaningfully available when the
calling app is running natively from `bin/rails server` or
equivalent.

### The flow

```
docker compose up
  ├── postgres                           (the dev cluster)
  ├── knievel                            (auth.modes: [opaque])
  └── knievel-seed                       (one-shot init)
        runs: knievel-cli seed-demo \
              --write-token-to=/out/knievel-dev-token
        volume mount: ./tmp:/out
```

The seed sidecar:

1. Waits for knievel's `/readyz` to return 200.
2. Connects directly to Postgres (bypassing HTTP — bootstrap auth
   is a chicken-and-egg problem; the CLI shares the schema and
   writes rows in a transaction).
3. Creates a demo Organization, demo Project, default Site/Zone,
   one CreativeTemplate, and a sample
   Advertiser → Campaign → Flight → Ad → Creative chain so
   decision requests return something interesting.
4. Mints one opaque Org Editor token, hashes it for storage, and
   writes the plaintext value to the host-mounted file.

The calling app reads that file from disk and uses it as the Bearer
credential. Real auth code path, real opaque-token validation, no
bypass — just no IdP in the picture.

### Configuration knobs

```yaml
# config.yaml (dev overrides)
auth:
  modes: [opaque]               # JWT off in dev
```

```bash
# Reproducible token value (e.g. for CI fixtures):
knievel-cli seed-demo --token=kvl_dev_local_demo_token

# Default: random token, written to the path passed to --write-token-to.
```

### Why no auth bypass

We considered (and rejected) a "dev mode skips auth" flag. It's a
common shortcut and a common security incident — a flag that ships
to staging or worse. Opaque tokens are cheap to provision via the
CLI; the seed sidecar makes it a single `docker compose up`. The
real auth code path runs every time, so dev catches auth bugs that
prod would otherwise hit first.

### Testing the JWT path locally

Optional, not the default. When debugging Keycloak protocol mappers
or claim-mapping rules:

1. Add a Keycloak service to the compose file (or point at a shared
   dev-environment Keycloak).
2. Configure knievel's `auth.modes: [opaque, jwt]` and add a
   `jwt.issuers[]` entry for the dev realm.
3. The calling app's gem is configured to fetch from Keycloak
   instead of reading the token file.
4. Both modes coexist; you can flip back without rebuilding.

K8s ServiceAccount tokens are not a dev-mode concern. They're
production/staging-cluster path only; the local native Rails
process has no projected token to present.

## OIDC for Humans

The plan sketched here originally has been promoted to a real
spec — see **"Keycloak Setup — Human Admin UI (PKCE)"** above for
the canonical flow, including the public-client config, the
group-membership claim mapper, the runtime-config endpoint, and
the paste-a-token fallback for bootstrap / Keycloak outages. The
admin UI (`UI.md`) consumes that contract directly.

Knievel-side, no new validation code is needed: the same JWKS
machinery that handles `client_credentials` JWTs handles human
PKCE tokens too. The OIDC dance lives entirely in the SPA via
`oidc-client-ts`; knievel only sees Bearer JWTs. Implementation
ships across Phase 7 (`PHASES.md` 7.4 and 7.9).

## References

- [`jsonwebtoken`](https://docs.rs/jsonwebtoken/) — JWT validation
- [`jwt-authorizer`](https://docs.rs/jwt-authorizer/) — higher-level wrapper with JWKS caching
- [`openidconnect`](https://docs.rs/openidconnect/) — full OIDC client (post-v0)
- [Keycloak — Protocol mappers](https://www.keycloak.org/docs/latest/server_admin/#_protocol-mappers)
- [Keycloak — Service Accounts](https://www.keycloak.org/docs/latest/server_admin/#_service_accounts)
- [RFC 7517 — JSON Web Key (JWK)](https://datatracker.ietf.org/doc/html/rfc7517)
- [RFC 7519 — JSON Web Token (JWT)](https://datatracker.ietf.org/doc/html/rfc7519)
- [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html)
