//! API tests: campaigns + flights happy path + cross-tenant.
//!
//! Phase 3.9. Lean coverage — the deeper contract (idempotency,
//! batch-upsert, etag) lives in `api_advertisers.rs` and applies
//! by construction since these handlers share the same shape.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_b_editor: String,
    /// Advertiser id seeded under pj_a so campaign tests have a
    /// valid FK target.
    advertiser_a: i64,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_project(&db.pool, "org_b", "pj_b").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_b_editor = mint_token(&db.pool, "tok_bedit", "org_b", "editor").await?;

    let mut tx = testlib::tenant::begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
    let advertiser_a: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.advertisers (org_id, project_id, name)
         VALUES ('org_a', 'pj_a', 'AcmeFix') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Fixture {
        db,
        pj_a_editor,
        pj_b_editor,
        advertiser_a,
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
async fn campaign_create_round_trip_and_cross_tenant() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create.
    let resp = cli
        .post("/v1/projects/pj_a/campaigns")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "advertiser_id": f.advertiser_a,
            "name": "Spring",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    assert_eq!(body["advertiser_id"].as_i64(), Some(f.advertiser_a));

    // GET.
    let resp = cli
        .get(format!("/v1/projects/pj_a/campaigns/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .send()
        .await;
    resp.assert_status_is_ok();

    // Cross-tenant.
    let resp = cli
        .get(format!("/v1/projects/pj_a/campaigns/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    // Bad FK → 422 fk_not_found.
    let resp = cli
        .post("/v1/projects/pj_a/campaigns")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"advertiser_id": 999999, "name": "Bad"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("fk_not_found"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn flight_create_with_arrays_and_validation() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Need a campaign first.
    let resp = cli
        .post("/v1/projects/pj_a/campaigns")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "advertiser_id": f.advertiser_a,
            "name": "Spring",
        }))
        .send()
        .await;
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let campaign_id = body["id"].as_i64().unwrap();

    // Empty ad_types → 400.
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": campaign_id,
            "name": "F0",
            "priority_id": 1,
            "ad_types": [],
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("ad_types_required"));

    // Happy path with arrays.
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": campaign_id,
            "name": "F1",
            "priority_id": 1,
            "site_ids": [10, 20],
            "ad_types": [16, 17],
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    assert_eq!(body["site_ids"], serde_json::json!([10, 20]));
    assert_eq!(body["zone_ids"], serde_json::json!([]));
    assert_eq!(body["ad_types"], serde_json::json!([16, 17]));

    // Cross-tenant.
    let resp = cli
        .get(format!("/v1/projects/pj_a/flights/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
