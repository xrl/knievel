//! Org-level Tokens API — mint, list, revoke.
//!
//! Phase 3.6. Spec refs:
//!   - `API.md` § 2.2 (Tokens)
//!   - `AUTH.md` "Opaque Tokens", "Endpoint -> minimum role"
//!   - `REQUIREMENTS.md` § 7.3 (audit_log writers)
//!
//! `POST` returns the plaintext secret exactly once; subsequent
//! reads expose metadata only. Revoke is a soft delete via
//! `revoked_at`; the auth path filters revoked rows at the DB
//! layer so a revocation takes effect on the next request.
//!
//! Token mint and revoke each emit one `audit_log` row inside the
//! same transaction as the data mutation, so a crash between the
//! two can't leave the audit trail behind.

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::api_tags::ApiTags;
use crate::audit;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::db;
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct TokensApi;

/// Hardcoded for v0; future commits parameterize via config so
/// `kvl_<env>_...` matches the deployment environment.
const TOKEN_ENV: &str = "prod";

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateTokenRequest {
    pub name: String,
    /// "org" or "project".
    pub scope: String,
    /// Required iff `scope == "project"`.
    pub project_id: Option<String>,
    /// `reader`, `editor`, `admin`, `org-admin`, `org-owner`.
    pub role: String,
    pub expires_at: Option<String>,
    pub ip_allowlist: Option<Vec<String>>,
}

#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateTokenResponse {
    pub id: String,
    /// Plaintext secret. **Returned exactly once** — knievel stores
    /// only the argon2id hash. Lost secrets cannot be recovered.
    pub secret: String,
    pub name: String,
    pub scope: String,
    pub role: String,
    pub created_at: String,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct TokenSummary {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub role: String,
    pub project_id: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct TokenListResponse {
    pub items: Vec<TokenSummary>,
    /// Cursor pagination lands when first paginated endpoint
    /// genuinely needs it; until then this is always null.
    pub next_cursor: Option<String>,
}

#[derive(ApiResponse)]
pub enum CreateTokenResp {
    #[oai(status = 201)]
    Created(Json<CreateTokenResponse>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum ListTokensResp {
    #[oai(status = 200)]
    Ok(Json<TokenListResponse>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum RevokeTokenResp {
    #[oai(status = 204)]
    NoContent,
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

fn err(code: &str, message: &str) -> ErrorEnvelope {
    ErrorEnvelope {
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
        },
    }
}

#[OpenApi(tag = "ApiTags::Tokens")]
impl TokensApi {
    /// Mint an opaque token. Returns the plaintext secret exactly
    /// once. Min role: org-admin.
    #[oai(
        path = "/v1/orgs/:org_id/tokens",
        method = "post",
        operation_id = "createToken"
    )]
    async fn create_token(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        body: Json<CreateTokenRequest>,
    ) -> CreateTokenResp {
        let principal = auth.0;
        let path_org_id = org_id.0;
        let req = body.0;

        if principal.org_id != path_org_id {
            return CreateTokenResp::Forbidden(Json(err(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::OrgAdmin) {
            return CreateTokenResp::Forbidden(Json(err(
                "role_insufficient",
                "minting tokens requires org-admin or higher",
            )));
        }

        // Validate scope/role/project_id consistency at the handler
        // layer so we get a clean 400 before touching the DB. Same
        // shape as the api_tokens.scope CHECK constraint.
        if req.scope != "org" && req.scope != "project" {
            return CreateTokenResp::BadRequest(Json(err(
                "invalid_scope",
                "scope must be 'org' or 'project'",
            )));
        }
        if !matches!(
            req.role.as_str(),
            "reader" | "editor" | "admin" | "org-admin" | "org-owner"
        ) {
            return CreateTokenResp::BadRequest(Json(err(
                "invalid_role",
                "role must be one of reader, editor, admin, org-admin, org-owner",
            )));
        }
        if req.scope == "project" && req.project_id.is_none() {
            return CreateTokenResp::BadRequest(Json(err(
                "project_id_required",
                "scope=project requires project_id",
            )));
        }
        if req.scope == "org" && req.project_id.is_some() {
            return CreateTokenResp::BadRequest(Json(err(
                "project_id_forbidden",
                "scope=org must not include project_id",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return CreateTokenResp::Internal(Json(err("no_db", "no database configured"))),
        };

        // Generate the opaque-token segments. id_short is what the
        // wire format embeds; tok_<id_short> is the row id.
        let id_short = random_hex(6);
        let secret = random_hex(16);
        let row_id = format!("tok_{id_short}");
        let plaintext = format!(
            "kvl_{TOKEN_ENV}_{scope}_{id_short}_{secret}",
            scope = req.scope,
        );

        let secret_hash = match crate::auth::opaque::hash(&secret) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(error = %e, "argon2 hash failed");
                return CreateTokenResp::Internal(Json(err(
                    "hash_failed",
                    "could not hash token secret",
                )));
            }
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return CreateTokenResp::Internal(Json(err(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        let insert_sql = "INSERT INTO knievel.api_tokens \
            (id, org_id, project_id, scope, role, name, secret_hash, ip_allowlist) \
            VALUES ($1, $2, $3, $4, $5, $6, $7, COALESCE($8, '{}'::text[])) \
            RETURNING id, name, scope, role, \
                      to_char(created_at AT TIME ZONE 'UTC', \
                              'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at";
        let inserted: Result<(String, String, String, String, String), _> =
            sqlx::query_as(insert_sql)
                .bind(&row_id)
                .bind(&path_org_id)
                .bind(req.project_id.as_deref())
                .bind(&req.scope)
                .bind(&req.role)
                .bind(&req.name)
                .bind(&secret_hash)
                .bind(req.ip_allowlist.as_deref())
                .fetch_one(&mut *tx)
                .await;
        let (id, name, scope, role, created_at) = match inserted {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "api_tokens insert failed");
                return CreateTokenResp::Internal(Json(err("db_error", "token insert failed")));
            }
        };

        // Audit row in the same transaction so mint and audit are
        // atomic. payload_hash captures a sanitized digest of the
        // request — no secret material lands in audit_log.
        if let Err(e) =
            audit::emit(&mut tx, &principal, "tokens.mint", "token", &id, Some(&req)).await
        {
            tracing::error!(error = %e, "audit_log insert failed");
            return CreateTokenResp::Internal(Json(err("db_error", "audit_log insert failed")));
        }

        if let Err(e) = tx.commit().await {
            tracing::error!(error = %e, "commit failed");
            return CreateTokenResp::Internal(Json(err(
                "db_error",
                "could not commit transaction",
            )));
        }

        CreateTokenResp::Created(Json(CreateTokenResponse {
            id,
            secret: plaintext,
            name,
            scope,
            role,
            created_at,
        }))
    }

    /// List tokens (metadata only; secrets never leave the mint
    /// response). Min role: org-admin.
    #[oai(
        path = "/v1/orgs/:org_id/tokens",
        method = "get",
        operation_id = "listTokens"
    )]
    async fn list_tokens(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
    ) -> ListTokensResp {
        let principal = auth.0;
        let path_org_id = org_id.0;

        if principal.org_id != path_org_id {
            return ListTokensResp::Forbidden(Json(err(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::OrgAdmin) {
            return ListTokensResp::Forbidden(Json(err(
                "role_insufficient",
                "listing tokens requires org-admin or higher",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ListTokensResp::Internal(Json(err("no_db", "no database configured"))),
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return ListTokensResp::Internal(Json(err(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        // Pagination lands in 3.14+; for v0 always return all rows
        // up to a safety cap.
        let rows: Result<Vec<TokenSummary>, _> = sqlx::query_as::<_, TokenSummary>(
            "SELECT id, name, scope, role, project_id, \
                to_char(created_at AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at, \
                to_char(last_used_at AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS last_used_at, \
                to_char(revoked_at AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS revoked_at, \
                to_char(expires_at AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS expires_at \
             FROM knievel.api_tokens \
             ORDER BY created_at DESC \
             LIMIT 500",
        )
        .fetch_all(&mut *tx)
        .await;

        match rows {
            Ok(items) => ListTokensResp::Ok(Json(TokenListResponse {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list_tokens failed");
                ListTokensResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    /// Revoke a token (soft delete via `revoked_at`). The auth
    /// path filters revoked rows at the next request. Min role:
    /// org-admin.
    #[oai(
        path = "/v1/orgs/:org_id/tokens/:token_id",
        method = "delete",
        operation_id = "revokeToken"
    )]
    async fn revoke_token(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        token_id: Path<String>,
    ) -> RevokeTokenResp {
        let principal = auth.0;
        let path_org_id = org_id.0;
        let path_token_id = token_id.0;

        if principal.org_id != path_org_id {
            return RevokeTokenResp::Forbidden(Json(err(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::OrgAdmin) {
            return RevokeTokenResp::Forbidden(Json(err(
                "role_insufficient",
                "revoking tokens requires org-admin or higher",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return RevokeTokenResp::Internal(Json(err("no_db", "no database configured"))),
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return RevokeTokenResp::Internal(Json(err(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        let updated = match sqlx::query(
            "UPDATE knievel.api_tokens
             SET revoked_at = now()
             WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(&path_token_id)
        .execute(&mut *tx)
        .await
        {
            Ok(r) => r.rows_affected(),
            Err(e) => {
                tracing::error!(error = %e, "revoke update failed");
                return RevokeTokenResp::Internal(Json(err("db_error", "revoke failed")));
            }
        };

        if updated == 0 {
            // Either the row doesn't exist or it's already revoked.
            // Both cases surface as 404 — caller can rely on
            // "no live token by this id" semantics either way.
            return RevokeTokenResp::NotFound(Json(err(
                "not_found",
                "token not found or already revoked",
            )));
        }

        // Symmetric with mint: revoke routes through `audit::emit`
        // so `payload_hash` is a SHA-256 digest, not the raw token
        // id. Closes sonnet #14 / opus O14.
        if let Err(e) = audit::emit(
            &mut tx,
            &principal,
            "tokens.revoke",
            "token",
            &path_token_id,
            // No request body to hash; the audit row is already
            // pinned by (org, actor, operation, resource_id).
            None::<&serde_json::Value>,
        )
        .await
        {
            tracing::error!(error = %e, "audit_log insert failed");
            return RevokeTokenResp::Internal(Json(err("db_error", "audit_log insert failed")));
        }

        if let Err(e) = tx.commit().await {
            tracing::error!(error = %e, "commit failed");
            return RevokeTokenResp::Internal(Json(err(
                "db_error",
                "could not commit transaction",
            )));
        }

        RevokeTokenResp::NoContent
    }
}

fn random_hex(bytes: usize) -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut buf = vec![0u8; bytes];
    let mut rng = OsRng;
    rng.fill_bytes(&mut buf);
    hex::encode(buf)
}
