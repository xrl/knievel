//! API tests: CORS middleware behavior.
//!
//! Phase 7.2. Empty `cfg.api.allowed_origins` → no middleware
//! installed (no preflight overhead, no `Access-Control-Allow-Origin`
//! on responses); non-empty → poem's Cors layer wraps the route
//! with the header/method/origin allow-lists from `UI.md` "CORS".
//!
//! No DB needed — the system endpoints (`/healthz`) are served
//! regardless of `database.url`, and CORS behavior is request-level
//! middleware.

use poem::http::{header, StatusCode};
use poem::test::TestClient;
use poem::{Endpoint, EndpointExt};

const ALLOWED: &str = "http://localhost:5173";
const NOT_ALLOWED: &str = "https://evil.example.com";

fn build_app(origins: Vec<String>) -> impl Endpoint {
    let cfg = config_with_origins(origins);
    let routes = knievel::server::routes().data(knievel::state::AppState::new());
    match knievel::server::cors_layer(&cfg) {
        Some(cors) => routes.with(cors).boxed(),
        None => routes.boxed(),
    }
}

fn config_with_origins(origins: Vec<String>) -> knievel::config::Config {
    let mut cfg = knievel::config::Config::default();
    cfg.api.allowed_origins = origins;
    cfg
}

#[tokio::test]
async fn empty_config_does_not_install_middleware() {
    // No CORS layer → preflight OPTIONS hits poem's default
    // method-not-allowed handler on /healthz (which only serves
    // GET), and a regular GET with an Origin header gets no
    // Access-Control-Allow-Origin in the response.
    let cli = TestClient::new(build_app(vec![]));

    // Sanity: a plain GET still works.
    let resp = cli.get("/healthz").send().await;
    resp.assert_status_is_ok();
    assert!(
        resp.0
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "ACAO must not appear when CORS is disabled",
    );

    // GET with an Origin header — still no CORS headers added,
    // since the middleware isn't in the chain.
    let resp = cli
        .get("/healthz")
        .header(header::ORIGIN, ALLOWED)
        .send()
        .await;
    resp.assert_status_is_ok();
    assert!(
        resp.0
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "ACAO must not appear when CORS is disabled (Origin header is irrelevant)",
    );

    // Preflight OPTIONS to /healthz — poem's default routing
    // doesn't have an OPTIONS handler for /healthz, so the
    // method routing layer rejects with 405 Method Not Allowed.
    let resp = cli
        .options("/healthz")
        .header(header::ORIGIN, ALLOWED)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .send()
        .await;
    assert_eq!(
        resp.0.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "no CORS layer → no preflight handling, just method routing"
    );
}

#[tokio::test]
async fn matching_origin_echoes_back_in_response() {
    let cli = TestClient::new(build_app(vec![ALLOWED.into()]));

    let resp = cli
        .get("/healthz")
        .header(header::ORIGIN, ALLOWED)
        .send()
        .await;
    resp.assert_status_is_ok();
    assert_eq!(
        resp.0
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some(ALLOWED),
        "ACAO must echo the matching Origin",
    );

    // Bearer-token model means credentials are off — the header
    // must NOT appear so wildcard origins stay viable later.
    assert!(
        resp.0
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .is_none(),
        "Allow-Credentials must not be set; we use bearer tokens, not cookies",
    );
}

#[tokio::test]
async fn non_matching_origin_is_rejected() {
    let cli = TestClient::new(build_app(vec![ALLOWED.into()]));

    let resp = cli
        .get("/healthz")
        .header(header::ORIGIN, NOT_ALLOWED)
        .send()
        .await;
    // poem's Cors returns CorsError::OriginNotAllowed which
    // surfaces as 401 (its `error_response` impl). Either way,
    // there must be no ACAO header in the response — that's
    // the security-relevant assertion.
    assert!(
        resp.0
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "ACAO must not echo a disallowed Origin",
    );
    assert!(
        !resp.0.status().is_success(),
        "request must not pass when Origin is disallowed (got {})",
        resp.0.status(),
    );
}

#[tokio::test]
async fn preflight_returns_allow_headers_methods_and_max_age() {
    let cli = TestClient::new(build_app(vec![ALLOWED.into()]));

    let resp = cli
        .options("/healthz")
        .header(header::ORIGIN, ALLOWED)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "PATCH")
        .header(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "authorization,idempotency-key",
        )
        .send()
        .await;
    resp.assert_status_is_ok();

    let headers = resp.0.headers();

    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some(ALLOWED),
    );

    let allow_methods = headers
        .get(header::ACCESS_CONTROL_ALLOW_METHODS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    for m in ["GET", "POST", "PATCH", "DELETE", "OPTIONS"] {
        assert!(
            allow_methods.contains(m),
            "Allow-Methods missing {m}: {allow_methods}",
        );
    }

    let allow_headers = headers
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let allow_headers_lc = allow_headers.to_lowercase();
    for h in [
        "authorization",
        "content-type",
        "idempotency-key",
        "if-match",
        "x-request-id",
    ] {
        assert!(
            allow_headers_lc.contains(h),
            "Allow-Headers missing {h}: {allow_headers}",
        );
    }

    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_MAX_AGE)
            .and_then(|v| v.to_str().ok()),
        Some("600"),
    );
}
