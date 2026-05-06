//! API tests: creatives + creative_templates.
//!
//! Phase 3.10. Includes the cross-cutting risk #1 spike — round
//! a JSON Schema document through `POST /creative-templates` and
//! `GET /creative-templates/{id}`, asserting the parsed JSON value
//! is preserved bit-for-bit through poem-openapi's typed surface.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    pj_a_editor: String,
    pj_b_editor: String,
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
         VALUES ('org_a', 'pj_a', 'Acme') RETURNING id",
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
async fn creative_template_json_schema_round_trips() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // A representative JSON Schema document — properties block,
    // required array, additionalProperties, mixed types, nesting.
    // This is the shape `API.md` § 3.6 documents.
    let schema = serde_json::json!({
        "type": "object",
        "required": ["title", "body", "ctaText"],
        "properties": {
            "title":    { "type": "string", "maxLength": 80 },
            "body":     { "type": "string", "maxLength": 240 },
            "imageUrl": { "type": "string", "format": "uri" },
            "ctaText":  { "type": "string", "maxLength": 24 },
            "tags":     { "type": "array", "items": { "type": "string" } },
        },
        "additionalProperties": false,
    });

    let resp = cli
        .post("/v1/projects/pj_a/creative-templates")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "name": "sponsored_card_v1",
            "schema": schema,
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    // Schema preserved bit-for-bit on the create response.
    assert_eq!(body["schema"], schema, "schema preserved on create");
    assert_eq!(body["version"].as_i64(), Some(1));

    // GET round-trip — same schema.
    let resp = cli
        .get(format!("/v1/projects/pj_a/creative-templates/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["schema"], schema, "schema preserved on GET");

    // PATCH the schema; version bumps.
    let new_schema = serde_json::json!({"type": "object", "properties": {}});
    let resp = cli
        .patch(format!("/v1/projects/pj_a/creative-templates/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({"schema": new_schema}))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["schema"], new_schema);
    assert_eq!(body["version"].as_i64(), Some(2));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

#[tokio::test]
async fn creative_kind_validation_and_round_trip() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Bad kind → 400.
    let resp = cli
        .post("/v1/projects/pj_a/creatives")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "advertiser_id": f.advertiser_a,
            "kind": "video",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);

    // image without image_url → 400.
    let resp = cli
        .post("/v1/projects/pj_a/creatives")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "advertiser_id": f.advertiser_a,
            "kind": "image",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::BAD_REQUEST);

    // Valid image creative.
    let resp = cli
        .post("/v1/projects/pj_a/creatives")
        .header("Authorization", format!("Bearer {}", f.pj_a_editor))
        .body_json(&serde_json::json!({
            "advertiser_id": f.advertiser_a,
            "kind": "image",
            "image_url": "https://cdn.example/banner.jpg",
            "width": 728,
            "height": 90,
            "alt": "Spring sale",
            "click_through_url": "https://acme.example/sale",
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let id = body["id"].as_i64().unwrap();
    assert_eq!(body["kind"], serde_json::json!("image"));
    assert_eq!(body["width"].as_i64(), Some(728));

    // GET cross-tenant → 403.
    let resp = cli
        .get(format!("/v1/projects/pj_a/creatives/{id}"))
        .header("Authorization", format!("Bearer {}", f.pj_b_editor))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
