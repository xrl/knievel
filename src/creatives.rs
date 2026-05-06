//! Creatives — `/v1/projects/{projectId}/creatives`.
//!
//! Phase 3.10. The `kind` discriminator is `image | html | native`
//! (`API.md` § 3.5 oneOf). For v0 the table stores the union of all
//! kind-specific fields with a CHECK constraint on `kind`; the
//! handler accepts the full union and the kind-specific validation
//! ("image creatives must have imageUrl") is enforced at write
//! time by sending NULLs through.
//!
//! Image upload (`POST .../{id}/image`) lands in 3.29.

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct CreativesApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateCreativeRequest {
    pub external_id: Option<String>,
    pub advertiser_id: i64,
    pub name: Option<String>,
    /// "image", "html", or "native".
    pub kind: String,
    pub image_url: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub alt: Option<String>,
    pub body: Option<String>,
    pub template_id: Option<i64>,
    pub values: Option<serde_json::Value>,
    pub click_through_url: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Creative {
    pub id: i64,
    pub external_id: Option<String>,
    pub advertiser_id: i64,
    pub name: Option<String>,
    pub kind: String,
    pub image_url: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub alt: Option<String>,
    pub body: Option<String>,
    pub template_id: Option<i64>,
    pub values: Option<serde_json::Value>,
    pub click_through_url: Option<String>,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreativeList {
    pub items: Vec<Creative>,
    pub next_cursor: Option<String>,
}

const COLS: &str = r#"
    id, external_id, advertiser_id, name, kind,
    image_url, width, height, alt, body,
    template_id, values, click_through_url,
    is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Creative>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
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
    Ok(Json<CreativeList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<Creative>),
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
impl CreativesApi {
    #[oai(
        path = "/v1/projects/:project_id/creatives",
        method = "post",
        operation_id = "createCreative"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateCreativeRequest>,
    ) -> CreateResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;

        // Validate kind + per-kind required fields. Spec § 3.5:
        // image needs imageUrl; html needs body; native needs
        // template_id + values.
        let kind_err = match req.kind.as_str() {
            "image" => req
                .image_url
                .as_ref()
                .map_or(Some("image creatives require image_url"), |_| None),
            "html" => req
                .body
                .as_ref()
                .map_or(Some("html creatives require body"), |_| None),
            "native" => {
                if req.template_id.is_none() || req.values.is_none() {
                    Some("native creatives require template_id and values")
                } else {
                    None
                }
            }
            _ => Some("kind must be one of image, html, native"),
        };
        if let Some(msg) = kind_err {
            return CreateResp::BadRequest(Json(err("invalid_creative", msg)));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return CreateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(CreateResp::Forbidden, e),
        };

        let sql = format!(
            "INSERT INTO knievel.creatives
                 (org_id, project_id, advertiser_id, external_id, name, kind,
                  image_url, width, height, alt, body, template_id, values,
                  click_through_url, is_active)
             VALUES ($1, $2, $3, $4, $5, $6,
                     $7, $8, $9, $10, $11, $12, $13,
                     $14, COALESCE($15, true))
             RETURNING {COLS}"
        );
        let row: Result<Creative, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.advertiser_id)
            .bind(req.external_id.as_deref())
            .bind(req.name.as_deref())
            .bind(&req.kind)
            .bind(req.image_url.as_deref())
            .bind(req.width)
            .bind(req.height)
            .bind(req.alt.as_deref())
            .bind(req.body.as_deref())
            .bind(req.template_id)
            .bind(req.values.as_ref())
            .bind(req.click_through_url.as_deref())
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;
        match row {
            Ok(c) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(c)),
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
                        "advertiser_id or template_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "creative insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/creatives",
        method = "get",
        operation_id = "listCreatives"
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
        let sql = format!("SELECT {COLS} FROM knievel.creatives ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, Creative>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => ListResp::Ok(Json(CreativeList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list creatives failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/creatives/:id",
        method = "get",
        operation_id = "getCreative"
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
        let sql = format!("SELECT {COLS} FROM knievel.creatives WHERE id = $1");
        match sqlx::query_as::<_, Creative>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(c)) => GetResp::Ok(Json(c)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "creative not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get creative failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }
}
