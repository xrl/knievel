//! API tests: `POST /v1/orgs/{orgId}/projects` and
//! `GET /v1/orgs/{orgId}/projects/{projectId}`.
//!
//! Phase 3.3. Spec refs:
//!   - `API.md` § 2.1 (Projects)
//!   - `AUTH.md` "Authorization" (org match, role check)
//!   - `TESTING.md` § 6.4 (`create_returns_201`), § 6.5
//!     (cross-tenant negative)
//!
//! Skipped when `DATABASE_URL` is not set; runs against the CI
//! Postgres service container.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

/// A fully-bootstrapped fixture: ephemeral DB with two orgs, each
/// with one minted token whose plaintext is returned alongside the
/// pool. Tests assemble a `TestClient` over knievel's production
/// routes pointed at this DB.
struct Fixture {
    db: testlib::db::EphemeralDb,
    /// Plaintext bearer for `org_a`'s `OrgAdmin` token.
    org_a_admin: String,
    /// Plaintext bearer for `org_b`'s `OrgAdmin` token.
    org_b_admin: String,
    /// Plaintext bearer for `org_a`'s `Reader` token.
    org_a_reader: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;

    // Seed two orgs, plus tokens. RLS gates writes on
    // `WITH CHECK (org_id = knievel.org_id)` — we open one
    // tenant-bound transaction per org to land its rows.
    seed_org(&db.pool, "org_a", "Org A").await?;
    seed_org(&db.pool, "org_b", "Org B").await?;
    let org_a_admin = mint_token(&db.pool, "tok_aadmin", "org_a", "org-admin").await?;
    let org_b_admin = mint_token(&db.pool, "tok_badmin", "org_b", "org-admin").await?;
    let org_a_reader = mint_token(&db.pool, "tok_areader", "org_a", "reader").await?;

    Ok(Fixture {
        db,
        org_a_admin,
        org_b_admin,
        org_a_reader,
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

/// Mint an org-scoped opaque token with the given role. Returns
/// the plaintext token. The wire format is
/// `kvl_test_org_<id_short>_<secret>` and the `id_short` is
/// `tok_id` minus its `tok_` prefix.
async fn mint_token(pool: &sqlx::PgPool, tok_id: &str, org_id: &str, role: &str) -> Result<String> {
    let id_short = tok_id.strip_prefix("tok_").expect("tok_ prefix");
    let secret = format!("s{}", id_short); // any non-empty string
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
async fn create_project_returns_201() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({
            "name": "First Project",
            "external_id": "proj-1",
        }))
        .send()
        .await;

    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert!(
        body["id"].as_str().is_some_and(|s| s.starts_with("pj_")),
        "id should be pj_<...>: {body}"
    );
    assert_eq!(body["external_id"], serde_json::json!("proj-1"));
    assert_eq!(body["name"], serde_json::json!("First Project"));
    assert_eq!(body["is_active"], serde_json::json!(true));
    assert!(body["etag"].as_str().is_some());
    assert!(body["created_at"].as_str().is_some());

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_cross_org_forbidden() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // org_b admin tries to create a project under org_a.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_b_admin))
        .body_json(&serde_json::json!({"name": "X"}))
        .send()
        .await;

    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_role_insufficient() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // org_a reader tries to create a project — role too low.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_reader))
        .body_json(&serde_json::json!({"name": "X"}))
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
async fn create_project_missing_auth_returns_401() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .body_json(&serde_json::json!({"name": "X"}))
        .send()
        .await;

    resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_bad_token_returns_401() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", "Bearer kvl_test_org_aadmin_wrongsecret")
        .body_json(&serde_json::json!({"name": "X"}))
        .send()
        .await;

    resp.assert_status(poem::http::StatusCode::UNAUTHORIZED);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_idempotency_key_replay() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    let body = serde_json::json!({"name": "Idem", "external_id": "idem-1"});

    // First call: 201 Created.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "abc-123")
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let first: serde_json::Value = resp.json().await.value().deserialize();
    let first_id = first["id"].as_str().unwrap().to_string();

    // Replay: same key, same body → 201 + Idempotent-Replay: true,
    // same id, no second row.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "abc-123")
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    resp.assert_header("Idempotent-Replay", "true");
    let replay: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(replay["id"].as_str().unwrap(), first_id);

    let count: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.projects WHERE org_id = 'org_a'")
            .fetch_one(&f.db.pool)
            .await?;
    assert_eq!(count, 1, "replay must not produce a second project row");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_idempotency_key_body_mismatch_409() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // First call.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "key-x")
        .body_json(&serde_json::json!({"name": "Original"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    // Second call: same key, different body → 409 idempotency_conflict.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "key-x")
        .body_json(&serde_json::json!({"name": "Different"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        body["error"]["code"],
        serde_json::json!("idempotency_conflict")
    );

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_project_idempotency_whitespace_stable() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // First call with compact JSON.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "ws-key")
        .body_json(&serde_json::json!({"name": "WS", "external_id": "ws-1"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    // Second call with the same logical body but extra whitespace
    // is treated as a replay (`body_hash` canonicalizes).
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .header("Idempotency-Key", "ws-key")
        .header("Content-Type", "application/json")
        .body(r#"{ "name" :  "WS",  "external_id":   "ws-1" }"#)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    resp.assert_header("Idempotent-Replay", "true");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn get_project_round_trips_through_create() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({"name": "RT"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_str().expect("id").to_string();

    let resp = cli
        .get(format!("/v1/orgs/org_a/projects/{id}"))
        .header("Authorization", format!("Bearer {}", f.org_a_reader))
        .send()
        .await;
    resp.assert_status_is_ok();
    let got: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(got["id"].as_str(), Some(id.as_str()));
    assert_eq!(got["name"], serde_json::json!("RT"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
