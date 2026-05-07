//! API tests for the admin SPA's org-browser handshake —
//! `getOrg` and `listProjects` (Phase 7.5). Each requires a
//! reader-or-higher bearer scoped to the org in the path;
//! cross-tenant access returns 403 wrong_tenant.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

fn build_app(pool: sqlx::PgPool) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_db(pool);
    knievel::server::routes().data(state)
}

#[tokio::test]
async fn get_org_returns_metadata_for_owner_principal() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a", "Org Alpha").await?;
    let token = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;

    let cli = TestClient::new(build_app(db.pool.clone()));
    let resp = cli
        .get("/v1/orgs/org_a")
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["id"], "org_a");
    assert_eq!(body["name"], "Org Alpha");
    assert!(body["created_at"].as_str().unwrap().contains('T'));
    Ok(())
}

#[tokio::test]
async fn get_org_cross_tenant_returns_403() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a", "Org Alpha").await?;
    seed_org(&db.pool, "org_b", "Org Bravo").await?;
    let token_a = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;

    let cli = TestClient::new(build_app(db.pool.clone()));
    let resp = cli
        .get("/v1/orgs/org_b")
        .header("authorization", format!("Bearer {token_a}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn list_projects_returns_envelope_with_null_cursor() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a", "Org Alpha").await?;
    seed_project(&db.pool, "org_a", "pj_one", "Project One").await?;
    seed_project(&db.pool, "org_a", "pj_two", "Project Two").await?;
    let token = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;

    let cli = TestClient::new(build_app(db.pool.clone()));
    let resp = cli
        .get("/v1/orgs/org_a/projects")
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);
    let names: Vec<&str> = items
        .iter()
        .filter_map(|i| i["name"].as_str())
        .collect();
    assert!(names.contains(&"Project One"));
    assert!(names.contains(&"Project Two"));
    // 7.5 wires the envelope but doesn't paginate (TEXT-id
    // tuple cursor lands in 6.5).
    assert!(body["next_cursor"].is_null());
    Ok(())
}

#[tokio::test]
async fn list_projects_other_org_returns_403() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a", "Org Alpha").await?;
    seed_org(&db.pool, "org_b", "Org Bravo").await?;
    let token_a = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;

    let cli = TestClient::new(build_app(db.pool.clone()));
    let resp = cli
        .get("/v1/orgs/org_b/projects")
        .header("authorization", format!("Bearer {token_a}"))
        .send()
        .await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::FORBIDDEN);
    Ok(())
}

// --- fixture helpers (duplicated per CLAUDE.md convention) ---

async fn seed_org(pool: &sqlx::PgPool, org: &str, name: &str) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, org, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn seed_project(
    pool: &sqlx::PgPool,
    org_id: &str,
    project_id: &str,
    name: &str,
) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, org_id, None).await?;
    sqlx::query(
        "INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)",
    )
    .bind(project_id)
    .bind(org_id)
    .bind(name)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn mint_token(
    pool: &sqlx::PgPool,
    tok_id: &str,
    org_id: &str,
    role: &str,
) -> Result<String> {
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
