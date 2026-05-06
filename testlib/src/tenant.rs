//! Tenant-binding helpers for integration tests.
//!
//! Establishes the session-scoped GUCs (`knievel.org_id`,
//! `knievel.project_id`) that RLS policies introduced in
//! migration `0002_tenants.sql` rely on.
//!
//! Production code sets these per-request via the `Principal`
//! extractor (Phase 3.2); tests use these helpers directly.
//!
//! Spec refs:
//!   - `REQUIREMENTS.md` § 7.1, § 7.1.1
//!   - `AUTH.md` "Authorization"

use anyhow::Result;
use sqlx::{PgPool, Postgres, Transaction};

/// Begin a transaction with `knievel.org_id` (and optionally
/// `knievel.project_id`) bound at the transaction scope. Use the
/// returned `Transaction` for all queries that need to be subject
/// to tenant RLS — closing it (`commit`/`rollback`) discards the
/// bindings.
///
/// `set_config(name, value, is_local=true)` is the parameterized
/// equivalent of `SET LOCAL`; we use it instead of literal
/// interpolation so caller-supplied IDs can't smuggle SQL.
pub async fn begin_bound<'p>(
    pool: &'p PgPool,
    org_id: &str,
    project_id: Option<&str>,
) -> Result<Transaction<'p, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('knievel.org_id', $1, true)")
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    if let Some(pid) = project_id {
        sqlx::query("SELECT set_config('knievel.project_id', $1, true)")
            .bind(pid)
            .execute(&mut *tx)
            .await?;
    }
    Ok(tx)
}
