//! Admin-UI runtime config endpoint.
//!
//! `GET /admin/config.json` — public, unauthenticated, served by
//! the API on the same origin as the SPA. The response carries
//! the OIDC issuer + public client ID + scopes the SPA needs to
//! initialize `oidc-client-ts`, plus the `require_oidc` toggle
//! that hides/shows the paste-a-token fallback. **No secrets** —
//! issuer URL and public client ID are public-by-design;
//! refresh-token / client-secret material never appears here
//! (PKCE replaces the client secret; cf. `AUTH.md` "Keycloak
//! Setup — Human Admin UI (PKCE)").
//!
//! The handler is registered ahead of the Phase 7.11
//! `StaticFilesEndpoint` mount in `routes()` so a `config.json`
//! file inside the bundle can't shadow it.
//!
//! Not in the OpenAPI spec: this is an admin-UI implementation
//! detail, not a public contract surface. Operators discover it
//! by convention (documented in `UI.md` "Auth / Runtime config").

use poem::handler;
use poem::web::{Data, Json};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUiConfigResponse {
    pub oidc: AdminUiOidcResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUiOidcResponse {
    /// OIDC issuer URL. Empty string when OIDC is disabled —
    /// the SPA falls through to the paste-a-token form.
    pub issuer: String,
    /// Public OIDC client ID. Empty when OIDC is disabled.
    pub client_id: String,
    pub scopes: Vec<String>,
    /// When true, the SPA hides the paste-a-token fallback.
    pub require_oidc: bool,
}

#[handler]
pub async fn config_json(state: Data<&AppState>) -> Json<AdminUiConfigResponse> {
    let cfg = &state.0.admin_ui;
    Json(AdminUiConfigResponse {
        oidc: AdminUiOidcResponse {
            issuer: cfg.oidc.issuer.clone().unwrap_or_default(),
            client_id: cfg.oidc.client_id.clone().unwrap_or_default(),
            scopes: cfg.oidc.scopes.clone(),
            require_oidc: cfg.oidc.require_oidc,
        },
    })
}
