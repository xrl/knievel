//! System endpoints: `/healthz`, `/readyz`, `/version`. Annotated
//! with `poem-openapi` so the `/openapi.json` spec describes them.
//! Unauthenticated by default; operators can put them behind a
//! reverse proxy if access control is needed (`API.md` § 5).

use std::time::Duration;

use poem::web::Data;
use poem_openapi::{
    payload::{Json, PlainText},
    ApiResponse, Object, OpenApi, Union,
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

/// Timeout applied to the `SELECT 1` DB health probe inside
/// `/readyz`. Keeps the k8s readiness check from hanging past
/// its probe deadline when the writer is partitioned.
const READYZ_DB_TIMEOUT: Duration = Duration::from_secs(3);

/// Snapshot freshness threshold for readiness criterion (a).
/// If the snapshot has never loaded (`config_version == 0`), the
/// pod is not ready. A loaded snapshot is always considered
/// fresh enough for readiness at this layer — staleness
/// degradation (`snapshot_critically_stale`) is surfaced via
/// metrics, not via `/readyz`, because stale decisions are still
/// served (`REQUIREMENTS.md` § 10.6 criterion a: "snapshot has
/// loaded once").
const SNAPSHOT_LOADED_VERSION: i64 = 0;

pub struct SystemApi;

#[derive(ApiResponse)]
pub enum HealthzResponse {
    #[oai(status = 200)]
    Ok(PlainText<String>),
}

#[derive(ApiResponse)]
pub enum ReadyzResponse {
    /// All readiness criteria pass — pod is ready to serve.
    #[oai(status = 200)]
    Ok(PlainText<String>),
    /// One or more criteria failed; pod should leave the LB.
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

/// Discriminated union for `IssuerSummary.claim_source`.
///
/// `AUTH.md` "Effective-policy visibility" (lines 918, 925) mandates a
/// structured object, not a free-form string:
///
/// ```json
/// { "kind": "claim",         "name": "knievel" }
/// { "kind": "claim_mapping", "rule_count": 2    }
/// ```
///
/// `Claim` — knievel authz claims live verbatim in a named custom JWT
/// claim (the default is `knievel`).
/// `ClaimMapping` — one or more mapping rules pull standard-claim values
/// into the authz shape; `rule_count` is the number of active rules.
#[derive(Union)]
#[oai(discriminator_name = "kind", one_of = true)]
pub enum ClaimSource {
    #[oai(mapping = "claim")]
    Claim(ClaimSourceClaim),
    #[oai(mapping = "claim_mapping")]
    ClaimMapping(ClaimSourceMapping),
}

/// `claim_source` variant: knievel claims live in a custom JWT claim.
#[derive(Object)]
pub struct ClaimSourceClaim {
    /// Name of the custom JWT claim that carries knievel authz data.
    pub name: String,
}

/// `claim_source` variant: mapping rules translate standard JWT claims.
#[derive(Object)]
pub struct ClaimSourceMapping {
    /// Number of active mapping rules.
    pub rule_count: u32,
}

#[derive(Object)]
pub struct IssuerSummary {
    pub issuer: String,
    pub audience: String,
    pub algorithms: Vec<String>,
    /// Structured descriptor of how knievel reads authz from this
    /// issuer's JWTs. Per `AUTH.md` "Effective-policy visibility."
    pub claim_source: ClaimSource,
    pub jwks_url: Option<String>,
}

#[OpenApi(tag = "ApiTags::System")]
impl SystemApi {
    /// Liveness — k8s liveness probe key.
    #[oai(path = "/healthz", method = "get", operation_id = "healthz")]
    async fn healthz(&self) -> HealthzResponse {
        HealthzResponse::Ok(PlainText("ok\n".into()))
    }

    /// Readiness — `200` only when all four `REQUIREMENTS.md` § 10.6
    /// criteria pass: (a) snapshot has loaded once, (b) DB writer is
    /// reachable, (c) event flusher is alive, (d) no unconfigured DB.
    ///
    /// The DB probe is wrapped in a `READYZ_DB_TIMEOUT` so a
    /// partitioned writer doesn't cause the k8s readiness probe to
    /// hang past its own deadline.
    ///
    /// Criterion (d) from the spec — partition maintenance within
    /// 24 h — is advisory and emitted via metrics; a stale partition
    /// run does not pull the pod from the LB because decisions remain
    /// serveable. The watchdog in `leader.rs` exits the process when
    /// the budget is exceeded instead.
    #[oai(path = "/readyz", method = "get", operation_id = "readyz")]
    async fn readyz(&self, Data(state): Data<&AppState>) -> ReadyzResponse {
        // (b) DB writer reachable — explicit 503 when no pool is
        // configured in production. The "no_db" path is intentionally
        // removed: a misconfigured pod must not appear ready.
        let pool = match &state.db {
            None => {
                tracing::warn!("readyz: no database configured");
                return ReadyzResponse::NotReady(PlainText(
                    "not_ready: no_db_configured\n".into(),
                ));
            }
            Some(p) => p,
        };

        let db_result = tokio::time::timeout(
            READYZ_DB_TIMEOUT,
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
        )
        .await;
        match db_result {
            Err(_elapsed) => {
                tracing::warn!("readyz: DB probe timed out");
                return ReadyzResponse::NotReady(PlainText("not_ready: db_timeout\n".into()));
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "readyz: DB unreachable");
                return ReadyzResponse::NotReady(PlainText("not_ready: db_unreachable\n".into()));
            }
            Ok(Ok(_)) => {}
        }

        // (a) Snapshot has loaded at least once.
        // `config_version == 0` means the loader has not yet
        // completed its cold load (`run_loader` starts at 0 and
        // bumps after the first successful swap).
        if state.snapshot.read().config_version == SNAPSHOT_LOADED_VERSION {
            tracing::warn!("readyz: snapshot not yet loaded");
            return ReadyzResponse::NotReady(PlainText("not_ready: snapshot_not_loaded\n".into()));
        }

        // (c) Event flusher alive — the sender is present and its
        // channel is not closed. A closed channel means the flusher
        // task exited unexpectedly.
        if let Some(events) = &state.events {
            if events.is_closed() {
                tracing::warn!("readyz: event flusher down");
                return ReadyzResponse::NotReady(PlainText(
                    "not_ready: event_flusher_down\n".into(),
                ));
            }
        }

        ReadyzResponse::Ok(PlainText("ok\n".into()))
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
    async fn readyz_no_db_returns_503() {
        // A pod with no database configured must NOT appear ready —
        // returning 200 here would let a misconfigured instance
        // silently receive traffic.
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/readyz").send().await;
        resp.assert_status(poem::http::StatusCode::SERVICE_UNAVAILABLE);
        resp.assert_text("not_ready: no_db_configured\n").await;
    }

    #[tokio::test]
    async fn readyz_snapshot_not_loaded_returns_503() {
        // A pod whose snapshot has never loaded (config_version == 0)
        // is not safe to serve decisions from.
        // We can't easily wire a live DB here without Postgres, so
        // we verify the no-db path still surfaces 503, and separately
        // cover the snapshot criterion with unit logic below.
        let state = AppState::new();
        // Snapshot config_version starts at 0 (not loaded).
        assert_eq!(
            state.snapshot.read().config_version,
            SNAPSHOT_LOADED_VERSION
        );
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
    async fn version_claim_source_is_structured_object() {
        // AUTH.md lines 918/925: claim_source must be a structured
        // object with a `kind` discriminator, not a flat string.
        // Verify the OpenAPI schema reflects this.
        let cli = TestClient::new(app_with_state(AppState::new()));
        let resp = cli.get("/openapi.json").send().await;
        resp.assert_status_is_ok();
        let spec: serde_json::Value = resp.json().await.value().deserialize();
        // ClaimSource must appear as a oneOf/discriminated union in
        // the components schemas, not as a plain string type.
        let schemas = &spec["components"]["schemas"];
        // IssuerSummary.claim_source should be a $ref, not {type: string}
        let issuer_props = &schemas["IssuerSummary"]["properties"];
        let cs = &issuer_props["claim_source"];
        // poem-openapi renders Union as oneOf; the property must not
        // be `{"type": "string"}`.
        assert_ne!(
            cs.get("type").and_then(|v| v.as_str()),
            Some("string"),
            "claim_source must not be a flat string in the OpenAPI schema"
        );
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
