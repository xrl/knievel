//! System endpoints: `/healthz`, `/readyz`, `/version`,
//! `/openapi.json`. Unauthenticated by default; operators put them
//! behind a reverse proxy if access control is needed
//! (`API.md` § 5).
//!
//! Phase 2.4 lands `/healthz`. `/readyz`, `/version`, and the
//! OpenAPI spec endpoint follow in 2.5–2.7.

use poem::{handler, http::StatusCode, IntoResponse, Response};

/// Liveness — returns 200 as long as the process is up. The
/// k8s liveness probe key (`API.md` § 5).
#[handler]
pub async fn healthz() -> Response {
    StatusCode::OK.with_body("ok\n").into_response()
}

#[cfg(test)]
mod tests {
    use poem::test::TestClient;
    use poem::{get, Route};

    use super::*;

    #[tokio::test]
    async fn healthz_returns_200() {
        let app = Route::new().at("/healthz", get(healthz));
        let cli = TestClient::new(app);
        let resp = cli.get("/healthz").send().await;
        resp.assert_status_is_ok();
        resp.assert_text("ok\n").await;
    }

    #[tokio::test]
    async fn healthz_routed_via_server_routes() {
        // Sanity: the routes() helper in src/server.rs wires us in.
        let cli = TestClient::new(crate::server::routes());
        let resp = cli.get("/healthz").send().await;
        resp.assert_status_is_ok();
    }
}
