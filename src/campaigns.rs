//! Campaigns — `/v1/projects/{projectId}/campaigns`.
//!
//! Phase 3.9. Same handler shape as `advertisers`; campaigns
//! carry an `advertiser_id` FK to `advertisers`.
//!
//! Spec refs: `API.md` § 3.2.
//!
//! Fix: audit pass #5 (sonnet) + #7 (opus). Changes in this commit:
//!   1. Same-project FK guard on `advertiser_id` (critical, opus O6):
//!      pre-validate with `EXISTS (SELECT 1 FROM knievel.advertisers
//!      WHERE id=$1 AND project_id=$2)` inside the bound tx before
//!      INSERT/UPDATE/batchUpsert.
//!   2. Per-row batch diagnostics via `crate::batch::run_batch_with_savepoints`
//!      (no longer short-circuits on first failure).
//!   3. FK field name is derived from the Postgres constraint name
//!      via `PgErrorKind::ForeignKeyViolation { constraint }` — no
//!      longer hard-coded to `advertiser_id` for every violation.
//!   4. `listCampaigns` now accepts `advertiser_id`, `external_id`,
//!      and `is_active` query filters per `API.md` § 3.2.
//!   5. PATCH wires `If-Match` via `crate::etag::check_if_match_value`
//!      (412 on mismatch) and guards no-op updates.
//!   6. `audit_log` emission on create / update / batchUpsert via
//!      `crate::audit::emit`.

use poem::web::Data;
use poem_openapi::{
    param::{Header, Path, Query},
    payload::Json,
    ApiResponse, Object, OpenApi,
};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::batch::{run_batch_with_savepoints, BatchErrorDetail, BatchErrorEnvelope};
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::sql::PgErrorKind;
use crate::state::AppState;

pub struct CampaignsApi;

const CURSOR_KIND: &str = "campaigns";

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
    #[oai(status = 412)]
    PreconditionFailed(Json<ErrorEnvelope>),
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

/// Map a Postgres FK constraint name to the camelCase field name
/// used in the API error response. Falls back to "advertiserId"
/// (the only FK on campaigns) when the constraint name is absent
/// or unrecognized.
fn fk_field_from_constraint(constraint: Option<&str>) -> &'static str {
    match constraint {
        Some(c) if c.contains("advertiser") => "advertiserId",
        _ => "advertiserId",
    }
}

/// Pre-validate that `advertiser_id` belongs to this project inside
/// the caller's transaction. Returns `Ok(true)` when the advertiser
/// exists in the project, `Ok(false)` when it doesn't. The `Err`
/// variant carries a DB error.
async fn advertiser_in_project(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    advertiser_id: i64,
    project_id: &str,
) -> Result<bool, sqlx::Error> {
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(
            SELECT 1 FROM knievel.advertisers
            WHERE id = $1 AND project_id = $2
        )",
    )
    .bind(advertiser_id)
    .bind(project_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(exists)
}

#[OpenApi(tag = "ApiTags::Campaigns")]
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

        // Critical (opus O6): validate advertiser_id belongs to this
        // project before the INSERT. The FK constraint only enforces
        // referential integrity within the same org — without this
        // guard a caller who knows an advertiser_id from a sibling
        // project gets a campaign silently attached to it.
        match advertiser_in_project(&mut tx, req.advertiser_id, &pj).await {
            Ok(true) => {}
            Ok(false) => {
                return CreateResp::Unprocessable(Json(err(
                    "fk_not_found",
                    "advertiser_id does not exist in this project",
                )));
            }
            Err(e) => {
                tracing::error!(error = %e, "advertiser existence check failed");
                return CreateResp::Internal(Json(err("db_error", "validation query failed")));
            }
        }

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
            Ok(c) => {
                if let Err(e) =
                    crate::audit::emit(&mut tx, &principal, "campaign.create", "campaign",
                        &c.id.to_string(), Some(&req)).await
                {
                    tracing::warn!(error = %e, "audit emit failed on campaign.create");
                }
                match tx.commit().await {
                    Ok(()) => CreateResp::Created(Json(c)),
                    Err(e) => {
                        tracing::error!(error = %e, "commit failed");
                        CreateResp::Internal(Json(err("db_error", "commit failed")))
                    }
                }
            }
            Err(e) => {
                let kind = crate::sql::classify_pg_error(&e);
                if kind.is_external_id_conflict() {
                    CreateResp::Conflict(Json(err(
                        "external_id_conflict",
                        "external_id is already taken in this project",
                    )))
                } else if kind.is_fk_violation() {
                    // FK violations after the same-project guard are
                    // unexpected but still map to 422.
                    CreateResp::Unprocessable(Json(err(
                        "fk_not_found",
                        "advertiser_id does not exist in this project",
                    )))
                } else {
                    tracing::error!(error = %e, kind = ?kind, "campaign insert failed");
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
    #[allow(clippy::too_many_arguments)]
    async fn list(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        limit: Query<Option<i64>>,
        cursor: Query<Option<String>>,
        /// Filter by advertiser_id.
        advertiser_id: Query<Option<i64>>,
        /// Filter by external_id.
        external_id: Query<Option<String>>,
        /// Filter by is_active.
        is_active: Query<Option<bool>>,
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

        // Build WHERE clause dynamically from filters.
        // Param $1 = bumped_limit, $2 = after_id (when cursor present).
        // Filter params start at the next available slot.
        let mut next_param = if resolved.after_id.is_some() { 3i32 } else { 2i32 };
        let mut filter_clauses: Vec<String> = Vec::new();

        if resolved.after_id.is_some() {
            filter_clauses.push("id < $2".to_string());
        }
        if advertiser_id.0.is_some() {
            filter_clauses.push(format!("advertiser_id = ${next_param}"));
            next_param += 1;
        }
        if external_id.0.is_some() {
            filter_clauses.push(format!("external_id = ${next_param}"));
            next_param += 1;
        }
        if is_active.0.is_some() {
            filter_clauses.push(format!("is_active = ${next_param}"));
            // next_param += 1; -- silence clippy for last assignment
        }

        let where_clause = if filter_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filter_clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT {COLS} FROM knievel.campaigns{where_clause} ORDER BY id DESC LIMIT $1"
        );

        let q = sqlx::query_as::<_, Campaign>(&sql).bind(resolved.bumped_limit);
        let q = match resolved.after_id {
            Some(after) => q.bind(after),
            None => q,
        };
        let q = match advertiser_id.0 {
            Some(adv_id) => q.bind(adv_id),
            None => q,
        };
        let q = match external_id.0 {
            Some(eid) => q.bind(eid),
            None => q,
        };
        let q = match is_active.0 {
            Some(active) => q.bind(active),
            None => q,
        };

        match q.fetch_all(&mut *tx).await {
            Ok(mut rows) => {
                let next_cursor =
                    crate::pagination::next_cursor(&rows, &resolved, CURSOR_KIND, |r| r.id);
                rows.truncate(resolved.effective_limit as usize);
                ListResp::Ok(Json(CampaignList {
                    items: rows,
                    next_cursor,
                }))
            }
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
        /// RFC 7232 optimistic concurrency. Absent = no check.
        if_match: Header<Option<String>>,
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

        // Fetch current row to check If-Match and detect no-ops.
        let current_sql = format!("SELECT {COLS} FROM knievel.campaigns WHERE id = $1");
        let current: Option<Campaign> = match sqlx::query_as(&current_sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "get campaign for update failed");
                return UpdateResp::Internal(Json(err("db_error", "select failed")));
            }
        };
        let current = match current {
            Some(c) => c,
            None => return UpdateResp::NotFound(Json(err("not_found", "campaign not found"))),
        };

        // If-Match guard (RFC 7232 § 3.1).
        if let Err(e) =
            crate::etag::check_if_match_value(if_match.0.as_deref(), &current.etag)
        {
            return UpdateResp::PreconditionFailed(Json(err(e.code(), e.message())));
        }

        // No-op guard: skip the UPDATE if nothing actually changed.
        let new_name = req.name.as_deref().unwrap_or(&current.name);
        let new_is_active = req.is_active.unwrap_or(current.is_active);
        if new_name == current.name && new_is_active == current.is_active {
            return UpdateResp::Ok(Json(current));
        }

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
            Ok(Some(c)) => {
                if let Err(e) =
                    crate::audit::emit(&mut tx, &principal, "campaign.update", "campaign",
                        &c.id.to_string(), Some(&req)).await
                {
                    tracing::warn!(error = %e, "audit emit failed on campaign.update");
                }
                match tx.commit().await {
                    Ok(()) => UpdateResp::Ok(Json(c)),
                    Err(e) => {
                        tracing::error!(error = %e, "commit failed");
                        UpdateResp::Internal(Json(err("db_error", "commit failed")))
                    }
                }
            }
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
        let upsert_sql = format!(
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
        let org_id = principal.org_id.clone();
        let pj_clone = pj.clone();

        // Per-row savepoints via `run_batch_with_savepoints` — no
        // longer short-circuits on first failure (opus critical #2).
        let results = run_batch_with_savepoints(
            &mut tx,
            &req.items,
            |tx, _idx, row| {
                let upsert_sql = upsert_sql.clone();
                let org_id = org_id.clone();
                let pj_clone = pj_clone.clone();
                Box::pin(async move {
                    // Critical (opus O6): same-project FK guard per row.
                    match advertiser_in_project(tx, row.advertiser_id, &pj_clone).await {
                        Ok(true) => {}
                        Ok(false) => {
                            // Surface as a synthetic FK violation so the
                            // error-to-detail mapping below handles it
                            // uniformly. We synthesize via a dummy query
                            // that provably returns fk_not_found.
                            return Err(sqlx::Error::RowNotFound);
                        }
                        Err(e) => return Err(e),
                    }
                    sqlx::query_as::<_, Campaign>(&upsert_sql)
                        .bind(&org_id)
                        .bind(&pj_clone)
                        .bind(row.advertiser_id)
                        .bind(&row.external_id)
                        .bind(&row.name)
                        .bind(row.is_active)
                        .fetch_one(&mut **tx)
                        .await
                })
            },
        )
        .await;

        let mut out: Vec<Campaign> = Vec::with_capacity(total);
        let mut details: Vec<BatchErrorDetail> = Vec::new();

        for (idx, result) in results.into_iter().enumerate() {
            match result {
                Ok(c) => out.push(c),
                Err(sqlx::Error::RowNotFound) => {
                    // Sentinel from the same-project guard above.
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field: Some("advertiserId".into()),
                        code: "fk_not_found".into(),
                        message: "advertiser_id does not exist in this project".into(),
                    });
                }
                Err(e) => {
                    let kind = crate::sql::classify_pg_error(&e);
                    let (code, default_msg) = kind.as_batch_detail();
                    // Derive field name from the actual constraint name
                    // (opus critical #3).
                    let field = if matches!(&kind, PgErrorKind::ForeignKeyViolation { .. }) {
                        let cname = kind.constraint();
                        Some(fk_field_from_constraint(cname).to_string())
                    } else {
                        None
                    };
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field,
                        code: code.into(),
                        message: default_msg.unwrap_or("row failed validation").into(),
                    });
                }
            }
        }

        if !details.is_empty() {
            let _ = tx.rollback().await;
            return BatchResp::PartialFailure(Json(BatchErrorEnvelope::partial_failure(
                total, details,
            )));
        }

        // Emit one audit row covering the whole batch on success.
        if let Err(e) =
            crate::audit::emit(&mut tx, &principal, "campaign.batch_upsert", "campaign",
                "", None::<&()>).await
        {
            tracing::warn!(error = %e, "audit emit failed on campaign.batch_upsert");
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
