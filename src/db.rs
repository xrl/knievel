//! Database session helpers.
//!
//! Two transaction openers, both built on `set_config(name, value,
//! is_local=true)` (the parameterized `SET LOCAL` equivalent —
//! caller-supplied IDs can't smuggle SQL).
//!
//! - `begin_bound` opens a tenant-bound transaction. The
//!   `knievel.org_id` and (optional) `knievel.project_id` GUCs are
//!   the binding the RLS policies in migration `0002_tenants.sql`
//!   read.
//! - `begin_auth_lookup` opens a transaction with the
//!   `knievel.auth_lookup_id` GUC set. The Phase 3.2 `api_tokens`
//!   RLS policy uses this as a single-row bypass at auth time:
//!   the principal extractor (`auth::security`) parses an opaque
//!   token's id, scopes the bypass to that one row, queries by
//!   PK, and verifies argon2id — all before the tenant GUCs can
//!   possibly be known.
//!
//! Spec refs: `REQUIREMENTS.md` § 7.1, § 7.1.1, `AUTH.md`
//! "Authorization."

use anyhow::Result;
use sqlx::{PgPool, Postgres, Transaction};

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

pub async fn begin_auth_lookup<'p>(
    pool: &'p PgPool,
    token_id: &str,
) -> Result<Transaction<'p, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('knievel.auth_lookup_id', $1, true)")
        .bind(token_id)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
