//! `poem-openapi` Bearer security scheme — opaque + JWT paths.
//!
//! Phase 3.3 wired the opaque path; Phase 3.26 follow-up wired the
//! JWT path on top.
//!
//! Flow:
//!   1. `poem-openapi` parses `Authorization: Bearer <token>`.
//!   2. `verify_bearer` is called with the bearer token.
//!   3. Token shape is sniffed: three dot-separated segments whose
//!      first base64url-segment decodes to a JSON object with an
//!      `alg` field is treated as a JWT and dispatched to
//!      `JwtVerifier::verify`. Anything else falls through to the
//!      opaque-token path (`auth::opaque::parse`).
//!   4. Opaque path: open a transaction with `db::begin_auth_lookup`
//!      so the `api_tokens` RLS auth-bootstrap branch unlocks a
//!      single row by primary key, verify argon2id, build a
//!      `Principal`. Any failure short-circuits to `None` → `401`.
//!   5. JWT path: `JwtVerifier` runs JWKS fetch + signature
//!      verification + claim extraction. Returns `None` on any
//!      failure (caller sees 401).
//!
//! The opaque-path transaction is rolled back on drop; the
//! auth-bootstrap GUC is therefore scoped to this single lookup.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
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

    // Sniff JWT first — opaque tokens never contain `.`, so this is
    // both unambiguous and cheap.
    if state.jwt_verifier.is_enabled() && looks_like_jwt(&bearer.token) {
        match state.jwt_verifier.verify(&bearer.token).await {
            Ok(principal) => return Some(principal),
            Err(err) => {
                tracing::debug!(?err, "JWT bearer verification failed");
                return None;
            }
        }
    }

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

/// Cheap JWT-shape detection. A real JWT has exactly two `.`
/// separators and a base64url-encoded JSON object with an `alg`
/// field as its first segment. Opaque tokens (`kvl_…`) carry no
/// `.`, so the first check is enough to reject them.
fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return false;
    }
    let Ok(header_bytes) = URL_SAFE_NO_PAD.decode(parts[0]) else {
        return false;
    };
    let Ok(header) = serde_json::from_slice::<serde_json::Value>(&header_bytes) else {
        return false;
    };
    header.get("alg").and_then(|v| v.as_str()).is_some()
}
