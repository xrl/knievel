//! Common handler infrastructure for project-scoped resources.
//!
//! Phase 3.8. Every project-scoped CRUD handler runs the same
//! prologue: validate Bearer, look up the path's project to
//! confirm it lives under the principal's org, project-scoped
//! tokens additionally must match the path project, role check,
//! tenant-bound transaction. Centralizing this keeps the per-
//! resource handlers focused on the resource-specific bits.

use sqlx::{PgPool, Postgres, Transaction};

use crate::auth::{Principal, Role, Scope};
use crate::db;

#[derive(Debug, Clone, Copy)]
pub enum AuthzError {
    /// Path project doesn't exist or isn't in the principal's org.
    WrongTenant,
    /// Project-scoped token addressing a different project.
    WrongProject,
    /// Principal's role is below the endpoint's minimum.
    RoleInsufficient,
    /// Internal DB error during the project lookup.
    Internal,
}

impl AuthzError {
    pub fn code(self) -> &'static str {
        match self {
            AuthzError::WrongTenant => "wrong_tenant",
            AuthzError::WrongProject => "wrong_project",
            AuthzError::RoleInsufficient => "role_insufficient",
            AuthzError::Internal => "internal_error",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            AuthzError::WrongTenant => "project does not belong to the principal's org",
            AuthzError::WrongProject => "project-scoped token cannot address a different project",
            AuthzError::RoleInsufficient => "principal's role is below the endpoint minimum",
            AuthzError::Internal => "internal authorization error",
        }
    }
}

/// Open a tenant-bound transaction after validating the principal
/// is allowed to operate on `path_project_id` at `min_role`.
///
/// On success the returned transaction has both
/// `knievel.org_id` and `knievel.project_id` set so RLS policies
/// pass through. Caller must `commit` or `rollback`.
pub async fn open_project_tx<'p>(
    pool: &'p PgPool,
    principal: &Principal,
    path_project_id: &str,
    min_role: Role,
) -> Result<Transaction<'p, Postgres>, AuthzError> {
    if !principal.has_role_at_least(min_role) {
        return Err(AuthzError::RoleInsufficient);
    }
    if matches!(principal.scope, Scope::Project) {
        let pid = principal.project_id.as_deref().unwrap_or("");
        if pid != path_project_id {
            return Err(AuthzError::WrongProject);
        }
    }
    // Two-step bind: verify the project is in the principal's org
    // with ONLY `knievel.org_id` set, then add `knievel.project_id`
    // after the verify. The projects RLS policy reads `org_id OR
    // id = bound project_id`; binding both GUCs up front would let
    // a wrong-tenant project pass the verify via the `id` clause
    // matching itself. With project_id GUC unset, the OR's right
    // side is NULL → falsy and only the org-membership check
    // gates the row.
    let mut tx = db::begin_bound(pool, &principal.org_id, None)
        .await
        .map_err(|_| AuthzError::Internal)?;
    let row: Option<(String,)> = sqlx::query_as("SELECT id FROM knievel.projects WHERE id = $1")
        .bind(path_project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| AuthzError::Internal)?;
    if row.is_none() {
        return Err(AuthzError::WrongTenant);
    }
    sqlx::query("SELECT set_config('knievel.project_id', $1, true)")
        .bind(path_project_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| AuthzError::Internal)?;
    Ok(tx)
}

/// Open an org-scoped tenant-bound transaction after validating the
/// principal can operate on `path_org_id` at `min_role`. Mirrors
/// `open_project_tx` for resources whose paths are
/// `/v1/orgs/:org_id/...` (today: tokens, ad_library) so org-scoped
/// handlers don't have to hand-roll the same prologue six times.
///
/// The returned transaction has `knievel.org_id` set so RLS
/// policies on org-scoped tables (`organizations`, `api_tokens`,
/// `ad_library_items`, `audit_log`) pass through. `project_id` is
/// **not** bound here — org-scoped handlers operate above the
/// project boundary.
///
/// `WrongProject` is the catch-all for project-scoped tokens
/// addressing org-scoped routes: a project-scoped principal cannot
/// mint a token or write to the ad library by spec
/// (`AUTH.md` "Authorization"), so we surface that as
/// `wrong_project` rather than `role_insufficient` to give callers
/// a clearer reason.
pub async fn open_org_tx<'p>(
    pool: &'p PgPool,
    principal: &Principal,
    path_org_id: &str,
    min_role: Role,
) -> Result<Transaction<'p, Postgres>, AuthzError> {
    if principal.org_id != path_org_id {
        return Err(AuthzError::WrongTenant);
    }
    if matches!(principal.scope, Scope::Project) {
        // Project-scoped tokens cannot address org-scoped routes.
        // The same code path the project-scoped flow uses for a
        // mismatched project — keep the wire-shape consistent.
        return Err(AuthzError::WrongProject);
    }
    if !principal.has_role_at_least(min_role) {
        return Err(AuthzError::RoleInsufficient);
    }
    let tx = db::begin_bound(pool, &principal.org_id, None)
        .await
        .map_err(|_| AuthzError::Internal)?;
    Ok(tx)
}
