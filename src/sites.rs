//! Sites — `/v1/projects/{projectId}/sites`.
//!
//! Phase 3.12. CRUD plus `:upsertByUrl` (the natural-key upsert
//! `API.md` § 3.7 calls out — keyed on `(project_id, url)`).
//! Aliases are stored as a text[] but uniqueness across
//! `url + aliases` is application-layer for v0 (a partial
//! expression-index can land later if write throughput needs it).

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

pub struct SitesApi;

const CURSOR_KIND: &str = "sites";

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateSiteRequest {
    pub external_id: Option<String>,
    pub channel_id: Option<i64>,
    pub name: String,
    pub url: String,
    pub aliases: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpsertSiteByUrlRequest {
    pub url: String,
    pub name: String,
    pub aliases: Option<Vec<String>>,
    pub channel_id: Option<i64>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateSiteRequest {
    pub name: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub channel_id: Option<i64>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Site {
    pub id: i64,
    pub external_id: Option<String>,
    pub channel_id: Option<i64>,
    pub name: String,
    pub url: String,
    pub aliases: Vec<String>,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct SiteList {
    pub items: Vec<Site>,
    pub next_cursor: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertSiteRow {
    pub external_id: String,
    pub channel_id: Option<i64>,
    pub name: String,
    pub url: String,
    pub aliases: Option<Vec<String>>,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertSitesRequest {
    pub items: Vec<BatchUpsertSiteRow>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct BatchUpsertSitesResult {
    pub items: Vec<Site>,
}

const COLS: &str = r#"
    id, external_id, channel_id, name, url, aliases, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<Site>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum UpsertResp {
    /// Existing row matched by URL.
    #[oai(status = 200)]
    Existing(Json<Site>),
    /// Newly inserted row.
    #[oai(status = 201)]
    Created(Json<Site>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum ListResp {
    #[oai(status = 200)]
    Ok(Json<SiteList>),
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
    Ok(Json<Site>),
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
    Ok(Json<Site>),
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
    Ok(Json<BatchUpsertSitesResult>),
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

#[OpenApi(tag = "ApiTags::Sites")]
impl SitesApi {
    #[oai(
        path = "/v1/projects/:project_id/sites",
        method = "post",
        operation_id = "createSite"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateSiteRequest>,
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
            "INSERT INTO knievel.sites
                 (org_id, project_id, channel_id, external_id, name, url,
                  aliases, is_active)
             VALUES ($1, $2, $3, $4, $5, $6,
                     COALESCE($7, '{{}}'::text[]),
                     COALESCE($8, true))
             RETURNING {COLS}"
        );
        let row: Result<Site, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.channel_id)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(&req.url)
            .bind(req.aliases.as_deref())
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;
        match row {
            Ok(s) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(s)),
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
                        "external_id or url is already taken in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "site insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    /// Natural-key upsert. Returns the existing row (200) when a
    /// site with the same URL exists; otherwise creates (201). Per
    /// `API.md` § 3.7: `:upsertByUrl` is the canonical entry point
    /// for URL-driven flows.
    #[oai(
        path = "/v1/projects/:project_id/sites:upsertByUrl",
        method = "post",
        operation_id = "upsertSiteByUrl"
    )]
    async fn upsert_by_url(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<UpsertSiteByUrlRequest>,
    ) -> UpsertResp {
        let principal = auth.0;
        let pj = project_id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return UpsertResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Editor).await {
            Ok(t) => t,
            Err(e) => return forbid(UpsertResp::Forbidden, e),
        };

        let select_sql = format!("SELECT {COLS} FROM knievel.sites WHERE url = $1");
        let existing: Option<Site> = match sqlx::query_as::<_, Site>(&select_sql)
            .bind(&req.url)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "upsert select failed");
                return UpsertResp::Internal(Json(err("db_error", "select failed")));
            }
        };
        if let Some(s) = existing {
            return UpsertResp::Existing(Json(s));
        }

        let insert_sql = format!(
            "INSERT INTO knievel.sites
                 (org_id, project_id, channel_id, name, url, aliases)
             VALUES ($1, $2, $3, $4, $5, COALESCE($6, '{{}}'::text[]))
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Site>(&insert_sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.channel_id)
            .bind(&req.name)
            .bind(&req.url)
            .bind(req.aliases.as_deref())
            .fetch_one(&mut *tx)
            .await
        {
            Ok(s) => match tx.commit().await {
                Ok(()) => UpsertResp::Created(Json(s)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpsertResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Err(e) => {
                tracing::error!(error = %e, "upsert insert failed");
                UpsertResp::Internal(Json(err("db_error", "insert failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/sites",
        method = "get",
        operation_id = "listSites"
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
            None => format!("SELECT {COLS} FROM knievel.sites ORDER BY id DESC LIMIT $1"),
            Some(_) => {
                format!("SELECT {COLS} FROM knievel.sites WHERE id < $2 ORDER BY id DESC LIMIT $1")
            }
        };
        let q = sqlx::query_as::<_, Site>(&sql).bind(resolved.bumped_limit);
        let q = match resolved.after_id {
            Some(after) => q.bind(after),
            None => q,
        };
        match q.fetch_all(&mut *tx).await {
            Ok(mut rows) => {
                let next_cursor =
                    crate::pagination::next_cursor(&rows, &resolved, CURSOR_KIND, |r| r.id);
                rows.truncate(resolved.effective_limit as usize);
                ListResp::Ok(Json(SiteList {
                    items: rows,
                    next_cursor,
                }))
            }
            Err(e) => {
                tracing::error!(error = %e, "list sites failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/sites/:id",
        method = "get",
        operation_id = "getSite"
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
        let sql = format!("SELECT {COLS} FROM knievel.sites WHERE id = $1");
        match sqlx::query_as::<_, Site>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(s)) => GetResp::Ok(Json(s)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "site not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get site failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/sites/:id",
        method = "patch",
        operation_id = "updateSite"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateSiteRequest>,
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
            "UPDATE knievel.sites SET
                 name = COALESCE($2, name),
                 aliases = COALESCE($3, aliases),
                 channel_id = COALESCE($4, channel_id),
                 is_active = COALESCE($5, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Site>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.aliases.as_deref())
            .bind(req.channel_id)
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(s)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(s)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "site not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update site failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/sites:batchUpsert",
        method = "post",
        operation_id = "batchUpsertSites"
    )]
    async fn batch_upsert(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<BatchUpsertSitesRequest>,
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
        let mut out: Vec<Site> = Vec::with_capacity(total);
        let mut details: Vec<BatchErrorDetail> = Vec::new();
        let sql = format!(
            "INSERT INTO knievel.sites
                 (org_id, project_id, channel_id, external_id, name, url,
                  aliases, is_active)
             VALUES ($1, $2, $3, $4, $5, $6,
                     COALESCE($7, '{{}}'::text[]),
                     COALESCE($8, true))
             ON CONFLICT (project_id, external_id) DO UPDATE SET
                 channel_id = EXCLUDED.channel_id,
                 name = EXCLUDED.name,
                 url = EXCLUDED.url,
                 aliases = EXCLUDED.aliases,
                 is_active = COALESCE(EXCLUDED.is_active, knievel.sites.is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             RETURNING {COLS}"
        );
        for (idx, row) in req.items.iter().enumerate() {
            let r: Result<Site, _> = sqlx::query_as(&sql)
                .bind(&principal.org_id)
                .bind(&pj)
                .bind(row.channel_id)
                .bind(&row.external_id)
                .bind(&row.name)
                .bind(&row.url)
                .bind(row.aliases.as_deref())
                .bind(row.is_active)
                .fetch_one(&mut *tx)
                .await;
            match r {
                Ok(s) => out.push(s),
                Err(e) => {
                    let m = format!("{e}");
                    let (code, msg) = classify_pg_error(&m);
                    details.push(BatchErrorDetail {
                        index: idx as i32,
                        field: None,
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
            Ok(()) => BatchResp::Ok(Json(BatchUpsertSitesResult { items: out })),
            Err(e) => {
                tracing::error!(error = %e, "batch upsert sites commit failed");
                BatchResp::Internal(Json(err("db_error", "commit failed")))
            }
        }
    }
}
