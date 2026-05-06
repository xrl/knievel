//! System endpoints: `/healthz`, `/readyz`, `/version`. Annotated
//! with `poem-openapi` so the `/openapi.json` spec describes them.
//! Unauthenticated by default; operators can put them behind a
//! reverse proxy if access control is needed (`API.md` § 5).

use poem::web::Data;
use poem_openapi::{
    payload::{Json, PlainText},
    ApiResponse, Object, OpenApi,
};

use crate::api_tags::ApiTags;
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
/// `AUTH.md` "Effective-policy visibility." `auth.modes` lists
/// the enabled credential types (`opaque`, `jwt`); `auth.issuers`
/// summarizes each configured JWT issuer with its audience,
/// algorithms, claim source, and JWKS URL. Secrets are never
/// returned.
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
    pub algorithms: Vec<String>,
    /// `claim` (default `knievel`) when claims live verbatim in
    /// a custom claim; `claim_mapping(<n>)` when one or more
    /// mapping rules pull standard-claim values into the authz
    /// shape.
    pub claim_source: String,
    pub jwks_url: Option<String>,
}

#[OpenApi(tag = "ApiTags::System")]
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
    async fn version(&self, Data(state): Data<&AppState>) -> Json<VersionResponse> {
        Json(VersionResponse {
            knievel: PKG_VERSION.into(),
            schema: SCHEMA_VERSION.into(),
            git_sha: GIT_SHA.into(),
            build_timestamp: BUILD_TIMESTAMP.into(),
            auth: build_auth_block(state),
        })
    }
}

/// Materialize the `/version` auth block from `AppState`. Phase
/// 3.27 v0: opaque tokens are always available (the `api_tokens`
/// table is in every deployment); JWT mode is enabled when the
/// config carries one or more issuer policies. Empty
/// `auth.issuers` here means "no JWT issuers configured" —
/// pure-opaque deployments serve a legitimate empty array.
fn build_auth_block(_state: &AppState) -> AuthBlock {
    let mut block = AuthBlock {
        modes: vec!["opaque".into()],
        issuers: vec![],
    };
    // JWT mode + per-issuer policies are wired in once `Config`
    // grows the `auth.jwt.issuers` block (3.27 follow-up). For
    // now the binary advertises only the always-on opaque mode.
    let _ = &mut block.issuers;
    block
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
        // Phase 3.27: opaque mode is always advertised; JWT
        // mode is conditional and absent in the no-issuer case.
        let modes = body["auth"]["modes"]
            .as_array()
            .expect("auth.modes is an array");
        let mode_strs: Vec<String> = modes
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        assert!(mode_strs.contains(&"opaque".to_string()));
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
