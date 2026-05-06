//! API tests: read-only taxonomy + project-creation seeding.
//!
//! Phase 3.13. Asserts that creating a project seeds default
//! channels / priorities / ad_types under that project, and the
//! read endpoints return them tenant-scoped.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    org_a_admin: String,
    org_b_admin: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;
    seed_org(&db.pool, "org_b").await?;
    let org_a_admin = mint_token(&db.pool, "tok_aadm", "org_a", "org-admin").await?;
    let org_b_admin = mint_token(&db.pool, "tok_badm", "org_b", "org-admin").await?;
    Ok(Fixture {
        db,
        org_a_admin,
        org_b_admin,
    })
}

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
async fn project_creation_seeds_default_taxonomy() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create the project — handler seeds taxonomy in the same
    // transaction as the project insert.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({"name": "P"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let pj = body["id"].as_str().unwrap().to_string();

    // Channels: 3 seeded.
    let resp = cli
        .get(format!("/v1/projects/{pj}/channels"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Web"));
    assert!(names.contains(&"Mobile"));

    // Priorities: 3 seeded, ordered by tier ascending.
    let resp = cli
        .get(format!("/v1/projects/{pj}/priorities"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], serde_json::json!("House"));
    assert_eq!(items[0]["tier"].as_i64(), Some(1));

    // Ad types: 4 seeded, with width/height.
    let resp = cli
        .get(format!("/v1/projects/{pj}/ad-types"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Medium Rectangle"));

    // Cross-tenant: org_b admin can't read org_a's seeded
    // priorities (project_id mismatch resolves to wrong_tenant).
    let resp = cli
        .get(format!("/v1/projects/{pj}/priorities"))
        .header("Authorization", format!("Bearer {}", f.org_b_admin))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
