//! `poem-openapi` Bearer security scheme — opaque-token path.
//!
//! Phase 3.3. JWT path lands in 3.26 alongside the JWKS cache.
//!
//! Flow:
//!   1. `poem-openapi` parses `Authorization: Bearer <token>`.
//!   2. `verify_bearer` is called with the bearer token.
//!   3. We `auth::opaque::parse` it. Structural failure → `None`
//!      → `poem-openapi` returns `401`.
//!   4. We open a transaction with `db::begin_auth_lookup` so the
//!      `api_tokens` RLS auth-bootstrap branch unlocks a single
//!      row by primary key. The query also filters out revoked /
//!      expired tokens at the DB layer.
//!   5. We verify argon2id and build a `Principal`. Any failure
//!      short-circuits to `None` → `401`.
//!
//! The transaction is rolled back on drop; the auth-bootstrap
//! GUC is therefore scoped to this single lookup.

use poem::Request;
use poem_openapi::auth::Bearer;
use poem_openapi::SecurityScheme;
use std::str::FromStr;

use crate::auth::{opaque, Principal, Role, Scope, TokenType};
use crate::db;
use crate::state::AppState;

#[derive(SecurityScheme)]
#[oai(ty = "bearer", checker = "verify_bearer")]
pub struct BearerAuth(pub Principal);

async fn verify_bearer(req: &Request, bearer: Bearer) -> Option<Principal> {
    let state = req.data::<AppState>()?;
    let pool = state.db.as_ref()?;

    let parsed = opaque::parse(&bearer.token).ok()?;
    let db_id = parsed.db_id();

    let mut tx = db::begin_auth_lookup(pool, &db_id).await.ok()?;

    // Liveness filtered at the DB layer so the time crate doesn't
    // need to be a sqlx feature.
    let row: Option<(String, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT org_id, project_id, scope, role, secret_hash
         FROM knievel.api_tokens
         WHERE id = $1
           AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(&db_id)
    .fetch_optional(&mut *tx)
    .await
    .ok()?;
    let (org_id, project_id, scope, role, secret_hash) = row?;

    opaque::verify(parsed.secret, &secret_hash).ok()?;

    let scope = match scope.as_str() {
        "org" => Scope::Org,
        "project" => Scope::Project,
        _ => return None,
    };
    let role = Role::from_str(&role).ok()?;

    Some(Principal {
        token_type: TokenType::Opaque,
        scope,
        org_id,
        project_id,
        role,
        actor_id: db_id,
    })
}
