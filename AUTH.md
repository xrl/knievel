# Knievel Authentication

Knievel supports two coexisting credential types on its Management and
Decision endpoints. Either or both can be enabled per deployment.

| Mode | Format | When to use |
|---|---|---|
| **Opaque token** | `kvl_<env>_<scope>_<random>` | Bootstrap, deployments without an IdP, the eventual admin UI's session credentials. |
| **JWT** | Three base64url segments separated by `.` | When a Keycloak / OIDC provider is already in place. Identity stays in the IdP; knievel just validates. |

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
- Minted via `POST /v1/orgs/{orgId}/tokens`. Revocable. Last-used
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
| `org_id` | yes | Knievel Org ID or `externalId`. |
| `project_id` | when `scope=project` | Knievel Project ID or `externalId`. |
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

## Keycloak Setup

For a Keycloak server at `https://keycloak.scientist.com`:

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
   - `/v1/orgs/{orgId}/...` → must equal `principal.org_id`.
   - `/v1/projects/{projectId}/...` → look up the project's parent
     org from the snapshot; must equal `principal.org_id`.
   Mismatch → `403 forbidden / wrong_tenant`.
3. **Project match** (only for project-scoped paths).
   - Project-scoped tokens: path `{projectId}` must equal the
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
  create an Ad that references `adLibraryItemId` (the library is
  org-scoped, but referencing it from a project Ad is just a
  validated foreign key). The library content itself remains
  read-only to project-only tokens; mutation requires an org-scoped
  Editor or higher.
- **Ad Library item deletion (via `isActive: false`)** with
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

```rust
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

## OIDC for Humans (post-v0)

When the admin UI lands, humans authenticate via Keycloak using the
authorization-code-with-PKCE flow:

1. UI redirects browser to Keycloak.
2. User authenticates, consents.
3. Keycloak redirects back with an auth code.
4. UI exchanges code for tokens server-side.
5. UI presents the access token to knievel as a Bearer JWT.

The validation backend on the knievel side is **the same JWKS
machinery we land in v0** — same crates, same cache, same claim
mapping. The difference is upstream: instead of `client_credentials`,
the human's access token is issued via auth-code flow, and the
`knievel` claim is filled from the user's group/role memberships in
Keycloak rather than a hardcoded mapper.

The `openidconnect` Rust crate handles the auth-code dance on the
admin UI's server side; we don't need it in knievel itself.

## References

- [`jsonwebtoken`](https://docs.rs/jsonwebtoken/) — JWT validation
- [`jwt-authorizer`](https://docs.rs/jwt-authorizer/) — higher-level wrapper with JWKS caching
- [`openidconnect`](https://docs.rs/openidconnect/) — full OIDC client (post-v0)
- [Keycloak — Protocol mappers](https://www.keycloak.org/docs/latest/server_admin/#_protocol-mappers)
- [Keycloak — Service Accounts](https://www.keycloak.org/docs/latest/server_admin/#_service_accounts)
- [RFC 7517 — JSON Web Key (JWK)](https://datatracker.ietf.org/doc/html/rfc7517)
- [RFC 7519 — JSON Web Token (JWT)](https://datatracker.ietf.org/doc/html/rfc7519)
- [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html)
