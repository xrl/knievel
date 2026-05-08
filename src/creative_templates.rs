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
use poem_openapi::{
    param::{Path, Query},
    payload::Json,
    ApiResponse, Object, OpenApi,
};

use crate::api_tags::ApiTags;
use crate::auth::security::BearerAuth;
use crate::auth::Role;
use crate::handlers::{open_project_tx, AuthzError};
use crate::orgs::{ErrorBody, ErrorEnvelope};
use crate::state::AppState;

pub struct CreativeTemplatesApi;

const CURSOR_KIND: &str = "creative-templates";

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct CreateCreativeTemplateRequest {
    pub external_id: Option<String>,
    pub name: String,
    /// Arbitrary JSON Schema document; not parsed by knievel.
    pub schema: serde_json::Value,
    /// Optional renderer source (today: Liquid). When present,
    /// `template_engine` MUST also be present and equal to
    /// `"liquid"`. Parsed at write time; malformed source returns
    /// `422 / template_parse_error`. Templates without a source
    /// are input-validation-only — only `native` creatives can
    /// reference them.
    pub template: Option<String>,
    pub template_engine: Option<String>,
}

#[derive(Object, serde::Serialize, serde::Deserialize)]
pub struct UpdateCreativeTemplateRequest {
    pub name: Option<String>,
    pub schema: Option<serde_json::Value>,
    pub template: Option<String>,
    pub template_engine: Option<String>,
}

#[derive(Object, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct CreativeTemplate {
    pub id: i64,
    pub external_id: Option<String>,
    pub name: String,
    pub schema: serde_json::Value,
    pub template: Option<String>,
    pub template_engine: Option<String>,
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
    id, external_id, name, schema, template, template_engine, version, etag,
    to_char(created_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS created_at,
    to_char(updated_at AT TIME ZONE 'UTC',
            'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS updated_at
"#;

/// Validate `template` + `template_engine` per `API.md` § 3.6
/// rules: when `template` is present, `template_engine` must be
/// `"liquid"`, and the source must parse. Returns the canonical
/// engine string on success, or an error envelope ready to ship.
pub(crate) fn validate_template_pair(
    template: Option<&str>,
    template_engine: Option<&str>,
) -> Result<Option<String>, ErrorEnvelope> {
    match (template, template_engine) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(err(
            "template_engine_without_template",
            "template_engine must be omitted when template is unset",
        )),
        (Some(_), None) => Err(err(
            "template_engine_required",
            "template_engine must be 'liquid' when template is set",
        )),
        (Some(_), Some(eng)) if eng != "liquid" => Err(err(
            "template_engine_unsupported",
            "only template_engine='liquid' is supported in v0",
        )),
        (Some(src), Some(eng)) => {
            // Parse-on-write so a bad source can't sneak through to
            // decision time. We don't render here — that happens
            // per-decision, post-snapshot.
            let parser = liquid::ParserBuilder::with_stdlib()
                .build()
                .expect("liquid stdlib parser construction");
            if let Err(e) = parser.parse(src) {
                let msg = format!("template parse error: {e}");
                return Err(err("template_parse_error", &msg));
            }
            Ok(Some(eng.to_string()))
        }
    }
}

#[derive(ApiResponse)]
pub enum CreateResp {
    #[oai(status = 201)]
    Created(Json<CreativeTemplate>),
    #[oai(status = 403)]
    Forbidden(Json<ErrorEnvelope>),
    #[oai(status = 409)]
    Conflict(Json<ErrorEnvelope>),
    /// Liquid parse failure or `template`/`template_engine`
    /// validation per `API.md` § 3.6.
    #[oai(status = 422)]
    Unprocessable(Json<ErrorEnvelope>),
    #[oai(status = 500)]
    Internal(Json<ErrorEnvelope>),
}
#[derive(ApiResponse)]
pub enum ListResp {
    #[oai(status = 200)]
    Ok(Json<CreativeTemplateList>),
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
    /// Liquid parse failure or `template`/`template_engine`
    /// validation per `API.md` § 3.6.
    #[oai(status = 422)]
    Unprocessable(Json<ErrorEnvelope>),
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

#[OpenApi(tag = "ApiTags::CreativeTemplates")]
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
        let engine =
            match validate_template_pair(req.template.as_deref(), req.template_engine.as_deref()) {
                Ok(e) => e,
                Err(env) => return CreateResp::Unprocessable(Json(env)),
            };
        let sql = format!(
            "INSERT INTO knievel.creative_templates
                 (org_id, project_id, external_id, name, schema,
                  template, template_engine)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING {COLS}"
        );
        let row: Result<CreativeTemplate, _> = sqlx::query_as(&sql)
            .bind(&principal.org_id)
            .bind(&pj)
            .bind(req.external_id.as_deref())
            .bind(&req.name)
            .bind(&req.schema)
            .bind(req.template.as_deref())
            .bind(engine.as_deref())
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
                let kind = crate::sql::classify_pg_error(&e);
                // creative_templates has two unique keys —
                // (project_id, external_id) and (project_id, name).
                // The 409 surface treats both as `external_id_conflict`
                // for v0; `is_unique_violation()` catches both.
                // Caller-side disambiguation can come later via the
                // constraint-name suffix.
                if kind.is_unique_violation() {
                    CreateResp::Conflict(Json(err(
                        "external_id_conflict",
                        "external_id or name is already taken in this project",
                    )))
                } else {
                    tracing::error!(error = %e, kind = ?kind, "creative_template insert failed");
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
            None => format!(
                "SELECT {COLS} FROM knievel.creative_templates ORDER BY id DESC LIMIT $1"
            ),
            Some(_) => format!(
                "SELECT {COLS} FROM knievel.creative_templates WHERE id < $2 ORDER BY id DESC LIMIT $1"
            ),
        };
        let q = sqlx::query_as::<_, CreativeTemplate>(&sql).bind(resolved.bumped_limit);
        let q = match resolved.after_id {
            Some(after) => q.bind(after),
            None => q,
        };
        match q.fetch_all(&mut *tx).await {
            Ok(mut rows) => {
                let next_cursor =
                    crate::pagination::next_cursor(&rows, &resolved, CURSOR_KIND, |r| r.id);
                rows.truncate(resolved.effective_limit as usize);
                ListResp::Ok(Json(CreativeTemplateList {
                    items: rows,
                    next_cursor,
                }))
            }
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

    /// PATCH bumps `version` only when `schema`, `template`, or
    /// `template_engine` *actually change value* (opus O19 fix).
    ///
    /// The previous implementation bumped version on mere field
    /// *presence* in the request, so a PATCH sending the same
    /// schema the row already had would increment the version
    /// counter. We now SELECT the current row first and compare
    /// each versioned field before deciding whether to bump.
    ///
    /// `name` is not versioned — only the rendering contract
    /// (`schema`, `template`, `template_engine`) is.
    ///
    /// Per `API.md` § 3.6, schema changes are versioned but do
    /// not retroactively re-validate existing creatives.
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
        // Validate the (template, template_engine) pair when
        // either is provided. PATCH semantics: omitted fields keep
        // existing values; explicitly setting `template = null`
        // (with `template_engine = null`) clears the renderer.
        let engine_to_set = match (req.template.as_deref(), req.template_engine.as_deref()) {
            (None, None) => None, // no change
            _ => match validate_template_pair(
                req.template.as_deref(),
                req.template_engine.as_deref(),
            ) {
                Ok(e) => Some(e),
                Err(env) => return UpdateResp::Unprocessable(Json(env)),
            },
        };
        // SELECT the existing row so we can compare values.
        // The transaction is already tenant-bound; this SELECT
        // runs under the same RLS context as the subsequent UPDATE.
        let existing_sql = format!("SELECT {COLS} FROM knievel.creative_templates WHERE id = $1");
        let existing: CreativeTemplate = match sqlx::query_as::<_, CreativeTemplate>(&existing_sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => {
                return UpdateResp::NotFound(Json(err("not_found", "creative_template not found")))
            }
            Err(e) => {
                tracing::error!(error = %e, "select for update creative_template failed");
                return UpdateResp::Internal(Json(err("db_error", "select failed")));
            }
        };
        // Determine the effective new values for versioned fields.
        // `None` in the request means "keep existing" for each field.
        let new_schema = req.schema.as_ref().unwrap_or(&existing.schema);
        let new_template: Option<&str> = if req.template.is_some() || req.template_engine.is_some()
        {
            // template/template_engine were supplied — effective
            // value is the newly-validated engine source (which may
            // be None if the caller sent null to clear).
            req.template.as_deref()
        } else {
            existing.template.as_deref()
        };
        let new_engine: Option<&str> = if req.template.is_some() || req.template_engine.is_some() {
            engine_to_set.as_ref().and_then(|e| e.as_deref())
        } else {
            existing.template_engine.as_deref()
        };
        // Version bumps only when a versioned field *value* changes.
        let schema_changed = new_schema != &existing.schema;
        let template_changed = new_template != existing.template.as_deref();
        let engine_changed = new_engine != existing.template_engine.as_deref();
        let bump = schema_changed || template_changed || engine_changed;
        // Whether to overwrite the template/engine columns at all.
        // We always write if either was supplied in the request.
        let replace_template = req.template.is_some() || req.template_engine.is_some();
        let sql = format!(
            "UPDATE knievel.creative_templates
             SET name = COALESCE($2, name),
                 schema = COALESCE($3, schema),
                 template        = CASE WHEN $4 THEN $5 ELSE template END,
                 template_engine = CASE WHEN $4 THEN $6 ELSE template_engine END,
                 version = version + CASE WHEN $7 THEN 1 ELSE 0 END,
                 etag = encode(gen_random_bytes(8), 'hex'),
                 updated_at = now()
             WHERE id = $1
             RETURNING {COLS}"
        );
        match sqlx::query_as::<_, CreativeTemplate>(&sql)
            .bind(id)
            .bind(req.name.as_deref())
            .bind(req.schema.as_ref())
            .bind(replace_template)
            .bind(new_template)
            .bind(new_engine)
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
