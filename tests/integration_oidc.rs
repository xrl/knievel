//! Integration test: real Keycloak JWT round-trips through
//! `BearerAuth` to `/v1/whoami`.
//!
//! Phase 3.26 follow-up. Closes the missing-E2E-coverage gap
//! the `claude/jwt-bearer-wiring` PR (knievel #23) exposed:
//! the unit tests in `src/auth/jwt.rs` cover the structural
//! validator (`validate(...)`) but never proved that
//! `JwtVerifier::verify` (the runtime path wired into
//! `BearerAuth::verify_bearer`) actually accepts a real
//! Keycloak-minted token. Three round-trips of "fix +
//! deploy-to-PR-preview" in knievel + scientist-hq/infra +
//! scientist-hq/k3-applications were paid because of that
//! gap — this test slice fails CI on any future drift between
//! knievel + the chart + the Keycloak realm shape.
//!
//! Flow:
//!   1. Spin up Keycloak via testcontainers in `start-dev` mode
//!      with the `knievel-test` realm pre-imported (mirrors the
//!      `client-knievel-pr.tf` realm shape — public PKCE client
//!      with `directAccessGrantsEnabled: true`, realm-level
//!      `knievel` scope with audience mapper, hardcoded
//!      `knievel`-claim mapper emitting
//!      `{scope: org, org_id: scientist-com-pr, role: editor}`).
//!   2. Mint a JWT via the resource-owner-password grant
//!      against `{issuer}/protocol/openid-connect/token`
//!      (PKCE flow needs a browser; password grant doesn't and
//!      is fine for this test fixture — the realm exists only
//!      in-process).
//!   3. Build an `AppState` with `auth.jwt.issuers` pointing at
//!      the testcontainer Keycloak issuer URL, audience
//!      `knievel`, claim `knievel`. No Postgres pool is wired
//!      because `/v1/whoami` only consumes `BearerAuth` and
//!      the JWT branch of `verify_bearer` short-circuits before
//!      hitting `state.db`.
//!   4. `GET /v1/whoami` with `Authorization: Bearer <jwt>`.
//!      Assert 200 + the principal carries
//!      `org_id=scientist-com-pr`, `role=editor`, `scope=org`,
//!      `actor_id` starts with `jwt:`.
//!   5. Negative tests: forged signature → 401; token signed
//!      by a key not in the JWKS → 401.
//!
//! Self-skips when Docker is unreachable so the CLAUDE.md "no
//! docker in sandbox" baseline stays intact. CI runs on a
//! GitHub-hosted runner where Docker is always available; the
//! `db-integ` job picks this binary up via the
//! `binary(/^integration/)` nextest filter.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use poem::test::TestClient;
use poem::EndpointExt;
use std::time::{Duration, Instant};
use testcontainers::core::{CopyDataSource, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, CopyTargetOptions, GenericImage, ImageExt};

const KEYCLOAK_IMAGE: &str = "quay.io/keycloak/keycloak";
// Pin a major. 25.x ships the modern Quarkus distribution and
// supports `start-dev --import-realm` against
// `/opt/keycloak/data/import/*.json`. Bumps are intentional.
const KEYCLOAK_TAG: &str = "25.0";
const REALM_JSON: &[u8] = include_bytes!("fixtures/keycloak-realm.json");
const REALM_NAME: &str = "knievel-test";
const CLIENT_ID: &str = "knievel-test-client";
const TEST_USER: &str = "testuser";
const TEST_PASS: &str = "testpass";
const AUDIENCE: &str = "knievel";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keycloak_minted_jwt_round_trips_through_whoami() -> Result<()> {
    if !docker_reachable().await {
        eprintln!(
            "integration_oidc: skipping — Docker not reachable. \
             This test self-skips in sandboxed environments per \
             CLAUDE.md 'Sandbox limitations'."
        );
        return Ok(());
    }

    let kc = start_keycloak().await?;
    let issuer = kc.issuer_url();
    eprintln!("integration_oidc: keycloak issuer = {issuer}");

    // Mint a real access token via the resource-owner-password
    // grant. PKCE is the SPA's path; password grant takes the
    // same realm + client config and produces an identical JWT
    // shape (same audience, same custom claim) without needing
    // a browser.
    let jwt = mint_password_grant_token(&issuer, CLIENT_ID, TEST_USER, TEST_PASS, AUDIENCE)
        .await
        .context("minting Keycloak access token")?;
    eprintln!(
        "integration_oidc: minted jwt header={} (len={})",
        jwt.split('.').next().unwrap_or(""),
        jwt.len()
    );

    let cli = TestClient::new(build_app_with_issuer(&issuer));

    // Happy path — Keycloak token authenticates `/v1/whoami` and
    // carries the hardcoded knievel-claim back through the
    // Principal.
    let resp = cli
        .get("/v1/whoami")
        .header("Authorization", format!("Bearer {jwt}"))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        body["scope"], "org",
        "scope must come from the hardcoded knievel-claim in the realm fixture"
    );
    assert_eq!(
        body["org_id"], "scientist-com-pr",
        "org_id must come from the hardcoded knievel-claim — \
         drift here means the realm fixture has diverged from \
         infra/terraform/keycloak/client-knievel-pr.tf"
    );
    assert_eq!(
        body["role"], "editor",
        "role must round-trip from the claim"
    );
    assert_eq!(
        body["token_type"], "jwt",
        "token_type=jwt proves verify_bearer dispatched to JwtVerifier::verify, \
         not the opaque path"
    );
    let actor_id = body["actor_id"]
        .as_str()
        .expect("actor_id is a string")
        .to_string();
    assert!(
        actor_id.starts_with("jwt:"),
        "actor_id must be prefixed with `jwt:` for JWT principals; got {actor_id}"
    );

    // Negative: flip a byte in the signature segment. The
    // structural shape stays valid (header+payload still parse,
    // alg/kid/iss/aud still match) so this exercises the
    // signature-verify step explicitly.
    let forged = forge_signature(&jwt);
    assert_ne!(forged, jwt, "forge must change the token");
    let resp = cli
        .get("/v1/whoami")
        .header("Authorization", format!("Bearer {forged}"))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);

    // Negative: a token signed by a different key entirely. We
    // fabricate a syntactically-valid JWT using
    // `jsonwebtoken::EncodingKey` with a fresh RSA key whose
    // `kid` is not in the realm JWKS. The header + payload
    // pass structural validation, the iss/aud/exp check would
    // pass, but the signature can't be verified by any key in
    // the cache. This is the path that catches a JWKS-cache
    // bug where a stale key would accidentally accept a
    // forgery.
    let unknown = mint_token_from_unrelated_key(&issuer, AUDIENCE)?;
    let resp = cli
        .get("/v1/whoami")
        .header("Authorization", format!("Bearer {unknown}"))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);

    Ok(())
}

/// Probe whether the local Docker daemon is reachable. Mirrors the
/// `tests/integration_migrations.rs` self-skip pattern but
/// re-targets it at Docker since this test brings up its own
/// container fleet rather than using a CI service container.
async fn docker_reachable() -> bool {
    // `docker info` is the cheapest "is the daemon awake" probe
    // that doesn't require pulling an image. Use a short timeout
    // so a hung daemon doesn't stall the suite.
    let probe = tokio::task::spawn_blocking(|| {
        std::process::Command::new("docker")
            .arg("info")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    });
    match tokio::time::timeout(Duration::from_secs(5), probe).await {
        Ok(Ok(Ok(status))) => status.success(),
        _ => false,
    }
}

struct KeycloakHandle {
    _container: ContainerAsync<GenericImage>,
    issuer: String,
}

impl KeycloakHandle {
    fn issuer_url(&self) -> String {
        self.issuer.clone()
    }
}

async fn start_keycloak() -> Result<KeycloakHandle> {
    // `start-dev --import-realm` boots Keycloak's dev mode (no
    // TLS, in-memory H2 by default) and imports any realm JSON
    // mounted at `/opt/keycloak/data/import/`. We embed the realm
    // fixture into the test binary via `include_bytes!` and copy
    // it into the container via `with_copy_to`, which works
    // regardless of the host filesystem layout (a bind-mount
    // would break in CI runners that ran the tests from a
    // different working directory).
    //
    // KC_BOOTSTRAP_ADMIN_* are required in 25.x even though we
    // never use the admin user — Keycloak refuses to boot
    // without an initial admin set.
    let image = GenericImage::new(KEYCLOAK_IMAGE, KEYCLOAK_TAG)
        .with_exposed_port(8080.tcp())
        // Wait until the openid-configuration endpoint exists for
        // our realm, not just generic "Keycloak started" log
        // text. The latter races: Keycloak prints "started" while
        // its realm-import worker is still applying the JSON,
        // and the first /token request can hit a 404 on the
        // realm path. The OIDC discovery doc is the natural
        // readiness gate — it's served only after import is done.
        .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
        .with_wait_for(WaitFor::message_on_stdout("started in"))
        .with_env_var("KC_BOOTSTRAP_ADMIN_USERNAME", "admin")
        .with_env_var("KC_BOOTSTRAP_ADMIN_PASSWORD", "admin")
        .with_env_var("KC_HTTP_ENABLED", "true")
        .with_env_var("KC_HOSTNAME_STRICT", "false")
        .with_env_var("KC_HOSTNAME_STRICT_HTTPS", "false")
        .with_env_var("KC_LOG_LEVEL", "INFO")
        .with_cmd(["start-dev", "--import-realm"])
        .with_copy_to(
            CopyTargetOptions::new("/opt/keycloak/data/import/realm.json"),
            CopyDataSource::Data(REALM_JSON.to_vec()),
        )
        .with_startup_timeout(Duration::from_secs(120));

    let container = image.start().await.context("starting Keycloak container")?;
    let host_port = container
        .get_host_port_ipv4(8080.tcp())
        .await
        .context("resolving Keycloak host port")?;
    let issuer = format!("http://127.0.0.1:{host_port}/realms/{REALM_NAME}");

    // Belt-and-braces readiness check — poll the OIDC discovery
    // doc until it 200s. testcontainers' wait-on-log strategy
    // covers Keycloak's "started" line, but the realm import
    // can lag by a few hundred ms after that. Bounded retry
    // with a hard 60s cap. Per CLAUDE.md / harness rules we
    // don't lead with a long sleep — short polls only.
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let discovery = format!("{issuer}/.well-known/openid-configuration");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_err: Option<String> = None;
    loop {
        if Instant::now() >= deadline {
            bail!(
                "Keycloak realm `{REALM_NAME}` not ready within 60s; last={:?}",
                last_err
            );
        }
        match http.get(&discovery).send().await {
            Ok(r) if r.status().is_success() => break,
            Ok(r) => last_err = Some(format!("status={}", r.status())),
            Err(e) => last_err = Some(e.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Ok(KeycloakHandle {
        _container: container,
        issuer,
    })
}

async fn mint_password_grant_token(
    issuer: &str,
    client_id: &str,
    username: &str,
    password: &str,
    scope_audience: &str,
) -> Result<String> {
    let token_url = format!("{issuer}/protocol/openid-connect/token");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    // `scope=openid <audience>` so the realm's `knievel` optional
    // scope is included on the token, which in turn pulls in the
    // audience mapper and grants `aud=knievel`.
    let resp = http
        .post(&token_url)
        .form(&[
            ("grant_type", "password"),
            ("client_id", client_id),
            ("username", username),
            ("password", password),
            ("scope", &format!("openid {scope_audience}")),
        ])
        .send()
        .await
        .context("POST token endpoint")?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("token endpoint returned {status}: {text}");
    }
    let body: serde_json::Value =
        serde_json::from_str(&text).context("parsing token endpoint JSON")?;
    let token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("token endpoint response missing access_token: {text}"))?;
    Ok(token.to_string())
}

fn build_app_with_issuer(issuer: &str) -> impl poem::Endpoint {
    let issuer_cfg = knievel::config::JwtIssuerConfig {
        issuer: issuer.to_string(),
        audience: AUDIENCE.to_string(),
        algorithms: vec!["RS256".into(), "ES256".into()],
        // Empty = derive via OIDC discovery
        // (`{issuer}/.well-known/openid-configuration`).
        jwks_url: String::new(),
        claim: "knievel".into(),
    };
    let verifier = knievel::auth::jwt::JwtVerifier::new(vec![issuer_cfg]);
    let state = knievel::state::AppState::new().with_jwt_verifier(verifier);
    knievel::server::routes().data(state)
}

/// Flip the leading byte of the signature segment so the token's
/// header + payload remain unchanged but the signature is
/// guaranteed-bad. Decoding the signature and re-encoding round-
/// trips through standard URL-safe-no-pad base64 so we don't
/// accidentally emit non-base64url characters.
fn forge_signature(token: &str) -> String {
    let mut parts: Vec<&str> = token.splitn(3, '.').collect();
    assert_eq!(parts.len(), 3, "expected three-segment JWT");
    let mut sig = URL_SAFE_NO_PAD
        .decode(parts[2])
        .expect("signature segment is base64url");
    if !sig.is_empty() {
        sig[0] ^= 0xFF;
    } else {
        sig.push(0x42);
    }
    let new_sig = URL_SAFE_NO_PAD.encode(&sig);
    parts[2] = &new_sig;
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

/// Mint a token signed by a fresh RSA key whose `kid` is NOT in
/// the realm's JWKS. Header + payload are syntactically valid
/// (correct iss/aud/exp/sub + the `knievel` claim), but the
/// signature won't verify against any key Keycloak hands out —
/// catches a regression where the verifier accidentally accepts
/// a token whose `kid` isn't found in the JWKS by falling back
/// to a stale or unrelated key.
fn mint_token_from_unrelated_key(issuer: &str, audience: &str) -> Result<String> {
    use jsonwebtoken::{encode, EncodingKey, Header};

    // Static test-only RSA private key, traditional RSA DER
    // (PKCS#1) format — `jsonwebtoken` is pulled in with
    // `default-features = false`, which disables the `use_pem`
    // feature, so we feed bytes that `EncodingKey::from_rsa_der`
    // accepts directly. Embedded so the test stays hermetic —
    // no key generation dependency, no network call. The
    // matching public key is never registered with Keycloak,
    // which is the whole point of the test.
    const TEST_RSA_DER: &[u8] = include_bytes!("fixtures/integration_oidc_unrelated_key.der");

    let key = EncodingKey::from_rsa_der(TEST_RSA_DER);

    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("integration-oidc-unrelated-key".into());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let claims = serde_json::json!({
        "iss": issuer,
        "aud": audience,
        "sub": "forged-user",
        "exp": now + 600,
        "iat": now,
        "knievel": {
            "scope": "org",
            "org_id": "scientist-com-pr",
            "role": "editor"
        }
    });

    let token = encode(&header, &claims, &key).context("encoding forged JWT")?;
    Ok(token)
}
