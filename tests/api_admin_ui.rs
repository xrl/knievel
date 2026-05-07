//! API tests for the admin-UI static-files mount. Phase 7.11.
//!
//! The mount is gated on `cfg.admin_ui.static_dir`. Empty /
//! unset → `/admin/*` returns 404 (headless API mode). Set →
//! poem's `StaticFilesEndpoint` serves the bundle with
//! `index.html` fallback for SPA history routing.
//!
//! `/admin/config.json` is registered as a specific `at()`
//! before the static nest, so a `config.json` inside the
//! bundle can't shadow it.

use std::fs;
use std::path::PathBuf;

use poem::test::TestClient;
use poem::EndpointExt;

fn fixture_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("knievel-admin-ui-{label}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("assets")).expect("mkdir fixture");
    fs::write(
        dir.join("index.html"),
        b"<!doctype html><html><body>SPA shell</body></html>",
    )
    .expect("write index.html");
    fs::write(dir.join("assets").join("app.js"), b"export const x = 1;").expect("write asset");
    dir
}

fn build_app(admin_ui: knievel::config::AdminUiConfig) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_admin_ui(admin_ui.clone());
    let r = knievel::server::routes();
    let r = knievel::server::mount_admin_ui(r, admin_ui.static_dir.as_deref());
    r.data(state)
}

#[tokio::test]
async fn unset_static_dir_returns_404_on_admin_root() {
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli.get("/admin/").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
    let resp = cli.get("/admin/orgs/foo").send().await;
    assert_eq!(resp.0.status(), poem::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn set_static_dir_serves_index_html() {
    let dir = fixture_dir("set");
    let cfg = knievel::config::AdminUiConfig {
        static_dir: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let cli = TestClient::new(build_app(cfg));

    let resp = cli.get("/admin/").send().await;
    resp.assert_status_is_ok();
    let body = resp.0.into_body().into_string().await.expect("body string");
    assert!(body.contains("SPA shell"), "body: {body}");
}

#[tokio::test]
async fn deep_path_falls_back_to_index_for_spa_routing() {
    let dir = fixture_dir("spa-fallback");
    let cfg = knievel::config::AdminUiConfig {
        static_dir: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let cli = TestClient::new(build_app(cfg));

    // The SPA's client-side router handles paths like
    // `/admin/orgs/foo/projects/bar`; on a deep-link refresh
    // the static endpoint must serve index.html, not 404.
    let resp = cli.get("/admin/orgs/foo/projects/bar").send().await;
    resp.assert_status_is_ok();
    let body = resp.0.into_body().into_string().await.expect("body string");
    assert!(body.contains("SPA shell"), "body: {body}");
}

#[tokio::test]
async fn config_json_is_not_shadowed_by_bundle_file() {
    // Even when a `config.json` exists inside the bundle, the
    // API's `/admin/config.json` handler wins because it's
    // registered as a specific `at()` before the static nest.
    let dir = fixture_dir("shadow");
    fs::write(
        dir.join("config.json"),
        b"{\"oidc\":{\"issuer\":\"BUNDLED-NOT-RUNTIME\",\"client_id\":\"\",\"scopes\":[],\"require_oidc\":false}}",
    )
    .expect("write bundled config");

    let admin = knievel::config::AdminUiConfig {
        static_dir: Some(dir.to_string_lossy().into_owned()),
        oidc: knievel::config::AdminUiOidcConfig {
            issuer: Some("https://kc.example/realms/x".into()),
            client_id: Some("knievel-admin-ui".into()),
            ..Default::default()
        },
    };

    let cli = TestClient::new(build_app(admin));
    let resp = cli.get("/admin/config.json").send().await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    assert_eq!(body["oidc"]["issuer"], "https://kc.example/realms/x");
    assert_ne!(
        body["oidc"]["issuer"], "BUNDLED-NOT-RUNTIME",
        "runtime API handler must win over the bundle file"
    );
}

#[tokio::test]
async fn unset_static_dir_keeps_config_json_working() {
    // Headless API mode (no SPA bundle) must still answer
    // /admin/config.json so a separately-deployed SPA could
    // reach it from a different origin.
    let cli = TestClient::new(build_app(knievel::config::AdminUiConfig::default()));
    let resp = cli.get("/admin/config.json").send().await;
    resp.assert_status_is_ok();
}
