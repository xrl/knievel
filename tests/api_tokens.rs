//! API tests: `POST/GET/DELETE /v1/orgs/{orgId}/tokens`.
//!
//! Phase 3.6. Spec refs:
//!   - `API.md` § 2.2 (Tokens)
//!   - `AUTH.md` "Opaque Tokens", "Endpoint -> minimum role"
//!   - `REQUIREMENTS.md` § 7.3 (audit_log writers)
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    /// Plaintext bearer for `org_a`'s `org-admin` token.
    org_a_admin: String,
    /// Plaintext bearer for `org_b`'s `org-admin` token.
    org_b_admin: String,
    /// Plaintext bearer for `org_a`'s `editor` token.
    org_a_editor: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a", "Org A").await?;
    seed_org(&db.pool, "org_b", "Org B").await?;
    let org_a_admin = mint_seed_token(&db.pool, "tok_aadmin", "org_a", "org-admin").await?;
    let org_b_admin = mint_seed_token(&db.pool, "tok_badmin", "org_b", "org-admin").await?;
    let org_a_editor = mint_seed_token(&db.pool, "tok_aeditor", "org_a", "editor").await?;
    Ok(Fixture {
        db,
        org_a_admin,
        org_b_admin,
        org_a_editor,
    })
}

async fn seed_org(pool: &sqlx::PgPool, id: &str, name: &str) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, id, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Insert a token row directly (bootstrap path; mirrors what
/// `knievel-cli seed-demo` will do in Phase 4.2). Returns the
/// plaintext bearer the test client can present.
async fn mint_seed_token(
    pool: &sqlx::PgPool,
    tok_id: &str,
    org_id: &str,
    role: &str,
) -> Result<String> {
    let id_short = tok_id.strip_prefix("tok_").expect("tok_ prefix");
    let secret = format!("s{}", id_short);
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

fn build_app(pool: sqlx::PgPool) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_db(pool);
    knievel::server::routes().data(state)
}

#[tokio::test]
async fn create_token_returns_201_with_plaintext_secret() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({
            "name": "ci sync",
            "scope": "org",
            "role": "editor",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert!(body["id"].as_str().unwrap().starts_with("tok_"));
    assert!(body["secret"]
        .as_str()
        .unwrap()
        .starts_with("kvl_prod_org_"));
    assert_eq!(body["name"], serde_json::json!("ci sync"));
    assert_eq!(body["scope"], serde_json::json!("org"));
    assert_eq!(body["role"], serde_json::json!("editor"));

    // Audit row exists for the mint.
    let mut tx = testlib::tenant::begin_bound(&f.db.pool, "org_a", None).await?;
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM knievel.audit_log
         WHERE operation = 'tokens.mint'",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    assert_eq!(audit_count, 1, "mint must emit one audit_log row");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn newly_minted_token_authenticates() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Mint an org-admin token.
    let resp = cli
        .post("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({
            "name": "rotated",
            "scope": "org",
            "role": "org-admin",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let new_secret = body["secret"].as_str().unwrap().to_string();

    // Use it to create a project — proves the freshly-minted secret
    // verifies through the same auth path that gates everything else.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {new_secret}"))
        .body_json(&serde_json::json!({"name": "via-minted-token"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_token_cross_org_forbidden() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_b_admin))
        .body_json(&serde_json::json!({
            "name": "x",
            "scope": "org",
            "role": "editor",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_token_role_insufficient_for_editor() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Editor token (role < org-admin) cannot mint.
    let resp = cli
        .post("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_a_editor))
        .body_json(&serde_json::json!({
            "name": "x",
            "scope": "org",
            "role": "reader",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        body["error"]["code"],
        serde_json::json!("role_insufficient")
    );

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_tokens_excludes_secrets_and_is_tenant_scoped() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().expect("items");
    assert!(!items.is_empty(), "should see at least the seeded tokens");
    for item in items {
        assert!(
            item.get("secret").is_none(),
            "list response must not expose secret: {item}"
        );
        assert_eq!(
            item["id"].as_str().map(|s| s.starts_with("tok_")),
            Some(true)
        );
    }
    // Org A sees only its own tokens (org_a_admin and org_a_editor) -
    // never org_b's.
    let ids: Vec<String> = items
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&"tok_aadmin".to_string()));
    assert!(ids.contains(&"tok_aeditor".to_string()));
    assert!(!ids.contains(&"tok_badmin".to_string()));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn revoke_token_blocks_subsequent_auth() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Mint a fresh org-admin token to revoke.
    let resp = cli
        .post("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({
            "name": "doomed",
            "scope": "org",
            "role": "org-admin",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_str().unwrap().to_string();
    let secret = body["secret"].as_str().unwrap().to_string();

    // Confirm it works.
    let resp = cli
        .get("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await;
    resp.assert_status_is_ok();

    // Revoke it.
    let resp = cli
        .delete(format!("/v1/orgs/org_a/tokens/{id}"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::NO_CONTENT);

    // Same token now 401s — auth filters revoked rows at the DB.
    let resp = cli
        .get("/v1/orgs/org_a/tokens")
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);

    // Second revoke on the same id 404s ("token not found or
    // already revoked").
    let resp = cli
        .delete(format!("/v1/orgs/org_a/tokens/{id}"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::NOT_FOUND);

    // Audit log records both mint + revoke.
    let mut tx = testlib::tenant::begin_bound(&f.db.pool, "org_a", None).await?;
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM knievel.audit_log
         WHERE operation IN ('tokens.mint', 'tokens.revoke')",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    assert!(audit_count >= 2, "expected mint+revoke audit rows");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
