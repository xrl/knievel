//! Ad Library — `/v1/orgs/{orgId}/ad-library/items`.
//!
//! Phase 3.28. Org-scoped catalog of reusable creatives. Project
//! Ads reference items via `ads.ad_library_item_id` (column
//! reserved in `migrations/0006_demand.sql` so this is purely
//! additive). Library references resolve through the in-memory
//! snapshot at decision time — updating an item updates all
//! referencing ads after the next snapshot swap (`API.md` §
//! 2.4).
//!
//! Spec refs: `API.md` § 2.4, `REQUIREMENTS.md` § 5.1.

#![allow(clippy::large_enum_variant)]

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::db;
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct AdLibraryApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateAdLibraryItemRequest {
    pub external_id: Option<String>,
    pub name: String,
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

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateAdLibraryItemRequest {
    pub name: Option<String>,
    pub image_url: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub alt: Option<String>,
    pub body: Option<String>,
    pub click_through_url: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct AdLibraryItem {
    pub id: String,
    pub external_id: Option<String>,
    pub name: String,
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
pub struct AdLibraryItemList {
    pub items: Vec<AdLibraryItem>,
    pub next_cursor: Option<String>,
}

const COLS: &str = r#"
    id, external_id, name, kind, image_url, width, height, alt, body,
    template_id, values, click_through_url, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<AdLibraryItem>),
    #[oai(status = 400)]
    BadRequest(Json<ErrorEnvelope>),
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
    Ok(Json<AdLibraryItemList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum GetResp {
    #[oai(status = 200)]
    Ok(Json<AdLibraryItem>),
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
    Ok(Json<AdLibraryItem>),
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

fn org_authz<R, F: FnOnce(Json<ErrorEnvelope>) -> R>(
    f: F,
    principal: &crate::auth::Principal,
    org_id: &str,
    min_role: Role,
) -> Option<R> {
    if principal.org_id != org_id {
        return Some(f(Json(err(
            "wrong_tenant",
            "principal's org_id does not match the path",
        ))));
    }
    if !principal.has_role_at_least(min_role) {
        return Some(f(Json(err(
            "role_insufficient",
            "principal's role is below the endpoint minimum",
        ))));
    }
    None
}

#[OpenApi(tag = "ApiTags::AdLibrary")]
impl AdLibraryApi {
    #[oai(
        path = "/v1/orgs/:org_id/ad-library/items",
        method = "post",
        operation_id = "createAdLibraryItem"
    )]
    async fn create(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        body: Json<CreateAdLibraryItemRequest>,
    ) -> CreateResp {
        let principal = auth.0;
        let path_org = org_id.0;
        if !matches!(body.0.kind.as_str(), "image" | "html" | "native") {
            return CreateResp::BadRequest(Json(err(
                "validation_failed",
                "kind must be one of: image, html, native",
            )));
        }
        if let Some(r) = org_authz(CreateResp::Forbidden, &principal, &path_org, Role::Editor) {
            return r;
        }
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return CreateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match db::begin_bound(pool, &path_org, None).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "begin_bound failed");
                return CreateResp::Internal(Json(err("db_error", "could not begin tx")));
            }
        };

        let new_id = format!("ali_{}", random_hex(12));
        let req = body.0;
        let sql = format!(
            "INSERT INTO knievel.ad_library_items
                 (id, org_id, external_id, name, kind, image_url, width, height,
                  alt, body, template_id, values, click_through_url, is_active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                     COALESCE($14, true))
             RETURNING {COLS}"
        );
        let r: Result<AdLibraryItem, _> = sqlx::query_as(&sql)
            .bind(&new_id)
            .bind(&path_org)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
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
        match r {
            Ok(item) => match tx.commit().await {
                Ok(()) => CreateResp::Created(Json(item)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    CreateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Err(e) => {
                let kind = crate::sql::classify_pg_error(&e);
                if kind.is_external_id_conflict() {
                    CreateResp::Conflict(Json(err(
                        "external_id_conflict",
                        "external_id already taken in this org",
                    )))
                } else {
                    tracing::error!(error = %e, kind = ?kind, "ad_library insert failed");
                    CreateResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/orgs/:org_id/ad-library/items",
        method = "get",
        operation_id = "listAdLibraryItems"
    )]
    async fn list(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
    ) -> ListResp {
        let principal = auth.0;
        let path_org = org_id.0;
        if let Some(r) = org_authz(ListResp::Forbidden, &principal, &path_org, Role::Reader) {
            return r;
        }
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return ListResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match db::begin_bound(pool, &path_org, None).await {
            Ok(t) => t,
            Err(_) => return ListResp::Internal(Json(err("db_error", "begin tx failed"))),
        };
        let sql = format!(
            "SELECT {COLS} FROM knievel.ad_library_items ORDER BY created_at DESC LIMIT 500"
        );
        match sqlx::query_as::<_, AdLibraryItem>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => ListResp::Ok(Json(AdLibraryItemList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list ad_library failed");
                ListResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/orgs/:org_id/ad-library/items/:item_id",
        method = "get",
        operation_id = "getAdLibraryItem"
    )]
    async fn get(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        item_id: Path<String>,
    ) -> GetResp {
        let principal = auth.0;
        let path_org = org_id.0;
        let id = item_id.0;
        if let Some(r) = org_authz(GetResp::Forbidden, &principal, &path_org, Role::Reader) {
            return r;
        }
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return GetResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match db::begin_bound(pool, &path_org, None).await {
            Ok(t) => t,
            Err(_) => return GetResp::Internal(Json(err("db_error", "begin tx failed"))),
        };
        let sql = format!("SELECT {COLS} FROM knievel.ad_library_items WHERE id = $1");
        match sqlx::query_as::<_, AdLibraryItem>(&sql)
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(item)) => GetResp::Ok(Json(item)),
            Ok(None) => GetResp::NotFound(Json(err("not_found", "ad library item not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get ad_library failed");
                GetResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/orgs/:org_id/ad-library/items/:item_id",
        method = "patch",
        operation_id = "updateAdLibraryItem"
    )]
    async fn update(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        org_id: Path<String>,
        item_id: Path<String>,
        body: Json<UpdateAdLibraryItemRequest>,
    ) -> UpdateResp {
        let principal = auth.0;
        let path_org = org_id.0;
        let id = item_id.0;
        let req = body.0;
        if let Some(r) = org_authz(UpdateResp::Forbidden, &principal, &path_org, Role::Editor) {
            return r;
        }
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => return UpdateResp::Internal(Json(err("no_db", "no database configured"))),
        };
        let mut tx = match db::begin_bound(pool, &path_org, None).await {
            Ok(t) => t,
            Err(_) => return UpdateResp::Internal(Json(err("db_error", "begin tx failed"))),
        };
        let sql = format!(
            "UPDATE knievel.ad_library_items SET
                 name = COALESCE($2, name),
                 image_url = COALESCE($3, image_url),
                 width = COALESCE($4, width),
                 height = COALESCE($5, height),
                 alt = COALESCE($6, alt),
                 body = COALESCE($7, body),
                 click_through_url = COALESCE($8, click_through_url),
                 is_active = COALESCE($9, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, AdLibraryItem>(&sql)
            .bind(&id)
            .bind(req.name.as_deref())
            .bind(req.image_url.as_deref())
            .bind(req.width)
            .bind(req.height)
            .bind(req.alt.as_deref())
            .bind(req.body.as_deref())
            .bind(req.click_through_url.as_deref())
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(item)) => match tx.commit().await {
                Ok(()) => UpdateResp::Ok(Json(item)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => UpdateResp::NotFound(Json(err("not_found", "ad library item not found"))),
            Err(e) => {
                tracing::error!(error = %e, "update ad_library failed");
                UpdateResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }
}

fn random_hex(n: usize) -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = vec![0u8; n / 2 + 1];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .take(n / 2)
        .map(|b| format!("{b:02x}"))
        .collect()
}
