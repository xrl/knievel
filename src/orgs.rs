//! Org-scoped handlers — `/v1/orgs/{orgId}/...`.
//!
//! Phase 3.3 lands the first two:
//!   - `POST /v1/orgs/{orgId}/projects` — create a project.
//!   - `GET  /v1/orgs/{orgId}/projects/{projectId}` — read.
//!
//! Both run inside a tenant-bound transaction (`knievel.org_id`
//! GUC set) so the projects-table RLS policy enforces isolation
//! at the DB layer in addition to the principal check at the
//! handler.
//!
//! Spec refs: `API.md` § 2.1, `AUTH.md` "Authorization,"
//! `REQUIREMENTS.md` § 4, § 7.1.1.

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::db;
use crate::state::AppState;

pub struct OrgApi;

/// Row shape returned by the projects-table queries below. Mirrors
/// `ProjectResponse` minus the wire formatting (snake-case here,
/// passed through poem-openapi's `#[derive(Object)]` rename rules
/// on the response).
#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    external_id: Option<String>,
    name: String,
    is_active: bool,
    etag: String,
    created_at: String,
    updated_at: String,
}

const PROJECT_SELECT_COLS: &str = r#"
    id, external_id, name, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

impl From<ProjectRow> for ProjectResponse {
    fn from(r: ProjectRow) -> Self {
        Self {
            id: r.id,
            external_id: r.external_id,
            name: r.name,
            is_active: r.is_active,
            etag: r.etag,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Object)]
pub struct CreateProjectRequest {
    /// Caller-assigned external id, unique within the org.
    pub external_id: Option<String>,
    pub name: String,
}

#[derive(Object, Clone)]
pub struct ProjectResponse {
    pub id: String,
    pub external_id: Option<String>,
    pub name: String,
    pub is_active: bool,
    pub etag: String,
    /// RFC 3339 UTC, formatted by Postgres via `to_jsonb`.
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Object)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

impl ErrorEnvelope {
    fn of(code: &str, message: &str) -> Self {
        Self {
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}

#[derive(ApiResponse)]
pub enum CreateProjectResponse {
    #[oai(status = 201)]
    Created(Json<ProjectResponse>),
    /// Org mismatch between the principal and the path.
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    /// `externalId` already taken in this org.
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum GetProjectResponse {
    #[oai(status = 200)]
    Ok(Json<ProjectResponse>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[OpenApi]
impl OrgApi {
    /// Create a project under an org. Idempotency-key replay and
    /// `externalId`-based no-op upsert land in Phase 3.5; v0 of
    /// this endpoint is a straight insert that returns `409` on
    /// `external_id` collision.
    #[oai(
        path = "/v1/orgs/:org_id/projects",
        method = "post",
        operation_id = "createProject"
    )]
    async fn create_project(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        body: Json<CreateProjectRequest>,
    ) -> CreateProjectResponse {
        let principal = auth.0;
        let path_org_id = org_id.0;

        // Authz: org match + role ≥ org-admin.
        // Per AUTH.md, project creation is org-admin (or owner);
        // the role check uses the Ord on Role.
        if principal.org_id != path_org_id {
            return CreateProjectResponse::Forbidden(Json(ErrorEnvelope::of(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::OrgAdmin) {
            return CreateProjectResponse::Forbidden(Json(ErrorEnvelope::of(
                "role_insufficient",
                "creating a project requires org-admin or higher",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "no_db",
                    "no database configured",
                )))
            }
        };

        let new_id = match random_pj_id() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = %e, "id generation failed");
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "id_generation_failed",
                    "could not generate a project id",
                )));
            }
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        // `to_char` formats timestamps as RFC 3339 in SQL so the
        // wire shape doesn't depend on a sqlx time-crate feature.
        let insert_sql = format!(
            "INSERT INTO knievel.projects (id, org_id, external_id, name)
             VALUES ($1, $2, $3, $4)
             RETURNING {PROJECT_SELECT_COLS}"
        );
        let row: Result<ProjectRow, _> = sqlx::query_as(&insert_sql)
            .bind(&new_id)
            .bind(&path_org_id)
            .bind(body.0.external_id.as_deref())
            .bind(&body.0.name)
            .fetch_one(&mut *tx)
            .await;

        match row {
            Ok(row) => {
                if let Err(e) = tx.commit().await {
                    tracing::error!(error = %e, "commit failed");
                    return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                        "db_error",
                        "could not commit transaction",
                    )));
                }
                CreateProjectResponse::Created(Json(row.into()))
            }
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("duplicate key") || msg.contains("unique constraint") {
                    CreateProjectResponse::Conflict(Json(ErrorEnvelope::of(
                        "external_id_conflict",
                        "external_id is already taken in this org",
                    )))
                } else {
                    tracing::error!(error = %e, "create_project insert failed");
                    CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                        "db_error",
                        "insert failed",
                    )))
                }
            }
        }
    }

    /// Read a single project by id (path).
    #[oai(
        path = "/v1/orgs/:org_id/projects/:project_id",
        method = "get",
        operation_id = "getProject"
    )]
    async fn get_project(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        project_id: Path<String>,
    ) -> GetProjectResponse {
        let principal = auth.0;
        let path_org_id = org_id.0;
        let path_project_id = project_id.0;

        if principal.org_id != path_org_id {
            return GetProjectResponse::Forbidden(Json(ErrorEnvelope::of(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::Reader) {
            return GetProjectResponse::Forbidden(Json(ErrorEnvelope::of(
                "role_insufficient",
                "reading projects requires reader or higher",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return GetProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "no_db",
                    "no database configured",
                )))
            }
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return GetProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        let select_sql =
            format!("SELECT {PROJECT_SELECT_COLS} FROM knievel.projects WHERE id = $1");
        let row: Result<Option<ProjectRow>, _> = sqlx::query_as(&select_sql)
            .bind(&path_project_id)
            .fetch_optional(&mut *tx)
            .await;

        match row {
            Ok(Some(row)) => GetProjectResponse::Ok(Json(row.into())),
            Ok(None) => GetProjectResponse::NotFound(Json(ErrorEnvelope::of(
                "not_found",
                "project not found",
            ))),
            Err(e) => {
                tracing::error!(error = %e, "get_project select failed");
                GetProjectResponse::Internal(Json(ErrorEnvelope::of("db_error", "select failed")))
            }
        }
    }
}

/// Generate a `pj_<12 hex chars>` id. Uses `OsRng` (already pulled
/// in via argon2's `password_hash::rand_core`) so we don't need a
/// direct `rand` dep.
fn random_pj_id() -> anyhow::Result<String> {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 6];
    let mut rng = OsRng;
    rng.fill_bytes(&mut bytes);
    let s: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("pj_{s}"))
}
