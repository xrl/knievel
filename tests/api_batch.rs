//! API tests: `:batchUpsert` contract.
//!
//! Phase 3.14. Covers the `API.md` "Write contract" and the
//! `TESTING.md` § 6.4 batch row: 200 success with all rows
//! upserted, 422 batch_partial_failure with deterministic
//! `details[]` on per-row failure, 403 wrong_tenant, transaction
//! rollback on any error.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_b_editor: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_project(&db.pool, "org_b", "pj_b").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_b_editor = mint_token(&db.pool, "tok_bedit", "org_b", "editor").await?;
    Ok(Fixture {
        db,
        pj_a_editor,
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

/// Happy path: a 3-row batch advertisers upsert returns 200 with
/// all three rows, and a re-run of the same external_ids
/// rewrites them in place (etag bumps, ids stable).
#[tokio::test]
async fn batch_upsert_advertisers_round_trip() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = serde_json::json!({
        "items": [
            {"external_id": "a1", "name": "A1"},
            {"external_id": "a2", "name": "A2"},
            {"external_id": "a3", "name": "A3"},
        ]
    });
    let resp = cli
        .post("/v1/projects/pj_a/advertisers:batchUpsert")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status_is_ok();
    let v: serde_json::Value = resp.json().await.value().deserialize();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let id1 = items[0]["id"].as_i64().unwrap();
    let etag1 = items[0]["etag"].as_str().unwrap().to_string();

    // Re-run: same external_ids — ids stable, etag rotates.
    let body2 = serde_json::json!({
        "items": [
            {"external_id": "a1", "name": "A1-renamed"},
            {"external_id": "a2", "name": "A2"},
            {"external_id": "a3", "name": "A3"},
        ]
    });
    let resp = cli
        .post("/v1/projects/pj_a/advertisers:batchUpsert")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&body2)
        .send()
        .await;
    resp.assert_status_is_ok();
    let v: serde_json::Value = resp.json().await.value().deserialize();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["id"].as_i64().unwrap(), id1);
    assert_eq!(items[0]["name"], serde_json::json!("A1-renamed"));
    assert_ne!(items[0]["etag"].as_str().unwrap(), etag1);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Cross-tenant: pj_b's editor can't drive a batch into pj_a.
#[tokio::test]
async fn cross_tenant_advertisers_batch_upsert() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    let body = serde_json::json!({"items": [{"external_id": "x", "name": "X"}]});
    let resp = cli
        .post("/v1/projects/pj_a/advertisers:batchUpsert")
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], serde_json::json!("wrong_tenant"));
    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// All-or-nothing rollback: a campaign batch with one bad
/// advertiser_id rolls back every row and returns
/// `batch_partial_failure` with the offending index.
#[tokio::test]
async fn batch_upsert_campaigns_partial_failure_rolls_back() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create one advertiser to reference legitimately.
    let resp = cli
        .post("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"name": "Acme", "external_id": "acme"}))
        .send()
        .await;
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let adv_id = body["id"].as_i64().unwrap();

    // Index 0 OK, index 1 has a bogus advertiser_id → entire batch
    // rolls back. After the failure, no campaigns visible.
    let body = serde_json::json!({
        "items": [
            {"external_id": "c1", "advertiser_id": adv_id, "name": "C1"},
            {"external_id": "c2", "advertiser_id": 99_999_999, "name": "C2"},
        ]
    });
    let resp = cli
        .post("/v1/projects/pj_a/campaigns:batchUpsert")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let v: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("batch_partial_failure")
    );
    let details = v["error"]["details"].as_array().unwrap();
    assert!(!details.is_empty());
    assert_eq!(details[0]["index"], serde_json::json!(1));
    assert_eq!(details[0]["code"], serde_json::json!("fk_not_found"));
    assert_eq!(details[0]["field"], serde_json::json!("advertiserId"));

    // Confirm rollback: GET list returns zero campaigns.
    let resp = cli
        .get("/v1/projects/pj_a/campaigns")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .send()
        .await;
    resp.assert_status_is_ok();
    let v: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(v["items"].as_array().unwrap().len(), 0);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Placeholder cross-tenant tests for the remaining 5 resources —
/// all share the same `open_project_tx` prologue so a single
/// short test per resource is sufficient (the wrong_tenant branch
/// is the bit that varies). Each self-skips without `DATABASE_URL`.
macro_rules! cross_tenant_batch_test {
    ($name:ident, $path:expr, $row:expr) => {
        #[tokio::test]
        async fn $name() -> Result<()> {
            if std::env::var("DATABASE_URL").is_err() {
                eprintln!("DATABASE_URL not set; skipping.");
                return Ok(());
            }
            let f = setup().await?;
            let cli = TestClient::new(build_app(f.db.pool.clone()));
            let body = serde_json::json!({ "items": [$row] });
            let resp = cli
                .post($path)
                .header("Authorization", format!("Bearer {}", f.pj_b_editor))
                .body_json(&body)
                .send()
                .await;
            let status = resp.0.status();
            if status != poem::http::StatusCode::FORBIDDEN {
                let body_bytes = resp.0.into_body().into_bytes().await.unwrap_or_default();
                let body_str = String::from_utf8_lossy(&body_bytes);
                panic!(
                    "expected 403 FORBIDDEN, got {status}: body={body_str}"
                );
            }
            testlib::db::ephemeral_drop(f.db).await?;
            Ok(())
        }
    };
}

cross_tenant_batch_test!(
    cross_tenant_campaigns_batch_upsert,
    "/v1/projects/pj_a/campaigns:batchUpsert",
    serde_json::json!({"external_id": "x", "advertiser_id": 1, "name": "X"})
);
cross_tenant_batch_test!(
    cross_tenant_flights_batch_upsert,
    "/v1/projects/pj_a/flights:batchUpsert",
    serde_json::json!({
        "external_id": "x", "campaign_id": 1, "name": "X",
        "priority_id": 1, "ad_types": [1]
    })
);
cross_tenant_batch_test!(
    cross_tenant_ads_batch_upsert,
    "/v1/projects/pj_a/ads:batchUpsert",
    serde_json::json!({"external_id": "x", "flight_id": 1, "creative_id": 1})
);
cross_tenant_batch_test!(
    cross_tenant_sites_batch_upsert,
    "/v1/projects/pj_a/sites:batchUpsert",
    serde_json::json!({"external_id": "x", "name": "X", "url": "https://x.test"})
);
cross_tenant_batch_test!(
    cross_tenant_zones_batch_upsert,
    "/v1/projects/pj_a/zones:batchUpsert",
    serde_json::json!({"external_id": "x", "site_id": 1, "name": "X"})
);
