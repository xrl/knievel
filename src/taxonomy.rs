//! Read-only taxonomy — channels, priorities, ad_types.
//!
//! Phase 3.13. Per `API.md` § 3.9 these are read-only via the
//! API in v0; rows are seeded at project creation by
//! `seed_default_taxonomy` (called from the `create_project`
//! handler). Write endpoints are post-v0
//! (`REQUIREMENTS.md` § 11 roadmap).
//!
//! Spec refs: `API.md` § 3.9.

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct TaxonomyApi;

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Priority {
    pub id: i64,
    pub name: String,
    pub tier: i32,
    pub created_at: String,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AdType {
    pub id: i64,
    pub name: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct ChannelList {
    pub items: Vec<Channel>,
    pub next_cursor: Option<String>,
}
#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct PriorityList {
    pub items: Vec<Priority>,
    pub next_cursor: Option<String>,
}
#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct AdTypeList {
    pub items: Vec<AdType>,
    pub next_cursor: Option<String>,
}

#[derive(ApiResponse)]
pub enum ChannelsResp {
    #[oai(status = 200)]
    Ok(Json<ChannelList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum ChannelResp {
    #[oai(status = 200)]
    Ok(Json<Channel>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum PrioritiesResp {
    #[oai(status = 200)]
    Ok(Json<PriorityList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum PriorityResp {
    #[oai(status = 200)]
    Ok(Json<Priority>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum AdTypesResp {
    #[oai(status = 200)]
    Ok(Json<AdTypeList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum AdTypeResp {
    #[oai(status = 200)]
    Ok(Json<AdType>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

const TS: &str =
    "to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at";

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

#[OpenApi(tag = "ApiTags::Taxonomy")]
impl TaxonomyApi {
    #[oai(
        path = "/v1/projects/:project_id/channels",
        method = "get",
        operation_id = "listChannels"
    )]
    async fn list_channels(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
    ) -> ChannelsResp {
        let principal = auth.0;
        let pj = project_id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ChannelsResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(ChannelsResp::Forbidden, e),
        };
        let sql = format!("SELECT id, name, {TS} FROM knievel.channels ORDER BY id");
        match sqlx::query_as::<_, Channel>(&sql).fetch_all(&mut *tx).await {
            Ok(items) => ChannelsResp::Ok(Json(ChannelList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list channels failed");
                ChannelsResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/channels/:id",
        method = "get",
        operation_id = "getChannel"
    )]
    async fn get_channel(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
    ) -> ChannelResp {
        let principal = auth.0;
        let pj = project_id.0;
        let id = id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ChannelResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(ChannelResp::Forbidden, e),
        };
        let sql = format!("SELECT id, name, {TS} FROM knievel.channels WHERE id = $1");
        match sqlx::query_as::<_, Channel>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(c)) => ChannelResp::Ok(Json(c)),
            Ok(None) => ChannelResp::NotFound(Json(err("not_found", "channel not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get channel failed");
                ChannelResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/priorities",
        method = "get",
        operation_id = "listPriorities"
    )]
    async fn list_priorities(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
    ) -> PrioritiesResp {
        let principal = auth.0;
        let pj = project_id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return PrioritiesResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(PrioritiesResp::Forbidden, e),
        };
        // Ordered by tier per API.md: lower tier wins.
        let sql = format!("SELECT id, name, tier, {TS} FROM knievel.priorities ORDER BY tier");
        match sqlx::query_as::<_, Priority>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => PrioritiesResp::Ok(Json(PriorityList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list priorities failed");
                PrioritiesResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/priorities/:id",
        method = "get",
        operation_id = "getPriority"
    )]
    async fn get_priority(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
    ) -> PriorityResp {
        let principal = auth.0;
        let pj = project_id.0;
        let id = id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return PriorityResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(PriorityResp::Forbidden, e),
        };
        let sql = format!("SELECT id, name, tier, {TS} FROM knievel.priorities WHERE id = $1");
        match sqlx::query_as::<_, Priority>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(p)) => PriorityResp::Ok(Json(p)),
            Ok(None) => PriorityResp::NotFound(Json(err("not_found", "priority not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get priority failed");
                PriorityResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ad-types",
        method = "get",
        operation_id = "listAdTypes"
    )]
    async fn list_ad_types(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
    ) -> AdTypesResp {
        let principal = auth.0;
        let pj = project_id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return AdTypesResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(AdTypesResp::Forbidden, e),
        };
        let sql = format!("SELECT id, name, width, height, {TS} FROM knievel.ad_types ORDER BY id");
        match sqlx::query_as::<_, AdType>(&sql).fetch_all(&mut *tx).await {
            Ok(items) => AdTypesResp::Ok(Json(AdTypeList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list ad_types failed");
                AdTypesResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/ad-types/:id",
        method = "get",
        operation_id = "getAdType"
    )]
    async fn get_ad_type(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
    ) -> AdTypeResp {
        let principal = auth.0;
        let pj = project_id.0;
        let id = id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return AdTypeResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match open_project_tx(pool, &principal, &pj, Role::Reader).await {
            Ok(t) => t,
            Err(e) => return forbid(AdTypeResp::Forbidden, e),
        };
        let sql =
            format!("SELECT id, name, width, height, {TS} FROM knievel.ad_types WHERE id = $1");
        match sqlx::query_as::<_, AdType>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(a)) => AdTypeResp::Ok(Json(a)),
            Ok(None) => AdTypeResp::NotFound(Json(err("not_found", "ad_type not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get ad_type failed");
                AdTypeResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }
}

/// Insert default channels / priorities / ad_types for a freshly
/// created project. Caller has already begun a tenant-bound
/// transaction with `org_id` and `project_id` set, so RLS lets
/// the inserts through. The seed shape is stable across knievel
/// versions; the post-v0 write endpoints (`REQUIREMENTS.md` § 11
/// roadmap) will replace this for projects that need custom
/// taxonomies.
pub async fn seed_default_taxonomy(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: &str,
    project_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO knievel.channels (org_id, project_id, name)
         VALUES ($1, $2, 'Web'), ($1, $2, 'Mobile'), ($1, $2, 'Email')",
    )
    .bind(org_id)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO knievel.priorities (org_id, project_id, name, tier) VALUES
         ($1, $2, 'House', 1), ($1, $2, 'Standard', 2), ($1, $2, 'Backfill', 3)",
    )
    .bind(org_id)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO knievel.ad_types (org_id, project_id, name, width, height) VALUES
         ($1, $2, 'Medium Rectangle', 300, 250),
         ($1, $2, 'Leaderboard',      728,  90),
         ($1, $2, 'Mobile Banner',    320,  50),
         ($1, $2, 'Large Leaderboard',970,  90)",
    )
    .bind(org_id)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
