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
    #[oai(status = 200)]
    Ok(Json<WhoamiResponse>),
}

#[OpenApi(tag = "ApiTags::Auth")]
impl WhoamiApi {
    #[oai(path = "/v1/whoami", method = "get", operation_id = "whoami")]
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
