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
