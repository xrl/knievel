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
use poem_openapi::{
    param::{Header, Path, Query},
    payload::Json,
    ApiResponse, Object, OpenApi,
};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::db;
use crate::idempotency::{self, CheckResult};
use crate::state::AppState;
use crate::taxonomy;

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

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateProjectRequest {
    /// Caller-assigned external id, unique within the org.
    pub external_id: Option<String>,
    pub name: String,
}

#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
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

/// Cursor-paginated envelope for `listProjects` (Phase 7.5).
/// Mirrors the shape from `listAdvertisers` etc.; `next_cursor`
/// is null today (orgs typically host single-digit project
/// counts, so the bounded-list path returns everything in one
/// page). Wired anyway so the SPA's pagination plumbing is
/// real now.
#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ProjectList {
    pub items: Vec<ProjectResponse>,
    pub next_cursor: Option<String>,
}

/// Org metadata, returned by `getOrg` (Phase 7.5). Same shape
/// as `ProjectResponse` since the columns mirror — orgs and
/// projects share the `id / external_id / name / is_active /
/// etag / created_at / updated_at` core.
#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrgResponse {
    pub id: String,
    pub external_id: Option<String>,
    pub name: String,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct OrgRow {
    id: String,
    external_id: Option<String>,
    name: String,
    is_active: bool,
    etag: String,
    created_at: String,
    updated_at: String,
}

const ORG_SELECT_COLS: &str = r#"
    id, external_id, name, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

impl From<OrgRow> for OrgResponse {
    fn from(r: OrgRow) -> Self {
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
    /// Fresh create, OR an idempotency replay. The
    /// `Idempotent-Replay: true` header lets the caller
    /// distinguish a cached return from a fresh one
    /// (`API.md` "Idempotency"). Absent on fresh executions.
    #[oai(status = 201)]
    Created(
        Json<ProjectResponse>,
        #[oai(header = "Idempotent-Replay")] Option<String>,
    ),
    /// Org mismatch between the principal and the path.
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    /// `externalId` already taken in this org, OR `Idempotency-Key`
    /// reused with a different body.
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

#[derive(ApiResponse)]
pub enum GetOrgResponse {
    #[oai(status = 200)]
    Ok(Json<OrgResponse>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum ListProjectsResponse {
    #[oai(status = 200)]
    Ok(Json<ProjectList>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[OpenApi(tag = "ApiTags::Orgs")]
impl OrgApi {
    /// Create a project under an org. Honors `Idempotency-Key`
    /// (24 h replay window per `API.md` "Idempotency"); `409
    /// idempotency_conflict` if the same key is reused with a
    /// different body. Returns `409 external_id_conflict` if the
    /// `externalId` is already taken in this org.
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
        #[oai(name = "Idempotency-Key")] idempotency_key: Header<Option<String>>,
        body: Json<CreateProjectRequest>,
    ) -> CreateProjectResponse {
        let principal = auth.0;
        let path_org_id = org_id.0;

        // Authz: org match + role >= org-admin.
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

        // Idempotency: check for an existing row keyed on
        // (org_id, NULL project, key, route, body_hash). The
        // route literal is the operation's `oai` path; if a future
        // refactor moves it, the same hash must move with it.
        const ROUTE: &str = "POST /v1/orgs/{org_id}/projects";
        let body_hash = match idempotency::body_hash(&body.0) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(error = %e, "idempotency body hash failed");
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "internal_error",
                    "could not hash request body",
                )));
            }
        };
        if let Some(key) = idempotency_key.0.as_deref() {
            match idempotency::check(&mut tx, &path_org_id, None, key, ROUTE, &body_hash).await {
                Ok(CheckResult::Replay { status, body }) => {
                    let parsed: Result<ProjectResponse, _> = serde_json::from_slice(&body);
                    match parsed {
                        Ok(resp) if status == 201 => {
                            return CreateProjectResponse::Created(Json(resp), Some("true".into()));
                        }
                        Ok(_) | Err(_) => {
                            tracing::warn!("idempotency replay payload incompatible; re-executing");
                            // Fall through to fresh execution below.
                        }
                    }
                }
                Ok(CheckResult::Conflict) => {
                    return CreateProjectResponse::Conflict(Json(ErrorEnvelope::of(
                        "idempotency_conflict",
                        "Idempotency-Key reused with a different request body",
                    )));
                }
                Ok(CheckResult::Fresh) => {} // proceed
                Err(e) => {
                    tracing::error!(error = %e, "idempotency check failed");
                    return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                        "db_error",
                        "idempotency check failed",
                    )));
                }
            }
        }

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

        let response: ProjectResponse = match row {
            Ok(row) => row.into(),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("duplicate key") || msg.contains("unique constraint") {
                    return CreateProjectResponse::Conflict(Json(ErrorEnvelope::of(
                        "external_id_conflict",
                        "external_id is already taken in this org",
                    )));
                }
                tracing::error!(error = %e, "create_project insert failed");
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "insert failed",
                )));
            }
        };

        // Bind knievel.project_id mid-transaction so the taxonomy
        // RLS policies (project_id-scoped) let the seed inserts
        // through, then seed default channels / priorities /
        // ad_types under the new project. Same transaction → seed
        // is atomic with the project create.
        if let Err(e) = sqlx::query("SELECT set_config('knievel.project_id', $1, true)")
            .bind(&response.id)
            .execute(&mut *tx)
            .await
        {
            tracing::error!(error = %e, "set_config(project_id) failed");
            return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                "db_error",
                "could not bind project for taxonomy seed",
            )));
        }
        if let Err(e) = taxonomy::seed_default_taxonomy(&mut tx, &path_org_id, &response.id).await {
            tracing::error!(error = %e, "default taxonomy seed failed");
            return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                "db_error",
                "default taxonomy seed failed",
            )));
        }

        // Store the idempotency row inside the same transaction so
        // a crash between insert and store can't leave a
        // half-applied state.
        if let Some(key) = idempotency_key.0.as_deref() {
            let stored_body = match serde_json::to_vec(&response) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "idempotency response serialization failed");
                    return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                        "internal_error",
                        "could not serialize response for idempotency cache",
                    )));
                }
            };
            if let Err(e) = idempotency::store(
                &mut tx,
                &path_org_id,
                None,
                key,
                ROUTE,
                &body_hash,
                201,
                &stored_body,
            )
            .await
            {
                tracing::error!(error = %e, "idempotency store failed");
                return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "idempotency store failed",
                )));
            }
        }

        if let Err(e) = tx.commit().await {
            tracing::error!(error = %e, "commit failed");
            return CreateProjectResponse::Internal(Json(ErrorEnvelope::of(
                "db_error",
                "could not commit transaction",
            )));
        }
        CreateProjectResponse::Created(Json(response), None)
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

    /// Org metadata (Phase 7.5). Powers the admin SPA's
    /// org-dashboard breadcrumbs + project-list page header.
    /// Multi-org access isn't a real feature yet; the auth
    /// check rejects when the principal's `org_id` doesn't
    /// match the path, so this is effectively "fetch my org."
    #[oai(path = "/v1/orgs/:org_id", method = "get", operation_id = "getOrg")]
    async fn get_org(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
    ) -> GetOrgResponse {
        let principal = auth.0;
        let path_org_id = org_id.0;

        if principal.org_id != path_org_id {
            return GetOrgResponse::Forbidden(Json(ErrorEnvelope::of(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::Reader) {
            return GetOrgResponse::Forbidden(Json(ErrorEnvelope::of(
                "role_insufficient",
                "reading org metadata requires reader or higher",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return GetOrgResponse::Internal(Json(ErrorEnvelope::of(
                    "no_db",
                    "no database configured",
                )))
            }
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return GetOrgResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        let select_sql =
            format!("SELECT {ORG_SELECT_COLS} FROM knievel.organizations WHERE id = $1");
        let row: Result<Option<OrgRow>, _> = sqlx::query_as(&select_sql)
            .bind(&path_org_id)
            .fetch_optional(&mut *tx)
            .await;
        match row {
            Ok(Some(row)) => GetOrgResponse::Ok(Json(row.into())),
            Ok(None) => {
                GetOrgResponse::NotFound(Json(ErrorEnvelope::of("not_found", "org not found")))
            }
            Err(e) => {
                tracing::error!(error = %e, "get_org select failed");
                GetOrgResponse::Internal(Json(ErrorEnvelope::of("db_error", "select failed")))
            }
        }
    }

    /// List projects under an org (Phase 7.5). The cursor
    /// envelope is wired so the SPA's pagination plumbing is
    /// real, but `next_cursor` is always `null` today — the
    /// `(created_at, id)` tuple-cursor that TEXT-id endpoints
    /// need is deferred to Phase 6.5 (per CLAUDE.md "Open
    /// known gaps"). For now an org's full project set comes
    /// back in one page; orgs typically host single-digit
    /// project counts, so this is fine.
    #[oai(
        path = "/v1/orgs/:org_id/projects",
        method = "get",
        operation_id = "listProjects"
    )]
    async fn list_projects(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        // `limit` accepted but capped (DoS protection); the
        // cursor envelope is non-paginating today.
        limit: Query<Option<i64>>,
    ) -> ListProjectsResponse {
        let principal = auth.0;
        let path_org_id = org_id.0;

        if principal.org_id != path_org_id {
            return ListProjectsResponse::Forbidden(Json(ErrorEnvelope::of(
                "wrong_tenant",
                "principal's org_id does not match the path",
            )));
        }
        if !principal.has_role_at_least(Role::Reader) {
            return ListProjectsResponse::Forbidden(Json(ErrorEnvelope::of(
                "role_insufficient",
                "listing projects requires reader or higher",
            )));
        }

        let limit_value = limit.0.unwrap_or(500).clamp(1, 500);

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return ListProjectsResponse::Internal(Json(ErrorEnvelope::of(
                    "no_db",
                    "no database configured",
                )))
            }
        };

        let mut tx = match db::begin_bound(pool, &path_org_id, None).await {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return ListProjectsResponse::Internal(Json(ErrorEnvelope::of(
                    "db_error",
                    "could not begin transaction",
                )));
            }
        };

        let sql = format!(
            "SELECT {PROJECT_SELECT_COLS} FROM knievel.projects \
             ORDER BY created_at DESC, id DESC LIMIT $1"
        );
        let q = sqlx::query_as::<_, ProjectRow>(&sql).bind(limit_value);
        match q.fetch_all(&mut *tx).await {
            Ok(rows) => ListProjectsResponse::Ok(Json(ProjectList {
                items: rows.into_iter().map(Into::into).collect(),
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list_projects select failed");
                ListProjectsResponse::Internal(Json(ErrorEnvelope::of("db_error", "select failed")))
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
