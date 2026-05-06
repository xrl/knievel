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
    // Bind both GUCs and verify the project is visible (i.e. lives
    // under the principal's org). The projects RLS policy filters
    // by org_id OR project_id; a wrong-tenant project is invisible.
    let mut tx = db::begin_bound(pool, &principal.org_id, Some(path_project_id))
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
    Ok(tx)
}
