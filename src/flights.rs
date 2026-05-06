//! Flights — `/v1/projects/{projectId}/flights`.
//!
//! Phase 3.9. Same handler shape as campaigns; flights carry a
//! `campaign_id` FK plus three int64[] arrays for inventory
//! targeting (`site_ids`, `zone_ids`, `ad_types`) and an optional
//! date window.
//!
//! Spec refs: `API.md` § 3.3.

// Flight is a wide struct (10+ fields), so each ApiResponse
// variant carrying `Json<Flight>` is large compared to the
// error-envelope variants. Boxing for one allocation per
// response isn't worth obscuring the typed handler return.
#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{
    param::{Path, Query},
    payload::Json,
    ApiResponse, Object, OpenApi,
};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::batch::{classify_pg_error, BatchErrorDetail, BatchErrorEnvelope};
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct FlightsApi;

const CURSOR_KIND: &str = "flights";

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateFlightRequest {
    pub external_id: Option<String>,
    pub campaign_id: i64,
    pub name: String,
    pub priority_id: i64,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub site_ids: Option<Vec<i64>>,
    pub zone_ids: Option<Vec<i64>>,
    pub ad_types: Vec<i64>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateFlightRequest {
    pub name: Option<String>,
    pub priority_id: Option<i64>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub site_ids: Option<Vec<i64>>,
    pub zone_ids: Option<Vec<i64>>,
    pub ad_types: Option<Vec<i64>>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Flight {
    pub id: i64,
    pub external_id: Option<String>,
    pub campaign_id: i64,
    pub name: String,
    pub priority_id: i64,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub site_ids: Vec<i64>,
    pub zone_ids: Vec<i64>,
    pub ad_types: Vec<i64>,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct FlightList {
    pub items: Vec<Flight>,
    pub next_cursor: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertFlightRow {
    pub external_id: String,
    pub campaign_id: i64,
    pub name: String,
    pub priority_id: i64,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub site_ids: Option<Vec<i64>>,
    pub zone_ids: Option<Vec<i64>>,
    pub ad_types: Vec<i64>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertFlightsRequest {
    pub items: Vec<BatchUpsertFlightRow>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertFlightsResult {
    pub items: Vec<Flight>,
}

const COLS: &str = r#"
    id, external_id, campaign_id, name, priority_id,
    to_char(start_date AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS start_date,
    to_char(end_date AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS end_date,
    site_ids, zone_ids, ad_types, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Flight>),
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
    Ok(Json<FlightList>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<Flight>),
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
    Ok(Json<Flight>),
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
    Ok(Json<BatchUpsertFlightsResult>),
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

#[OpenApi(tag = "ApiTags::Flights")]
impl FlightsApi {
    #[oai(
        path = "/v1/projects/:project_id/flights",
        method = "post",
        operation_id = "createFlight"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateFlightRequest>,
    ) -> CreateResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;

        // ad_types must be non-empty per API.md § 3.3.
        if req.ad_types.is_empty() {
            return CreateResp::BadRequest(Json(err(
                "ad_types_required",
                "ad_types must be a non-empty array",
            )));
        }

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return CreateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(CreateResp::Forbidden, e),
        };

        // Parse start/end_date as timestamptz at the SQL layer to
        // avoid pulling in chrono. NULL stays NULL.
        let sql = format!(
            "INSERT INTO knievel.flights
                 (org_id, project_id, campaign_id, external_id, name, priority_id,
                  start_date, end_date, site_ids, zone_ids, ad_types, is_active)
             VALUES ($1, $2, $3, $4, $5, $6,
                     $7::timestamptz, $8::timestamptz,
                     COALESCE($9, '{{}}'::bigint[]),
                     COALESCE($10, '{{}}'::bigint[]),
                     $11,
                     COALESCE($12, true))
             RETURNING {COLS}"
        );
        let row: Result<Flight, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.campaign_id)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(req.priority_id)
            .bind(req.start_date.as_deref())
            .bind(req.end_date.as_deref())
            .bind(req.site_ids.as_deref())
            .bind(req.zone_ids.as_deref())
            .bind(&req.ad_types)
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;

        match row {
            Ok(f) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(f)),
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
                        "campaign_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "flight insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/flights",
        method = "get",
        operation_id = "listFlights"
    )]
    async fn list(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        limit: Query<Option<i64>>,
        cursor: Query<Option<String>>,
    ) -> ListResp {
        let principal = auth.0;
        let pj = project_id.0;
        let resolved = match crate::pagination::resolve(limit.0, cursor.0.as_deref(), CURSOR_KIND) {
            Ok(r) => r,
            Err(e) => return ListResp::BadRequest(Json(err(e.code(), e.message()))),
        };
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ListResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(ListResp::Forbidden, e),
        };
        let sql = match resolved.after_id {
            None => format!("SELECT {COLS} FROM knievel.flights ORDER BY id DESC LIMIT $1"),
            Some(_) => format!(
                "SELECT {COLS} FROM knievel.flights WHERE id < $2 ORDER BY id DESC LIMIT $1"
            ),
        };
        let q = sqlx::query_as::<_, Flight>(&sql).bind(resolved.bumped_limit);
        let q = match resolved.after_id {
            Some(after) => q.bind(after),
            None => q,
        };
        match q.fetch_all(&mut *tx).await {
            Ok(mut rows) => {
                let next_cursor =
                    crate::pagination::next_cursor(&rows, &resolved, CURSOR_KIND, |r| r.id);
                rows.truncate(resolved.effective_limit as usize);
                ListResp::Ok(Json(FlightList {
                    items: rows,
                    next_cursor,
                }))
            }
            Err(e) => {
                tracing::error!(error = %e, "list flights failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/flights/:id",
        method = "get",
        operation_id = "getFlight"
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
        let sql = format!("SELECT {COLS} FROM knievel.flights WHERE id = $1");
        match sqlx::query_as::<_, Flight>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(f)) => GetResp::Ok(Json(f)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "flight not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get flight failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/flights/:id",
        method = "patch",
        operation_id = "updateFlight"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateFlightRequest>,
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
            "UPDATE knievel.flights SET
                 name = COALESCE($2, name),
                 priority_id = COALESCE($3, priority_id),
                 start_date = COALESCE($4::timestamptz, start_date),
                 end_date = COALESCE($5::timestamptz, end_date),
                 site_ids = COALESCE($6, site_ids),
                 zone_ids = COALESCE($7, zone_ids),
                 ad_types = COALESCE($8, ad_types),
                 is_active = COALESCE($9, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Flight>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.priority_id)
            .bind(req.start_date.as_deref())
            .bind(req.end_date.as_deref())
            .bind(req.site_ids.as_deref())
            .bind(req.zone_ids.as_deref())
            .bind(req.ad_types.as_deref())
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(f)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(f)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "flight not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update flight failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/flights:batchUpsert",
        method = "post",
        operation_id = "batchUpsertFlights"
    )]
    async fn batch_upsert(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<BatchUpsertFlightsRequest>,
    ) -> BatchResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return BatchResp::Internal(Json(err("no_db", "no database configured"))),
        };
        // Pre-validate ad_types non-empty per `API.md` § 3.3 BEFORE
        // touching the DB — these are validation_failed, not
        // FK/unique violations.
        for (idx, row) in req.items.iter().enumerate() {
            if row.ad_types.is_empty() {
                return BatchResp::PartialFailure(Json(BatchErrorEnvelope::partial_failure(
                    req.items.len(),
                    vec![BatchErrorDetail {
                        index: idx as i32,
                        field: Some("adTypes".into()),
                        code: "validation_failed".into(),
                        message: "ad_types must be a non-empty array".into(),
                    }],
                )));
            }
        }
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(BatchResp::Forbidden, e),
        };
        let total = req.items.len();
        let mut out: Vec<Flight> = Vec::with_capacity(total);
        let mut details: Vec<BatchErrorDetail> = Vec::new();
        let sql = format!(
            "INSERT INTO knievel.flights
                 (org_id, project_id, campaign_id, external_id, name, priority_id,
                  start_date, end_date, site_ids, zone_ids, ad_types, is_active)
             VALUES ($1, $2, $3, $4, $5, $6,
                     $7::timestamptz, $8::timestamptz,
                     COALESCE($9, '{{}}'::bigint[]),
                     COALESCE($10, '{{}}'::bigint[]),
                     $11, COALESCE($12, true))
             ON CONFLICT (project_id, external_id) DO UPDATE SET
                 campaign_id = EXCLUDED.campaign_id,
                 name = EXCLUDED.name,
                 priority_id = EXCLUDED.priority_id,
                 start_date = EXCLUDED.start_date,
                 end_date = EXCLUDED.end_date,
                 site_ids = EXCLUDED.site_ids,
                 zone_ids = EXCLUDED.zone_ids,
                 ad_types = EXCLUDED.ad_types,
                 is_active = COALESCE(EXCLUDED.is_active, knievel.flights.is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             RETURNING {COLS}"
        );
        for (idx, row) in req.items.iter().enumerate() {
            let r: Result<Flight, _> = sqlx::query_as(&sql)
                .bind(&principal.org_id)
                .bind(&pj)
                .bind(row.campaign_id)
                .bind(&row.external_id)
                .bind(&row.name)
                .bind(row.priority_id)
                .bind(row.start_date.as_deref())
                .bind(row.end_date.as_deref())
                .bind(row.site_ids.as_deref())
                .bind(row.zone_ids.as_deref())
                .bind(&row.ad_types)
                .bind(row.is_active)
                .fetch_one(&mut *tx)
                .await;
            match r {
                Ok(f) => out.push(f),
                Err(e) => {
                    let m = format!("{e}");
                    let (code, msg) = classify_pg_error(&m);
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field: if code == "fk_not_found" {
                            Some("campaignId".into())
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
            Ok(()) => BatchResp::Ok(Json(BatchUpsertFlightsResult { items: out })),
            Err(e) => {
                tracing::error!(error = %e, "batch upsert flights commit failed");
                BatchResp::Internal(Json(err("db_error", "commit failed")))
            }
        }
    }
}
