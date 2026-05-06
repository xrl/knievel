//! Ads — `/v1/projects/{projectId}/ads`.
//!
//! Phase 3.11. Inline-creative variant only — the body must
//! provide `creative_id`. The `oneOf` reference variant
//! (`ad_library_item_id`) is reserved by the schema (`ads_kind_check`
//! constraint) but lands as additive surface in 3.28 alongside
//! the Ad Library handler.
//!
//! Spec refs: `API.md` § 3.4.

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::batch::{classify_pg_error, BatchErrorDetail, BatchErrorEnvelope};
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct AdsApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateAdRequest {
    pub external_id: Option<String>,
    pub flight_id: i64,
    pub creative_id: i64,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateAdRequest {
    pub creative_id: Option<i64>,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Ad {
    pub id: i64,
    pub external_id: Option<String>,
    pub flight_id: i64,
    pub creative_id: Option<i64>,
    pub ad_library_item_id: Option<String>,
    pub weight: i32,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct AdList {
    pub items: Vec<Ad>,
    pub next_cursor: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertAdRow {
    pub external_id: String,
    pub flight_id: i64,
    pub creative_id: i64,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertAdsRequest {
    pub items: Vec<BatchUpsertAdRow>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertAdsResult {
    pub items: Vec<Ad>,
}

const COLS: &str = r#"
    id, external_id, flight_id, creative_id, ad_library_item_id,
    weight, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Ad>),
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
    Ok(Json<AdList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<Ad>),
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
    Ok(Json<Ad>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum BatchResp {
    #[oai(status = 200)]
    Ok(Json<BatchUpsertAdsResult>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 422)]
    PartialFailure(Json<BatchErrorEnvelope>),
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

#[OpenApi(tag = "ApiTags::Ads")]
impl AdsApi {
    #[oai(
        path = "/v1/projects/:project_id/ads",
        method = "post",
        operation_id = "createAd"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateAdRequest>,
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
            "INSERT INTO knievel.ads
                 (org_id, project_id, flight_id, creative_id,
                  external_id, weight, is_active)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, 100), COALESCE($7, true))
             RETURNING {COLS}"
        );
        let row: Result<Ad, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.flight_id)
            .bind(req.creative_id)
            .bind(req.external_id.as_deref())
            .bind(req.weight)
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;
        match row {
            Ok(a) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(a)),
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
                        "flight_id or creative_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "ad insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ads",
        method = "get",
        operation_id = "listAds"
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
        let sql = format!("SELECT {COLS} FROM knievel.ads ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, Ad>(&sql).fetch_all(&mut *tx).await {
            Ok(items) => ListResp::Ok(Json(AdList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list ads failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ads/:id",
        method = "get",
        operation_id = "getAd"
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
        let sql = format!("SELECT {COLS} FROM knievel.ads WHERE id = $1");
        match sqlx::query_as::<_, Ad>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(a)) => GetResp::Ok(Json(a)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "ad not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get ad failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ads/:id",
        method = "patch",
        operation_id = "updateAd"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateAdRequest>,
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
            "UPDATE knievel.ads SET
                 creative_id = COALESCE($2, creative_id),
                 weight = COALESCE($3, weight),
                 is_active = COALESCE($4, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Ad>(&sql)
            .bind(id)
            .bind(req.creative_id)
            .bind(req.weight)
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(a)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(a)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "ad not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update ad failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ads:batchUpsert",
        method = "post",
        operation_id = "batchUpsertAds"
    )]
    async fn batch_upsert(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<BatchUpsertAdsRequest>,
    ) -> BatchResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return BatchResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(BatchResp::Forbidden, e),
        };
        let total = req.items.len();
        let mut out: Vec<Ad> = Vec::with_capacity(total);
        let mut details: Vec<BatchErrorDetail> = Vec::new();
        let sql = format!(
            "INSERT INTO knievel.ads
                 (org_id, project_id, flight_id, creative_id,
                  external_id, weight, is_active)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, 100), COALESCE($7, true))
             ON CONFLICT (project_id, external_id) DO UPDATE SET
                 flight_id = EXCLUDED.flight_id,
                 creative_id = EXCLUDED.creative_id,
                 weight = EXCLUDED.weight,
                 is_active = COALESCE(EXCLUDED.is_active, knievel.ads.is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             RETURNING {COLS}"
        );
        for (idx, row) in req.items.iter().enumerate() {
            let r: Result<Ad, _> = sqlx::query_as(&sql)
                .bind(&principal.org_id)
                .bind(&pj)
                .bind(row.flight_id)
                .bind(row.creative_id)
                .bind(&row.external_id)
                .bind(row.weight)
                .bind(row.is_active)
                .fetch_one(&mut *tx)
                .await;
            match r {
                Ok(a) => out.push(a),
                Err(e) => {
                    let m = format!("{e}");
                    let (code, msg) = classify_pg_error(&m);
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field: if code == "fk_not_found" {
                            Some("flightId".into())
                        } else {
                            None
                        },
                        code: code.into(),
                        message: msg.unwrap_or("row failed validation").into(),
                    });
                    break;
                }
            }
        }
        if !details.is_empty() {
            let _ = tx.rollback().await;
            return BatchResp::PartialFailure(Json(BatchErrorEnvelope::partial_failure(
                total, details,
            )));
        }
        match tx.commit().await {
            Ok(()) => BatchResp::Ok(Json(BatchUpsertAdsResult { items: out })),
            Err(e) => {
                tracing::error!(error = %e, "batch upsert ads commit failed");
                BatchResp::Internal(Json(err("db_error", "commit failed")))
            }
        }
    }
}
