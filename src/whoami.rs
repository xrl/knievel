//! `GET /v1/whoami` — minimal "who am I" handshake.
//!
//! The smallest possible authenticated endpoint: validates the
//! bearer (opaque or JWT, identical code path), echoes the
//! Principal back. Phase 7.4 adds it so the admin SPA can
//! confirm `Authorization: Bearer <…>` is valid before
//! rendering the workspace, and so the role-claim-driven UI
//! gating (Phase 7.9) has a server-vouched source of truth
//! for `principal.role` rather than parsing the JWT
//! client-side.
//!
//! Returns nothing sensitive — scope, role, org_id (and
//! project_id when scope=project), token_type, actor_id. The
//! actor_id is `tok_<short>` for opaque tokens and
//! `(iss, sub, azp)` (joined) for JWTs; it's already in
//! `audit_log.actor`, so exposing it here doesn't leak
//! anything that isn't also in operator-side logs.
//!
//! Principal exposure: `org_id` is always returned because the
//! bearer itself is bound to an org; the caller already knows
//! it (they minted the token against that org). `project_id`
//! is returned only when `scope == "project"` — org-scoped
//! callers receive `null`. This matches the token's own scope
//! and adds nothing the caller doesn't already hold.

use poem_openapi::{payload::Json, ApiResponse, Object, OpenApi};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::{Scope, TokenType};

pub struct WhoamiApi;

#[derive(Object)]
pub struct WhoamiResponse {
    /// `org` or `project`.
    pub scope: String,
    pub org_id: String,
    /// Present only when `scope == "project"`.
    pub project_id: Option<String>,
    /// One of `org-owner`, `org-admin`, `admin`, `editor`, `reader`.
    pub role: String,
    /// `opaque` for `kvl_*` bearers; `jwt` for OIDC bearers.
    pub token_type: String,
    /// Opaque actor identifier; matches `audit_log.actor`.
    pub actor_id: String,
}

#[derive(ApiResponse)]
pub enum WhoamiResp {
    /// Bearer is valid — returns the resolved principal.
    #[oai(status = 200)]
    Ok(Json<WhoamiResponse>),
    /// Bearer is missing, malformed, revoked, or expired.
    ///
    /// `poem-openapi` returns this automatically when the
    /// `BearerAuth` extractor yields `None` — the variant is
    /// declared here so the `401` status appears in the OpenAPI
    /// spec rather than being silently omitted.
    #[oai(status = 401)]
    Unauthorized,
}

#[OpenApi(tag = "ApiTags::Auth")]
impl WhoamiApi {
    /// Validate the bearer token and return the resolved principal.
    ///
    /// Use this as a pre-flight before rendering the admin
    /// workspace: a `200` guarantees the token is valid and
    /// surfaces the server-resolved role so the UI can gate
    /// features without parsing the JWT client-side.
    #[oai(
        path = "/v1/whoami",
        method = "get",
        operation_id = "whoami",
        summary = "Validate bearer and return resolved principal"
    )]
    async fn whoami(&self, auth: BearerAuth) -> WhoamiResp {
        let p = auth.0;
        WhoamiResp::Ok(Json(WhoamiResponse {
            scope: match p.scope {
                Scope::Org => "org".into(),
                Scope::Project => "project".into(),
            },
            org_id: p.org_id,
            project_id: p.project_id,
            role: p.role.as_str().into(),
            token_type: match p.token_type {
                TokenType::Opaque => "opaque".into(),
                TokenType::Jwt => "jwt".into(),
            },
            actor_id: p.actor_id,
        }))
    }
}

#[cfg(test)]
mod tests {
    use poem::http::StatusCode;
    use poem::test::TestClient;
    use poem::EndpointExt;

    use crate::server::routes;
    use crate::state::AppState;

    fn app() -> impl poem::Endpoint {
        routes().data(AppState::new())
    }

    #[tokio::test]
    async fn whoami_unauthenticated_returns_401() {
        // No Authorization header — `poem-openapi`'s BearerAuth
        // extractor must return 401, not panic or 500.
        let cli = TestClient::new(app());
        let resp = cli.get("/v1/whoami").send().await;
        resp.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn whoami_bad_token_returns_401() {
        // A structurally invalid bearer (not a kvl_ token, no DB
        // configured) must also return 401 cleanly.
        let cli = TestClient::new(app());
        let resp = cli
            .get("/v1/whoami")
            .header("Authorization", "Bearer not-a-valid-token")
            .send()
            .await;
        resp.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn whoami_appears_in_openapi_spec() {
        let cli = TestClient::new(app());
        let resp = cli.get("/openapi.json").send().await;
        resp.assert_status_is_ok();
        let spec: serde_json::Value = resp.json().await.value().deserialize();
        let paths = spec["paths"].as_object().expect("paths is an object");
        assert!(
            paths.contains_key("/v1/whoami"),
            "whoami path missing from spec"
        );
        // 401 must be declared in the spec so clients know to expect it.
        let responses = &spec["paths"]["/v1/whoami"]["get"]["responses"];
        assert!(
            responses.get("401").is_some(),
            "whoami spec must declare 401 response; got: {responses}"
        );
    }
}
