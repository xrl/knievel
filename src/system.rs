//! System endpoints: `/healthz`, `/readyz`, `/version`,
//! `/openapi.json`. Unauthenticated by default; operators put them
//! behind a reverse proxy if access control is needed
//! (`API.md` § 5).
//!
//! Phase 2.4 landed `/healthz`; Phase 2.5 lands `/readyz`.
//! `/version` and the OpenAPI spec endpoint follow in 2.6–2.7.

use poem::http::header;
use poem::web::Data;
use poem::{handler, http::StatusCode, IntoResponse, Response};
use serde_json::json;

use crate::state::AppState;

/// OpenAPI schema version. Lives separately from the package
/// version because the spec compatibility model is additive
/// (`REQUIREMENTS.md` § 6.4) and may evolve at a different cadence
/// than the binary itself. v0 sits at `0.0` until a tagged release
/// pins it.
pub const SCHEMA_VERSION: &str = "0.0";

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHA: &str = env!("KNIEVEL_GIT_SHA");
const BUILD_TIMESTAMP: &str = env!("KNIEVEL_BUILD_TIMESTAMP");

/// Liveness — returns 200 as long as the process is up. The
/// k8s liveness probe key (`API.md` § 5).
#[handler]
pub async fn healthz() -> Response {
    StatusCode::OK.with_body("ok\n").into_response()
}

/// Readiness — returns 200 only when knievel can serve. Per
/// `REQUIREMENTS.md` § 10.6 the full check is:
///
///   (a) snapshot has loaded once,
///   (b) DB writer is reachable,
///   (c) event flusher hasn't deadlocked,
///   (d) some pod reports a successful partition maintenance run
///       within the last 24 h.
///
/// Today only (b) is checked; (a), (c), and (d) land alongside
/// their subsystems in Phase 3+.
#[handler]
pub async fn readyz(Data(state): Data<&AppState>) -> Response {
    match &state.db {
        None => StatusCode::OK
            .with_body("ok: no_db_configured\n")
            .into_response(),
        Some(pool) => match sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(pool)
            .await
        {
            Ok(_) => StatusCode::OK.with_body("ok\n").into_response(),
            Err(e) => {
                tracing::warn!(error = %e, "readyz: DB unreachable");
                StatusCode::SERVICE_UNAVAILABLE
                    .with_body("not_ready: db_unreachable\n")
                    .into_response()
            }
        },
    }
}

/// Build metadata + effective auth policy. Per `API.md` § 5 and
/// `AUTH.md` "Effective-policy visibility." Today the auth block
/// is empty — Phase 3.16 lands real modes/issuers.
#[handler]
pub async fn version() -> Response {
    let body = json!({
        "knievel":         PKG_VERSION,
        "schema":          SCHEMA_VERSION,
        "git_sha":         GIT_SHA,
        "build_timestamp": BUILD_TIMESTAMP,
        "auth": {
            "modes":   [],
            "issuers": []
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
}

#[cfg(test)]
mod tests {
    use poem::test::TestClient;
    use poem::EndpointExt;

    use super::*;
    use crate::server::routes;

    fn app_with_state(state: AppState) -> impl poem::Endpoint {
        routes().data(state)
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/healthz").send().await;
        resp.assert_status_is_ok();
        resp.assert_text("ok\n").await;
    }

    #[tokio::test]
    async fn readyz_no_db_returns_200_with_reason() {
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/readyz").send().await;
        resp.assert_status_is_ok();
        resp.assert_text("ok: no_db_configured\n").await;
    }

    // The DB-reachable + DB-unreachable paths land in the
    // db-integ CI job once Phase 1.9's testlib is being exercised
    // by an HTTP-level test (Phase 3 brings the test client
    // together with state holding a real PgPool).

    #[tokio::test]
    async fn version_returns_json_with_required_fields() {
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/version").send().await;
        resp.assert_status_is_ok();
        resp.assert_header("content-type", "application/json");
        let body: serde_json::Value = resp.json().await.value().deserialize();
        assert!(body.get("knievel").is_some());
        assert!(body.get("schema").is_some());
        assert!(body.get("git_sha").is_some());
        assert!(body.get("build_timestamp").is_some());
        assert!(body.get("auth").is_some());
        let modes = body
            .get("auth")
            .and_then(|a| a.get("modes"))
            .and_then(|m| m.as_array())
            .expect("auth.modes is an array");
        // Phase 2 has no auth wired yet.
        assert!(modes.is_empty());
    }
}
