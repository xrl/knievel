//! Advertisers — `POST/GET/PATCH /v1/projects/{projectId}/advertisers[/{id}]`.
//!
//! Phase 3.8. First project-scoped CRUD resource. Validates the
//! handler shape that 3.9–3.13 reuse (the `crud_contract!` macro
//! lands as the extraction in 3.9 once the duplication is real).
//!
//! Spec refs: `API.md` § 3.1, `AUTH.md` "Project resources",
//! `REQUIREMENTS.md` § 5.

use poem::web::Data;
use poem_openapi::{param::Path, payload::Json, ApiResponse, Object, OpenApi};

use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct AdvertisersApi;

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateAdvertiserRequest {
    pub external_id: Option<String>,
    pub name: String,
    pub is_active: Option<bool>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateAdvertiserRequest {
    pub name: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct Advertiser {
    pub id: i64,
    pub external_id: Option<String>,
    pub name: String,
    pub is_active: bool,
    pub etag: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct AdvertiserList {
    pub items: Vec<Advertiser>,
    pub next_cursor: Option<String>,
}

const COLS: &str = r#"
    id, external_id, name, is_active, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

#[derive(ApiResponse)]
pub enum CreateAdvertiserResp {
    #[oai(status = 201)]
    Created(Json<Advertiser>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum ListAdvertisersResp {
    #[oai(status = 200)]
    Ok(Json<AdvertiserList>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum GetAdvertiserResp {
    #[oai(status = 200)]
    Ok(Json<Advertiser>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 404)]
    NotFound(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}

#[derive(ApiResponse)]
pub enum UpdateAdvertiserResp {
    #[oai(status = 200)]
    Ok(Json<Advertiser>),
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

fn forbidden_create(e: AuthzError) -> CreateAdvertiserResp {
    CreateAdvertiserResp::Forbidden(Json(err(e.code(), e.message())))
}
fn forbidden_list(e: AuthzError) -> ListAdvertisersResp {
    ListAdvertisersResp::Forbidden(Json(err(e.code(), e.message())))
}
fn forbidden_get(e: AuthzError) -> GetAdvertiserResp {
    GetAdvertiserResp::Forbidden(Json(err(e.code(), e.message())))
}
fn forbidden_update(e: AuthzError) -> UpdateAdvertiserResp {
    UpdateAdvertiserResp::Forbidden(Json(err(e.code(), e.message())))
}

#[OpenApi]
impl AdvertisersApi {
    #[oai(
        path = "/v1/projects/:project_id/advertisers",
        method = "post",
        operation_id = "createAdvertiser"
    )]
    async fn create_advertiser(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        body: Json<CreateAdvertiserRequest>,
    ) -> CreateAdvertiserResp {
        let principal = auth.0;
        let path_project_id = project_id.0;
        let req = body.0;

        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return CreateAdvertiserResp::Internal(Json(err("no_db", "no database configured")))
            }
        };

        let mut tx = match open_project_tx(pool, &principal, &path_project_id, Role::Editor).await {
            Ok(tx) => tx,
            Err(e) => return forbidden_create(e),
        };

        let sql = format!(
            "INSERT INTO knievel.advertisers (org_id, project_id, external_id, name, is_active)
             VALUES ($1, $2, $3, $4, COALESCE($5, true))
             RETURNING {COLS}"
        );
        let row: Result<Advertiser, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&path_project_id)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(req.is_active)
            .fetch_one(&mut *tx)
            .await;

        match row {
            Ok(adv) => match tx.commit().await {
                Ok(()) => CreateAdvertiserResp::Created(Json(adv)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    CreateAdvertiserResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Err(e) => {
                let m = format!("{e}");
                if m.contains("duplicate key") || m.contains("unique constraint") {
                    CreateAdvertiserResp::Conflict(Json(err(
                        "external_id_conflict",
                        "external_id is already taken in this project",
                    )))
                } else {
                    tracing::error!(error = %e, "advertiser insert failed");
                    CreateAdvertiserResp::Internal(Json(err("db_error", "insert failed")))
                }
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/advertisers",
        method = "get",
        operation_id = "listAdvertisers"
    )]
    async fn list_advertisers(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
    ) -> ListAdvertisersResp {
        let principal = auth.0;
        let path_project_id = project_id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return ListAdvertisersResp::Internal(Json(err("no_db", "no database configured")))
            }
        };
        let mut tx = match open_project_tx(pool, &principal, &path_project_id, Role::Reader).await {
            Ok(tx) => tx,
            Err(e) => return forbidden_list(e),
        };
        let sql = format!("SELECT {COLS} FROM knievel.advertisers ORDER BY id DESC LIMIT 500");
        match sqlx::query_as::<_, Advertiser>(&sql)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(items) => ListAdvertisersResp::Ok(Json(AdvertiserList {
                items,
                next_cursor: None,
            })),
            Err(e) => {
                tracing::error!(error = %e, "list advertisers failed");
                ListAdvertisersResp::Internal(Json(err("db_error", "list failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/advertisers/:id",
        method = "get",
        operation_id = "getAdvertiser"
    )]
    async fn get_advertiser(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
    ) -> GetAdvertiserResp {
        let principal = auth.0;
        let path_project_id = project_id.0;
        let id = id.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return GetAdvertiserResp::Internal(Json(err("no_db", "no database configured")))
            }
        };
        let mut tx = match open_project_tx(pool, &principal, &path_project_id, Role::Reader).await {
            Ok(tx) => tx,
            Err(e) => return forbidden_get(e),
        };
        let sql = format!("SELECT {COLS} FROM knievel.advertisers WHERE id = $1");
        match sqlx::query_as::<_, Advertiser>(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(adv)) => GetAdvertiserResp::Ok(Json(adv)),
            Ok(None) => GetAdvertiserResp::NotFound(Json(err("not_found", "advertiser not found"))),
            Err(e) => {
                tracing::error!(error = %e, "get advertiser failed");
                GetAdvertiserResp::Internal(Json(err("db_error", "select failed")))
            }
        }
    }

    #[oai(
        path = "/v1/projects/:project_id/advertisers/:id",
        method = "patch",
        operation_id = "updateAdvertiser"
    )]
    async fn update_advertiser(
        &self,
        auth: BearerAuth,
        state: Data<&AppState>,
        project_id: Path<String>,
        id: Path<i64>,
        body: Json<UpdateAdvertiserRequest>,
    ) -> UpdateAdvertiserResp {
        let principal = auth.0;
        let path_project_id = project_id.0;
        let id = id.0;
        let req = body.0;
        let pool = match state.0.db.as_ref() {
            Some(p) => p,
            None => {
                return UpdateAdvertiserResp::Internal(Json(err("no_db", "no database configured")))
            }
        };
        let mut tx = match open_project_tx(pool, &principal, &path_project_id, Role::Editor).await {
            Ok(tx) => tx,
            Err(e) => return forbidden_update(e),
        };
        // COALESCE makes per-field null-means-unchanged. updated_at
        // and etag bump on every PATCH so callers' If-Match (which
        // lands with the etag-aware update path) sees fresh values.
        let sql = format!(
            "UPDATE knievel.advertisers
             SET name = COALESCE($2, name),
                 is_active = COALESCE($3, is_active),
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, Advertiser>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.is_active)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(adv)) => match tx.commit().await {
                Ok(()) => UpdateAdvertiserResp::Ok(Json(adv)),
                Err(e) => {
                    tracing::error!(error = %e, "commit failed");
                    UpdateAdvertiserResp::Internal(Json(err("db_error", "commit failed")))
                }
            },
            Ok(None) => {
                UpdateAdvertiserResp::NotFound(Json(err("not_found", "advertiser not found")))
            }
            Err(e) => {
                tracing::error!(error = %e, "update advertiser failed");
                UpdateAdvertiserResp::Internal(Json(err("db_error", "update failed")))
            }
        }
    }
}
