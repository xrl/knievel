//! Campaigns — `/v1/projects/{projectId}/campaigns`.
//!
//! Phase 3.9. Same handler shape as `advertisers`; campaigns
//! carry an `advertiser_id` FK to `advertisers`.
//!
//! Spec refs: `API.md` § 3.2.

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::batch::{classify_pg_error, BatchErrorDetail, BatchErrorEnvelope};
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct CampaignsApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateCampaignRequest {
    pub external_id: Option<String>,
    pub advertiser_id: i64,
    pub name: String,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateCampaignRequest {
    pub name: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Campaign {
    pub id: i64,
    pub external_id: Option<String>,
    pub advertiser_id: i64,
    pub name: String,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CampaignList {
    pub items: Vec<Campaign>,
    pub next_cursor: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertCampaignRow {
    pub external_id: String,
    pub advertiser_id: i64,
    pub name: String,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertCampaignsRequest {
    pub items: Vec<BatchUpsertCampaignRow>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertCampaignsResult {
    pub items: Vec<Campaign>,
}

const COLS: &str = r#"
    id, external_id, advertiser_id, name, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Campaign>),
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
    Ok(Json<CampaignList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<Campaign>),
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
    Ok(Json<Campaign>),
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
    Ok(Json<BatchUpsertCampaignsResult>),
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

#[OpenApi]
impl CampaignsApi {
    #[oai(
        path = "/v1/projects/:project_id/campaigns",
        method = "post",
        operation_id = "createCampaign"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateCampaignRequest>,
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
            "INSERT INTO knievel.campaigns
                 (org_id, project_id, advertiser_id, external_id, name, is_active)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, true))
             RETURNING {COLS}"
        );
        let row: Result<Campaign, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.advertiser_id)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
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
                        "advertiser_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "campaign insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/campaigns",
        method = "get",
        operation_id = "listCampaigns"
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
        let sql = format!("SELECT {COLS} FROM knievel.campaigns ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, Campaign>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => ListResp::Ok(Json(CampaignList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list campaigns failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/campaigns/:id",
        method = "get",
        operation_id = "getCampaign"
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
        let sql = format!("SELECT {COLS} FROM knievel.campaigns WHERE id = $1");
        match sqlx::query_as::<_, Campaign>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(c)) => GetResp::Ok(Json(c)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "campaign not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get campaign failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/campaigns/:id",
        method = "patch",
        operation_id = "updateCampaign"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateCampaignRequest>,
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
            "UPDATE knievel.campaigns
             SET name = COALESCE($2, name),
                 is_active = COALESCE($3, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Campaign>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(c)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(c)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "campaign not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update campaign failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/campaigns:batchUpsert",
        method = "post",
        operation_id = "batchUpsertCampaigns"
    )]
    async fn batch_upsert(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<BatchUpsertCampaignsRequest>,
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
        let mut out: Vec<Campaign> = Vec::with_capacity(total);
        let mut details: Vec<BatchErrorDetail> = Vec::new();
        let sql = format!(
            "INSERT INTO knievel.campaigns
                 (org_id, project_id, advertiser_id, external_id, name, is_active)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, true))
             ON CONFLICT (project_id, external_id) DO UPDATE SET
                 advertiser_id = EXCLUDED.advertiser_id,
                 name = EXCLUDED.name,
                 is_active = COALESCE(EXCLUDED.is_active, knievel.campaigns.is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             RETURNING {COLS}"
        );
        for (idx, row) in req.items.iter().enumerate() {
            let r: Result<Campaign, _> = sqlx::query_as(&sql)
                .bind(&principal.org_id)
                .bind(&pj)
                .bind(row.advertiser_id)
                .bind(&row.external_id)
                .bind(&row.name)
                .bind(row.is_active)
                .fetch_one(&mut *tx)
                .await;
            match r {
                Ok(c) => out.push(c),
                Err(e) => {
                    let m = format!("{e}");
                    let (code, msg) = classify_pg_error(&m);
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field: if code == "fk_not_found" {
                            Some("advertiserId".into())
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
            Ok(()) => BatchResp::Ok(Json(BatchUpsertCampaignsResult { items: out })),
            Err(e) => {
                tracing::error!(error = %e, "batch upsert campaigns commit failed");
                BatchResp::Internal(Json(err("db_error", "commit failed")))
            }
        }
    }
}
