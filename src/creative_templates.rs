//! Creative Templates — `/v1/projects/{projectId}/creative-templates`.
//!
//! Phase 3.10. Cross-cutting risk #1: the `schema` field embeds an
//! arbitrary JSON Schema document, so the handler stores it as
//! `serde_json::Value`. poem-openapi treats `serde_json::Value` as
//! a free-form JSON `Any` schema in the generated OpenAPI; the
//! Phase 3.10 spike test
//! `creative_template_json_schema_round_trips` confirms a typical
//! schema body survives POST → GET intact.
//!
//! Spec refs: `API.md` § 3.6, `REQUIREMENTS.md` § 12 cross-cutting
//! risk (1).

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct CreativeTemplatesApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateCreativeTemplateRequest {
    pub external_id: Option<String>,
    pub name: String,
    /// Arbitrary JSON Schema document; not parsed by knievel.
    pub schema: serde_json::Value,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateCreativeTemplateRequest {
    pub name: Option<String>,
    pub schema: Option<serde_json::Value>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct CreativeTemplate {
    pub id: i64,
    pub external_id: Option<String>,
    pub name: String,
    pub schema: serde_json::Value,
    pub version: i32,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreativeTemplateList {
    pub items: Vec<CreativeTemplate>,
    pub next_cursor: Option<String>,
}

const COLS: &str = r#"
    id, external_id, name, schema, version, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<CreativeTemplate>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum ListResp {
    #[oai(status = 200)]
    Ok(Json<CreativeTemplateList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<CreativeTemplate>),
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
    Ok(Json<CreativeTemplate>),
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
impl CreativeTemplatesApi {
    #[oai(
        path = "/v1/projects/:project_id/creative-templates",
        method = "post",
        operation_id = "createCreativeTemplate"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateCreativeTemplateRequest>,
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
            "INSERT INTO knievel.creative_templates
                 (org_id, project_id, external_id, name, schema)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING {COLS}"
        );
        let row: Result<CreativeTemplate, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(&req.schema)
            .fetch_one(&mut *tx)
            .await;
        match row {
            Ok(t) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(t)),
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
                        "external_id or name is already taken in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "creative_template insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/creative-templates",
        method = "get",
        operation_id = "listCreativeTemplates"
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
        let sql =
            format!("SELECT {COLS} FROM knievel.creative_templates ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, CreativeTemplate>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => ListResp::Ok(Json(CreativeTemplateList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list creative_templates failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/creative-templates/:id",
        method = "get",
        operation_id = "getCreativeTemplate"
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
        let sql = format!("SELECT {COLS} FROM knievel.creative_templates WHERE id = $1");
        match sqlx::query_as::<_, CreativeTemplate>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(t)) => GetResp::Ok(Json(t)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "creative_template not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get creative_template failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    /// PATCH bumps `version` whenever the schema field is provided
    /// (per `API.md` § 3.6 — schema changes are versioned but do
    /// not retroactively re-validate existing creatives).
    #[oai(
        path = "/v1/projects/:project_id/creative-templates/:id",
        method = "patch",
        operation_id = "updateCreativeTemplate"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateCreativeTemplateRequest>,
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
        let bump = req.schema.is_some();
        let sql = format!(
            "UPDATE knievel.creative_templates
             SET name = COALESCE($2, name),
                 schema = COALESCE($3, schema),
                 version = version + CASE WHEN $4 THEN 1 ELSE 0 END,
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, CreativeTemplate>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.schema.as_ref())
            .bind(bump)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(t)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(t)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "creative_template not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update creative_template failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }
}
