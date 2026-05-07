//! API tests: admin-UI handshake — `/v1/whoami` and
//! `/admin/config.json`.
//!
//! Phase 7.4. The two endpoints the admin SPA hits at boot:
//! `/admin/config.json` (unauthenticated) for OIDC runtime config,
//! and `/v1/whoami` (authenticated) to confirm the bearer is
//! valid. Both have to work without a real DB so the SPA can
//! render the login screen against a freshly-booted cluster
//! before any tenants exist.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

fn build_app(admin_ui: knievel::config::AdminUiConfig) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_admin_ui(admin_ui);
    knievel::server::routes().data(state)
}

fn build_app_with_db(
    pool: sqlx::PgPool,
    admin_ui: knievel::config::AdminUiConfig,
) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new()
        .with_db(pool)
        .with_admin_ui(admin_ui);
    knievel::server::routes().data(state)
}

#[tokio::test]
async fn admin_config_json_defaults_are_safe() {
    // Empty AdminUiConfig → OIDC disabled (empty issuer +
    // client_id), default scopes, paste-token fallback enabled
    // (require_oidc: false). This is the bootstrap shape: a
    // freshly-deployed cluster with no Keycloak yet.
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli.get("/admin/config.json").send().await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["oidc"]["issuer"], "");
    assert_eq!(body["oidc"]["client_id"], "");
    assert_eq!(body["oidc"]["require_oidc"], false);
    let scopes: Vec<String> = body["oidc"]["scopes"]
        .as_array()
        .expect("scopes is an array")
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert_eq!(scopes, vec!["openid", "profile", "knievel"]);
}

#[tokio::test]
async fn admin_config_json_round_trips_oidc_block() {
    // Populated AdminUiConfig → SPA gets the issuer + client_id
    // + require_oidc back verbatim. No client secret in the
    // payload (PKCE replaces it).
    let mut admin_ui = knievel::config::AdminUiConfig::default();
    admin_ui.oidc.issuer = Some("https://keycloak.example.com/realms/scientist".into());
    admin_ui.oidc.client_id = Some("knievel-admin-ui-prod".into());
    admin_ui.oidc.require_oidc = true;
    let cli = TestClient::new(build_app(admin_ui));

    let resp = cli.get("/admin/config.json").send().await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        body["oidc"]["issuer"],
        "https://keycloak.example.com/realms/scientist"
    );
    assert_eq!(body["oidc"]["client_id"], "knievel-admin-ui-prod");
    assert_eq!(body["oidc"]["require_oidc"], true);
    // No leaks of secrets / unrelated config.
    assert!(
        body.get("client_secret").is_none(),
        "config.json must never carry a client_secret"
    );
}

#[tokio::test]
async fn admin_config_json_is_unauthenticated() {
    // Bootstrap requirement: SPA hits this BEFORE the user
    // logs in, so it must not require a bearer.
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli.get("/admin/config.json").send().await;
    resp.assert_status_is_ok();
}

#[tokio::test]
async fn whoami_without_bearer_is_401() {
    // No DB needed — the bearer parser rejects before any
    // db lookup.
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli.get("/v1/whoami").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whoami_with_garbage_bearer_is_401() {
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli
        .get("/v1/whoami")
        .header("authorization", "Bearer not-a-real-token")
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn whoami_returns_principal_for_valid_bearer() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping whoami DB-backed assertion.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_w").await?;
    let token = mint_token(&db.pool, "tok_who", "org_w", "editor").await?;

    let cli = TestClient::new(build_app_with_db(
        db.pool.clone(),
        knievel::config::AdminUiConfig::default(),
    ));
    let resp = cli
        .get("/v1/whoami")
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["scope"], "org");
    assert_eq!(body["org_id"], "org_w");
    assert_eq!(body["role"], "editor");
    assert_eq!(body["token_type"], "opaque");
    assert!(body["project_id"].is_null());
    assert!(
        body["actor_id"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "actor_id must be non-empty for audit trail",
    );
    Ok(())
}

// Fixture helpers — mirror the shape used in api_taxonomy.rs etc.
// Kept duplicated per CLAUDE.md "Per-resource test files duplicate
// fixture helpers intentionally"; lift to a shared `tests/common`
// module only after the patterns stabilize.

async fn seed_org(pool: &sqlx::PgPool, org: &str) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, org, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org)
        .bind(format!("Org {org}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn mint_token(pool: &sqlx::PgPool, tok_id: &str, org_id: &str, role: &str) -> Result<String> {
    let id_short = tok_id.strip_prefix("tok_").expect("tok_ prefix");
    let secret = format!("s{id_short}");
    let hash = knievel::auth::opaque::hash(&secret)?;
    let mut tx = testlib::tenant::begin_bound(pool, org_id, None).await?;
    sqlx::query(
        "INSERT INTO knievel.api_tokens (id, org_id, scope, role, name, secret_hash)
         VALUES ($1, $2, 'org', $3, $4, $5)",
    )
    .bind(tok_id)
    .bind(org_id)
    .bind(role)
    .bind(format!("{role} fixture"))
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(format!("kvl_test_org_{id_short}_{secret}"))
}
