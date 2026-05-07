# Security Policy

## Reporting a vulnerability

Email **security@knievel-ads.example** (GPG key on the GitHub org
profile if you'd like to encrypt). Please:

- Don't open a public issue for security findings.
- Include reproduction steps, an impact assessment, and the
  affected version (or `main` SHA).
- Allow up to **5 business days** for an initial response.

We aim for **90-day coordinated disclosure** by default. If you need
a tighter window because the vulnerability is being exploited in the
wild, say so — we'll move accordingly. If you need a longer window
for downstream coordination, also say so; we'll work with you.

We don't currently run a paid bug-bounty program; we're happy to
acknowledge contributions in `CHANGELOG.md` (and on a future security
advisories page) if you'd like the credit.

## Supported versions

| Version | Status |
|---|---|
| `0.1.x` (latest) | ✅ supported |
| `0.0.x` (squat) | ❌ end-of-life — never published a real release. |

Once `0.2.0` ships, `0.1.x` will receive security fixes for **6 months**
in line with the deprecation window in `REQUIREMENTS.md` § 6.4. After
that, `0.1.x` is end-of-life.

## Security model

Three paragraphs covering the v0 perimeter; the operator owns the
infrastructure-side details.

### 1. Trust boundary

Knievel is a **server-to-server** API in v0. Every authenticated
request comes from a calling application holding a bearer token; the
calling application is trusted to authenticate its own end users.
The browser-facing surface is limited to the HMAC-signed
`/e/i/{sig}` and `/e/c/{sig}` event endpoints, where the signature
in the URL is the authorization. There is no public CORS-relaxed
decision endpoint, no first-party-cookie session, and no anonymous
write surface.

Out-of-scope (operator-owned): TLS termination, WAF / DDoS
protection, IP allow-listing, network segmentation, and the
authenticity of the calling application itself.

### 2. Tenant isolation

Postgres `FORCE ROW LEVEL SECURITY` is the floor:

- Every per-tenant table has an RLS policy keyed on
  `current_setting('knievel.org_id')` and (where applicable)
  `current_setting('knievel.project_id')`.
- The query layer sets both GUCs at the start of every project-
  scoped transaction (`src/handlers.rs::open_project_tx`).
- CI enforces a **third layer**: `xtask check-cross-tenant`
  fails the build if a project-scoped endpoint is missing from
  `tests/cross_tenant_manifest.toml`. As of `v0.1.6`, 47
  endpoints are covered.

A superuser-on-the-app-role mistake silently defeats RLS. The
reference compose stack and the CI test harness both downgrade the
app role to `NOSUPERUSER CREATEDB` immediately after creation;
operator-managed Postgres deploys must do the same. Documented as
gotcha 17 in `CLAUDE.md`.

`REQUIREMENTS.md` § 7.1.1 has the full RLS contract.

### 3. Authentication

Two authentication paths, configured per project:

- **Opaque bearers** (`kvl_<env>_<scope>_<short-id>_<secret>`) —
  hashed with argon2id at rest, verified on every request. Default
  Argon2id parameters per OWASP recommendations (memory 64 MiB,
  iterations 3, parallelism 1).
- **JWTs via JWKS** — issuer + audience matched against config;
  JWKS auto-discovered from the `iss` claim's well-known endpoint
  with a 5-minute cache. Claim mapping configurable per issuer.

Event endpoints use HMAC-SHA256 signatures with stable `dedup_key`
across an 8-hour signing-secret rotation overlap. Replays are
silently dropped at the dedup layer; impressions/clicks are
idempotent.

`AUTH.md` covers the full surface, including the K8s service-account
recipe and the rotation procedure.

## What's deliberately out of scope

Operator-owned items that knievel doesn't ship:

- **TLS termination.** A reverse proxy (nginx, Envoy, ALB, etc.) sits
  in front of knievel.
- **Network policy / WAF.** Use your cloud's tools.
- **S3 bucket policy.** The operator scopes the IAM role / bucket
  policy on the upload target.
- **OS hardening.** The container ships distroless; the operator
  picks the host kernel.
- **Backup encryption.** The operator owns backup encryption at
  rest.
- **Per-region key management.** Single-region in v0; multi-region
  KMS isn't a knievel concern.

## Known surface gotchas

Documented for awareness; none are bugs in knievel:

- **Postgres `FORCE ROW LEVEL SECURITY` is bypassed by superusers.**
  See § 2 above.
- **Aurora drops LISTEN/NOTIFY across failovers.** The poll backstop
  catches up within 5 seconds; no durable contract loss.
- **Argon2id verification is the most expensive part of the hot
  path.** Per-request, dwarfs the HTTP framing + DB lookup. Bench
  before tuning.

## Disclosure log

(To be populated as advisories ship.)
