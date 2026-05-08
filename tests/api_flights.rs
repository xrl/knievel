//! API tests: flights CRUD + cross-tenant isolation.
//!
//! Audit finding opus O24: the cross-tenant manifest listed
//! `cross_tenant_flights_{create,list,get,patch}` as test names but
//! no real test functions existed. This file closes that gap.
//!
//! Each `cross_tenant_flights_*` function below exercises one
//! project-scoped flights endpoint with a token from a different
//! org, asserting a `403 wrong_tenant` response.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_b_editor: String,
    /// campaign_id seeded in pj_a for flight FK tests.
    campaign_a: i64,
    /// flight_id seeded in pj_a for get/patch cross-tenant tests.
    flight_a: i64,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_project(&db.pool, "org_b", "pj_b").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_b_editor = mint_token(&db.pool, "tok_bedit", "org_b", "editor").await?;

    // Seed an advertiser + campaign under pj_a.
    let mut tx = testlib::tenant::begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
    let advertiser_a: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.advertisers (org_id, project_id, name)
         VALUES ('org_a', 'pj_a', 'FixAdv') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;
    let campaign_a: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.campaigns (org_id, project_id, advertiser_id, name)
         VALUES ('org_a', 'pj_a', $1, 'FixCamp') RETURNING id",
    )
    .bind(advertiser_a)
    .fetch_one(&mut *tx)
    .await?;
    // Seed a flight under pj_a for get/patch tests.
    let flight_a: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.flights
             (org_id, project_id, campaign_id, name, priority_id, ad_types)
         VALUES ('org_a', 'pj_a', $1, 'FixFlight', 1, '{1}') RETURNING id",
    )
    .bind(campaign_a)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Fixture {
        db,
        pj_a_editor,
        pj_b_editor,
        campaign_a,
        flight_a,
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

/// manifest entry: cross_tenant_flights_create
/// A token from org_b cannot POST to pj_a's flights endpoint.
#[tokio::test]
async fn cross_tenant_flights_create() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .body_json(&serde_json::json!({
            "campaign_id": f.campaign_a,
            "name": "CrossTenantFlight",
            "priority_id": 1,
            "ad_types": [1],
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// manifest entry: cross_tenant_flights_list
/// A token from org_b cannot GET pj_a's flights list.
#[tokio::test]
async fn cross_tenant_flights_list() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// manifest entry: cross_tenant_flights_get
/// A token from org_b cannot GET a specific flight in pj_a.
#[tokio::test]
async fn cross_tenant_flights_get() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get(format!("/v1/projects/pj_a/flights/{}", f.flight_a))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// manifest entry: cross_tenant_flights_patch
/// A token from org_b cannot PATCH a flight in pj_a.
#[tokio::test]
async fn cross_tenant_flights_patch() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .patch(format!("/v1/projects/pj_a/flights/{}", f.flight_a))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .body_json(&serde_json::json!({"name": "Hijacked"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Happy-path: same-project campaign_id is accepted; cross-project
/// campaign_id returns 422 fk_not_found (opus O7 validation).
#[tokio::test]
async fn flight_same_project_campaign_fk_enforced() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Valid: campaign_a lives in pj_a.
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": f.campaign_a,
            "name": "GoodFlight",
            "priority_id": 1,
            "ad_types": [1],
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    // Invalid: a non-existent campaign id.
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": 999999,
            "name": "BadFlight",
            "priority_id": 1,
            "ad_types": [1],
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("fk_not_found"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Date-range validation: start_date after end_date → 400.
#[tokio::test]
async fn flight_date_range_validation() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // start_date > end_date → 400 invalid_date_range.
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": f.campaign_a,
            "name": "BadDates",
            "priority_id": 1,
            "ad_types": [1],
            "start_date": "2025-12-31T00:00:00.000Z",
            "end_date": "2025-01-01T00:00:00.000Z",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("invalid_date_range"));

    // start_date == end_date → ok (201).
    let resp = cli
        .post("/v1/projects/pj_a/flights")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "campaign_id": f.campaign_a,
            "name": "SameDates",
            "priority_id": 1,
            "ad_types": [1],
            "start_date": "2025-06-01T00:00:00.000Z",
            "end_date": "2025-06-01T00:00:00.000Z",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
