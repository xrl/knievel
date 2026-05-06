//! System endpoints: `/healthz`, `/readyz`, `/version`. Annotated
//! with `poem-openapi` so the `/openapi.json` spec describes them.
//! Unauthenticated by default; operators can put them behind a
//! reverse proxy if access control is needed (`API.md` § 5).

use poem::web::Data;
use poem_openapi::{
    payload::{Json, PlainText},
    ApiResponse, Object, OpenApi,
};

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

pub struct SystemApi;

#[derive(ApiResponse)]
pub enum HealthzResponse {
    #[oai(status = 200)]
    Ok(PlainText<String>),
}

#[derive(ApiResponse)]
pub enum ReadyzResponse {
    /// Process up + DB reachable (or no DB configured).
    #[oai(status = 200)]
    Ok(PlainText<String>),
    /// DB writer unreachable; pods should be removed from the LB.
    #[oai(status = 503)]
    NotReady(PlainText<String>),
}

/// Build metadata + effective auth policy. Per `API.md` § 5 and
/// `AUTH.md` "Effective-policy visibility." `auth.modes` and
/// `auth.issuers` are empty until Phase 3.16 lands real auth.
#[derive(Object)]
pub struct VersionResponse {
    pub knievel: String,
    pub schema: String,
    pub git_sha: String,
    pub build_timestamp: String,
    pub auth: AuthBlock,
}

#[derive(Object, Default)]
pub struct AuthBlock {
    pub modes: Vec<String>,
    pub issuers: Vec<IssuerSummary>,
}

#[derive(Object)]
pub struct IssuerSummary {
    pub issuer: String,
    pub audience: String,
}

#[OpenApi]
impl SystemApi {
    /// Liveness — k8s liveness probe key.
    #[oai(path = "/healthz", method = "get", operation_id = "healthz")]
    async fn healthz(&self) -> HealthzResponse {
        HealthzResponse::Ok(PlainText("ok\n".into()))
    }

    /// Readiness — only 200 when knievel can serve. Per
    /// `REQUIREMENTS.md` § 10.6, the full check has four
    /// criteria; today only the DB-reachability one is real.
    #[oai(path = "/readyz", method = "get", operation_id = "readyz")]
    async fn readyz(&self, Data(state): Data<&AppState>) -> ReadyzResponse {
        match &state.db {
            None => ReadyzResponse::Ok(PlainText("ok: no_db_configured\n".into())),
            Some(pool) => match sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(pool)
                .await
            {
                Ok(_) => ReadyzResponse::Ok(PlainText("ok\n".into())),
                Err(e) => {
                    tracing::warn!(error = %e, "readyz: DB unreachable");
                    ReadyzResponse::NotReady(PlainText("not_ready: db_unreachable\n".into()))
                }
            },
        }
    }

    /// Build metadata + effective auth policy.
    #[oai(path = "/version", method = "get", operation_id = "version")]
    async fn version(&self) -> Json<VersionResponse> {
        Json(VersionResponse {
            knievel: PKG_VERSION.into(),
            schema: SCHEMA_VERSION.into(),
            git_sha: GIT_SHA.into(),
            build_timestamp: BUILD_TIMESTAMP.into(),
            auth: AuthBlock::default(),
        })
    }
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

    #[tokio::test]
    async fn version_returns_json_with_required_fields() {
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/version").send().await;
        resp.assert_status_is_ok();
        let body: serde_json::Value = resp.json().await.value().deserialize();
        assert_eq!(body["knievel"], serde_json::json!(PKG_VERSION));
        assert_eq!(body["schema"], serde_json::json!(SCHEMA_VERSION));
        assert!(body.get("git_sha").is_some());
        assert!(body.get("build_timestamp").is_some());
        let modes = body["auth"]["modes"]
            .as_array()
            .expect("auth.modes is an array");
        assert!(modes.is_empty());
    }

    #[tokio::test]
    async fn openapi_json_describes_system_endpoints() {
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/openapi.json").send().await;
        resp.assert_status_is_ok();
        let spec: serde_json::Value = resp.json().await.value().deserialize();
        let paths = spec["paths"].as_object().expect("paths is an object");
        assert!(paths.contains_key("/healthz"), "{spec}");
        assert!(paths.contains_key("/readyz"));
        assert!(paths.contains_key("/version"));
    }
}
