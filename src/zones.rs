//! Zones — `/v1/projects/{projectId}/zones`.
//!
//! Phase 3.12. Zones carry a `site_id` FK; otherwise they're a
//! straight CRUD shape.
//!
//! Spec refs: `API.md` § 3.8.

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct ZonesApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateZoneRequest {
    pub external_id: Option<String>,
    pub site_id: i64,
    pub name: String,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateZoneRequest {
    pub name: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Zone {
    pub id: i64,
    pub external_id: Option<String>,
    pub site_id: i64,
    pub name: String,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ZoneList {
    pub items: Vec<Zone>,
    pub next_cursor: Option<String>,
}

const COLS: &str = r#"
    id, external_id, site_id, name, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Zone>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    #[oai(status = 422)]
    Unprocessable(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum ListResp {
    #[oai(status = 200)]
    Ok(Json<ZoneList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<Zone>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum UpdateResp {
    #[oai(status = 200)]
    Ok(Json<Zone>),
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
fn forbid<R, F: FnOnce(Json<ErrorEnvelope>) -> R>(f: F, e: AuthzError) -> R {
    f(Json(err(e.code(), e.message())))
}

#[OpenApi]
impl ZonesApi {
    #[oai(
        path = "/v1/projects/:project_id/zones",
        method = "post",
        operation_id = "createZone"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateZoneRequest>,
    ) -> CreateResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return CreateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(CreateResp::Forbidden, e),
        };
        let sql = format!(
            "INSERT INTO knievel.zones
                 (org_id, project_id, site_id, external_id, name, is_active)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, true))
             RETURNING {COLS}"
        );
        let row: Result<Zone, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.site_id)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;
        match row {
            Ok(z) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(z)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    CreateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Err(e) => {
                let m = format!("{e}");
                if m.contains("duplicate key") || m.contains("unique constraint") {
                    CreateResp::Conflict(Json(err(
                        "external_id_conflict",
                        "external_id is already taken in this project",
                    )))
                } else if m.contains("foreign key") {
                    CreateResp::Unprocessable(Json(err(
                        "fk_not_found",
                        "site_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "zone insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/zones",
        method = "get",
        operation_id = "listZones"
    )]
    async fn list(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
    ) -> ListResp {
        let principal = auth.0;
        let pj = project_id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ListResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(ListResp::Forbidden, e),
        };
        let sql = format!("SELECT {COLS} FROM knievel.zones ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, Zone>(&sql).fetch_all(&mut *tx).await {
            Ok(items) => ListResp::Ok(Json(ZoneList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list zones failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/zones/:id",
        method = "get",
        operation_id = "getZone"
    )]
    async fn get(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
    ) -> GetResp {
        let principal = auth.0;
        let pj = project_id.0;
        let id = id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return GetResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(GetResp::Forbidden, e),
        };
        let sql = format!("SELECT {COLS} FROM knievel.zones WHERE id = $1");
        match sqlx::query_as::<_, Zone>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(z)) => GetResp::Ok(Json(z)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "zone not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get zone failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/zones/:id",
        method = "patch",
        operation_id = "updateZone"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateZoneRequest>,
    ) -> UpdateResp {
        let principal = auth.0;
        let pj = project_id.0;
        let id = id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return UpdateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(UpdateResp::Forbidden, e),
        };
        let sql = format!(
            "UPDATE knievel.zones SET
                 name = COALESCE($2, name),
                 is_active = COALESCE($3, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Zone>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(z)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(z)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "zone not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update zone failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }
}
