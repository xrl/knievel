//! API tests: ads (inline-creative variant only).
//!
//! Phase 3.11. The library-reference variant lands in 3.28.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_b_editor: String,
    flight_id: i64,
    creative_id: i64,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_project(&db.pool, "org_b", "pj_b").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_b_editor = mint_token(&db.pool, "tok_bedit", "org_b", "editor").await?;

    let mut tx = testlib::tenant::begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
    let advertiser_id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.advertisers (org_id, project_id, name)
         VALUES ('org_a', 'pj_a', 'Acme') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;
    let campaign_id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.campaigns (org_id, project_id, advertiser_id, name)
         VALUES ('org_a', 'pj_a', $1, 'Spring') RETURNING id",
    )
    .bind(advertiser_id)
    .fetch_one(&mut *tx)
    .await?;
    let flight_id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.flights
             (org_id, project_id, campaign_id, name, priority_id, ad_types)
         VALUES ('org_a', 'pj_a', $1, 'F', 1, '{1}'::bigint[]) RETURNING id",
    )
    .bind(campaign_id)
    .fetch_one(&mut *tx)
    .await?;
    let creative_id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.creatives
             (org_id, project_id, advertiser_id, kind, image_url,
              click_through_url)
         VALUES ('org_a', 'pj_a', $1, 'image', 'https://x', 'https://t')
         RETURNING id",
    )
    .bind(advertiser_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Fixture {
        db,
        pj_a_editor,
        pj_b_editor,
        flight_id,
        creative_id,
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
async fn ad_inline_create_round_trip_and_cross_tenant() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create with default weight.
    let resp = cli
        .post("/v1/projects/pj_a/ads")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "flight_id": f.flight_id,
            "creative_id": f.creative_id,
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    assert_eq!(body["weight"].as_i64(), Some(100));
    assert_eq!(body["creative_id"].as_i64(), Some(f.creative_id));
    assert!(body["ad_library_item_id"].is_null());

    // PATCH weight.
    let resp = cli
        .patch(format!("/v1/projects/pj_a/ads/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"weight": 50}))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["weight"].as_i64(), Some(50));

    // Cross-tenant get → 403.
    let resp = cli
        .get(format!("/v1/projects/pj_a/ads/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);

    // Bad FK → 422.
    let resp = cli
        .post("/v1/projects/pj_a/ads")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "flight_id": 999999,
            "creative_id": f.creative_id,
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
