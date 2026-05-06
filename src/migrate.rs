//! Migration runner.
//!
//! Phase 4.1. Embeds every file under `migrations/` into the
//! binary at compile time and exposes a `run` helper that:
//!
//!   1. Creates the `knievel` schema (idempotent — first-boot
//!      bootstrap; matches the operator-equivalent `CREATE
//!      SCHEMA` from `MIGRATION_RX.md` "One-time provisioning").
//!   2. Creates the `pgcrypto` extension (also idempotent).
//!   3. Runs the embedded `sqlx` migrator. The pool's
//!      `after_connect` hook in `server.rs` sets
//!      `search_path = knievel, public` before any query so the
//!      tracking table (`_sqlx_migrations`) lands in `knievel`,
//!      not `public`.
//!
//! Wired behind the `database.auto_migrate` config flag so
//! production deployments that prefer running migrations
//! out-of-band can leave the flag false.

use anyhow::{Context, Result};
use sqlx::migrate::Migrator;
use sqlx::PgPool;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn run(pool: &PgPool) -> Result<()> {
    sqlx::query("CREATE SCHEMA IF NOT EXISTS knievel")
        .execute(pool)
        .await
        .context("creating knievel schema")?;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(pool)
        .await
        .context("creating pgcrypto extension")?;
    MIGRATOR
        .run(pool)
        .await
        .context("applying embedded migrations")?;
    Ok(())
}
