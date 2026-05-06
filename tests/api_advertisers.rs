//! API tests: advertisers CRUD.
//!
//! Phase 3.8. Covers the basic CRUD contract from `TESTING.md`
//! § 6.4 (`create_returns_201`, read 404, round-trip,
//! cross-tenant). Etag/pagination/batch-upsert tests come online
//! as their handler features land.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    /// Plaintext bearer for `(org_a, pj_a)` editor.
    pj_a_editor: String,
    /// Plaintext bearer for `(org_a, pj_a)` reader.
    pj_a_reader: String,
    /// Plaintext bearer for `(org_b, pj_b)` editor.
    pj_b_editor: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_project(&db.pool, "org_b", "pj_b").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_a_reader = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;
    let pj_b_editor = mint_token(&db.pool, "tok_bedit", "org_b", "editor").await?;
    Ok(Fixture {
        db,
        pj_a_editor,
        pj_a_reader,
        pj_b_editor,
    })
}

async fn seed_org_project(pool: &sqlx::PgPool, org: &str, proj: &str) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, org, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org)
        .bind(format!("Org {org}"))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(proj)
        .bind(org)
        .bind(format!("Project {proj}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn mint_token(pool: &sqlx::PgPool, tok_id: &str, org_id: &str, role: &str) -> Result<String> {
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
async fn create_advertiser_returns_201() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"name": "Acme", "external_id": "acme"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert!(body["id"].as_i64().unwrap() > 0);
    assert_eq!(body["external_id"], serde_json::json!("acme"));
    assert_eq!(body["name"], serde_json::json!("Acme"));
    assert_eq!(body["is_active"], serde_json::json!(true));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_advertiser_external_id_conflict_returns_409() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = serde_json::json!({"name": "Acme", "external_id": "acme"});
    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CONFLICT);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        err["error"]["code"],
        serde_json::json!("external_id_conflict")
    );

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn cross_tenant_advertisers_get() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create one in pj_a; pj_b's editor cannot see it.
    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"name": "AcmeA"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();

    // pj_b editor → pj_a's resource → 403 wrong_tenant.
    let resp = cli
        .get(format!("/v1/projects/pj_a/advertisers/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn create_advertiser_role_insufficient() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .body_json(&serde_json::json!({"name": "x"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("role_insufficient"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_get_patch_round_trip() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create.
    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"name": "RoundTrip"}))
        .send()
        .await;
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    let original_etag = body["etag"].as_str().unwrap().to_string();

    // List shows it.
    let resp = cli
        .get("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert!(items.iter().any(|i| i["id"].as_i64() == Some(id)));

    // GET by id.
    let resp = cli
        .get(format!("/v1/projects/pj_a/advertisers/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["name"], serde_json::json!("RoundTrip"));

    // PATCH name; etag changes.
    let resp = cli
        .patch(format!("/v1/projects/pj_a/advertisers/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"name": "Renamed"}))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["name"], serde_json::json!("Renamed"));
    assert_ne!(body["etag"].as_str().unwrap(), original_etag);

    // GET unknown id → 404.
    let resp = cli
        .get("/v1/projects/pj_a/advertisers/9999999")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::NOT_FOUND);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
