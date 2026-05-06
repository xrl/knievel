//! API tests: templated creative variant + CreativeTemplate
//! `template` / `templateEngine` extensions.
//!
//! Phase 4.8. Covers the write-side surface:
//!   - CreativeTemplate accepts `template` + `template_engine`,
//!     parses Liquid on write, rejects malformed source with
//!     `422 / template_parse_error`.
//!   - `template_engine_required` and
//!     `template_engine_unsupported` fire on shape errors.
//!   - `templated` creative kind requires `template_id` + `values`
//!     and rejects with `422 / template_missing_body` when the
//!     referenced template has no `template` source.
//!   - End-to-end: write a template with a Liquid body, write a
//!     `templated` creative referencing it, GET round-trips.
//!
//! Decision-time rendering lands as a follow-up (snapshot loader
//! must carry the parsed template); see PHASES.md 4.8 Note.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;
use serde_json::json;

struct F {
    db: testlib::db::EphemeralDb,
    project_id: String,
    advertiser_id: i64,
    admin: String,
}

async fn setup() -> Result<F> {
    let db = testlib::db::ephemeral().await?;
    let out = knievel::cli::seed_demo::run(knievel::cli::seed_demo::SeedDemoArgs {
        database_url: db.url.clone(),
        org_external_id: "templated-org".into(),
        project_external_id: "templated-project".into(),
        token: Some("kvl_dev_org_tplt_secret".into()),
        write_token_to: None,
    })
    .await?;
    Ok(F {
        db,
        project_id: out.project_id,
        advertiser_id: out.advertiser_id,
        admin: out.token,
    })
}

fn build_app(pool: sqlx::PgPool) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_db(pool);
    knievel::server::routes().data(state)
}

fn skip_if_no_db() -> bool {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return true;
    }
    false
}

/// CreativeTemplate accepts a Liquid template + engine pair and
/// echoes them back on GET.
#[tokio::test]
async fn creative_template_with_liquid_round_trips() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = json!({
        "external_id": "tpl-render",
        "name": "renderable",
        "schema": {"type": "object"},
        "template":
            "<a href=\"{{ad.clickUrl}}\"><span>{{values.title}}</span></a>",
        "template_engine": "liquid"
    });
    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let created: serde_json::Value = resp.json().await.value().deserialize();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["template_engine"], json!("liquid"));
    assert!(created["template"]
        .as_str()
        .unwrap()
        .contains("ad.clickUrl"));

    let resp = cli
        .get(format!(
            "/v1/projects/{}/creative-templates/{}",
            f.project_id, id
        ))
        .header("Authorization", format!("Bearer {}", f.admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let got: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(got["template_engine"], json!("liquid"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Liquid that doesn't parse → 422 / template_parse_error.
#[tokio::test]
async fn malformed_liquid_returns_template_parse_error() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = json!({
        "name": "bad",
        "schema": {"type": "object"},
        "template": "{% if x %}",  // unterminated
        "template_engine": "liquid"
    });
    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], json!("template_parse_error"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// `template` without `template_engine` → 422.
#[tokio::test]
async fn template_without_engine_rejected() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = json!({
        "name": "needs-engine",
        "schema": {"type": "object"},
        "template": "{{values.title}}"
    });
    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], json!("template_engine_required"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Unknown engine → 422.
#[tokio::test]
async fn unsupported_engine_rejected() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let body = json!({
        "name": "wrong-engine",
        "schema": {"type": "object"},
        "template": "{{values.title}}",
        "template_engine": "handlebars"
    });
    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&body)
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], json!("template_engine_unsupported"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// `templated` creative referencing a validation-only template
/// (no `template` source) → 422 / template_missing_body.
#[tokio::test]
async fn templated_creative_requires_template_body() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create a validation-only CreativeTemplate (no `template`).
    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&json!({
            "name": "validate-only",
            "schema": {"type": "object", "required": ["title"]}
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let tpl: serde_json::Value = resp.json().await.value().deserialize();
    let template_id = tpl["id"].as_i64().unwrap();

    // Try to write a templated creative against it.
    let resp = cli
        .post(format!("/v1/projects/{}/creatives", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&json!({
            "advertiser_id": f.advertiser_id,
            "kind": "templated",
            "template_id": template_id,
            "values": {"title": "Hi"}
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::UNPROCESSABLE_ENTITY);
    let err: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(err["error"]["code"], json!("template_missing_body"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// End-to-end: a CreativeTemplate with Liquid + a `templated`
/// creative referencing it round-trips.
#[tokio::test]
async fn templated_creative_round_trip() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    let resp = cli
        .post(format!("/v1/projects/{}/creative-templates", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&json!({
            "name": "renderable-card",
            "schema": {"type": "object"},
            "template": "<div>{{values.title}}</div>",
            "template_engine": "liquid"
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let tpl: serde_json::Value = resp.json().await.value().deserialize();
    let template_id = tpl["id"].as_i64().unwrap();

    let resp = cli
        .post(format!("/v1/projects/{}/creatives", f.project_id))
        .header("Authorization", format!("Bearer {}", f.admin))
        .body_json(&json!({
            "advertiser_id": f.advertiser_id,
            "kind": "templated",
            "template_id": template_id,
            "values": {"title": "Hi"},
            "click_through_url": "https://demo.example.com/landing"
        }))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let cre: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(cre["kind"], json!("templated"));
    assert_eq!(cre["template_id"], json!(template_id));
    assert_eq!(cre["values"]["title"], json!("Hi"));

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}
