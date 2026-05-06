//! API tests: cursor pagination contract from `API.md` §
//! "Pagination."
//!
//! Phase 3.33. Exercises `listAdvertisers` as the canonical
//! representative of the 8 paginated demand+inventory list
//! endpoints — they share `crate::pagination::{resolve,
//! next_cursor}`, so per-resource coverage on any one of them
//! validates the contract for all of them. Per-resource
//! divergence (different SQL table) is caught at compile time.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_a_reader: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org_project(&db.pool, "org_a", "pj_a").await?;
    let pj_a_editor = mint_token(&db.pool, "tok_aedit", "org_a", "editor").await?;
    let pj_a_reader = mint_token(&db.pool, "tok_aread", "org_a", "reader").await?;
    Ok(Fixture {
        db,
        pj_a_editor,
        pj_a_reader,
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

async fn seed_advertisers(
    cli: &TestClient<impl poem::Endpoint>,
    bearer: &str,
    n: usize,
) -> Result<()> {
    for i in 0..n {
        let body = serde_json::json!({
            "name": format!("Adv {i:03}"),
            "external_id": format!("adv_{i:03}"),
        });
        cli.post("/v1/projects/pj_a/advertisers")
            .header("Authorization", format!("Bearer {bearer}"))
            .body_json(&body)
            .send()
            .await
            .assert_status(poem::http::StatusCode::CREATED);
    }
    Ok(())
}

#[tokio::test]
async fn list_uses_default_limit_when_unset() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    seed_advertisers(&cli, &f.pj_a_editor, 75).await?;

    let resp = cli
        .get("/v1/projects/pj_a/advertisers")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 50, "default limit returns 50");
    assert!(
        body["next_cursor"].is_string(),
        "next_cursor present when more pages exist"
    );

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_walks_all_pages_via_cursor() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    seed_advertisers(&cli, &f.pj_a_editor, 23).await?;

    let mut cursor: Option<String> = None;
    let mut walked = 0_usize;
    let mut pages = 0;
    loop {
        pages += 1;
        let url = match &cursor {
            None => "/v1/projects/pj_a/advertisers?limit=10".to_string(),
            Some(c) => format!("/v1/projects/pj_a/advertisers?limit=10&cursor={c}"),
        };
        let resp = cli
            .get(&url)
            .header("Authorization", format!("Bearer {}", f.pj_a_reader))
            .send()
            .await;
        resp.assert_status_is_ok();
        let body: serde_json::Value = resp.json().await.value().deserialize();
        walked += body["items"].as_array().unwrap().len();
        cursor = body["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        assert!(pages < 10, "cursor walk should terminate");
    }
    assert_eq!(walked, 23, "every seeded advertiser surfaced exactly once");
    assert_eq!(pages, 3, "23 items / limit 10 == 3 pages (10+10+3)");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_last_page_has_null_next_cursor() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    seed_advertisers(&cli, &f.pj_a_editor, 5).await?;

    let resp = cli
        .get("/v1/projects/pj_a/advertisers?limit=10")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["items"].as_array().unwrap().len(), 5);
    assert!(
        body["next_cursor"].is_null(),
        "fewer items than limit → no next page"
    );

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_limit_zero_returns_400() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get("/v1/projects/pj_a/advertisers?limit=0")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("invalid_limit"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_limit_overcap_returns_400() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get("/v1/projects/pj_a/advertisers?limit=501")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("invalid_limit"));
    assert!(body["error"]["message"].as_str().unwrap().contains("500"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_garbage_cursor_returns_400() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .get("/v1/projects/pj_a/advertisers?cursor=not-a-cursor")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("invalid_cursor"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn list_cross_resource_cursor_rejected() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));
    // Seed enough campaigns to mint a real `campaigns` cursor.
    for i in 0..15 {
        // Need an advertiser first (FK).
        let adv_body = serde_json::json!({
            "name": format!("Adv {i}"),
            "external_id": format!("adv_for_camp_{i}"),
        });
        let adv_resp = cli
            .post("/v1/projects/pj_a/advertisers")
            .header("Authorization", format!("Bearer {}", f.pj_a_editor))
            .body_json(&adv_body)
            .send()
            .await;
        adv_resp.assert_status(poem::http::StatusCode::CREATED);
        let adv_id = adv_resp
            .json()
            .await
            .value()
            .deserialize::<serde_json::Value>()["id"]
            .as_i64()
            .unwrap();

        let camp_body = serde_json::json!({
            "name": format!("Camp {i}"),
            "external_id": format!("camp_{i}"),
            "advertiser_id": adv_id,
        });
        cli.post("/v1/projects/pj_a/campaigns")
            .header("Authorization", format!("Bearer {}", f.pj_a_editor))
            .body_json(&camp_body)
            .send()
            .await
            .assert_status(poem::http::StatusCode::CREATED);
    }

    // Mint a campaigns cursor.
    let camp_resp = cli
        .get("/v1/projects/pj_a/campaigns?limit=5")
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    camp_resp.assert_status_is_ok();
    let camp_cursor = camp_resp
        .json()
        .await
        .value()
        .deserialize::<serde_json::Value>()["next_cursor"]
        .as_str()
        .expect("campaigns has next page")
        .to_owned();

    // Replay against advertisers.
    let resp = cli
        .get(format!(
            "/v1/projects/pj_a/advertisers?cursor={camp_cursor}"
        ))
        .header("Authorization", format!("Bearer {}", f.pj_a_reader))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["error"]["code"], serde_json::json!("invalid_cursor"));
    let msg = body["error"]["message"].as_str().unwrap();
    assert!(msg.contains("campaigns"), "{msg}");
    assert!(msg.contains("advertisers"), "{msg}");

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
