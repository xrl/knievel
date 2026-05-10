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
//!
//! ## Why SELECT-first instead of `CREATE … IF NOT EXISTS`
//!
//! Postgres checks the `CREATE`-on-database privilege *before*
//! the `IF NOT EXISTS` short-circuit fires. On Aurora/RDS where
//! the schema and extension are pre-provisioned by a master role
//! (the `MIGRATION_RX.md` "One-time provisioning" pattern that
//! `infra` follows for `sci_rx_production_knievel`), the app
//! role doesn't have `CREATE` on the database — it only owns
//! its own schema. A bare `CREATE SCHEMA IF NOT EXISTS knievel`
//! still trips the privilege check and surfaces as `permission
//! denied for database <name>`, even though the schema is
//! already there. We probe `pg_namespace` / `pg_extension`
//! first so the privileged DDL is never attempted on the
//! happy path. In CI/dev (where the role IS the DB owner) the
//! existence check returns `false` on a fresh DB, so behaviour
//! is unchanged for those environments.

use anyhow::{Context, Result};
use sqlx::migrate::Migrator;
use sqlx::PgPool;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn run(pool: &PgPool) -> Result<()> {
    let schema_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'knievel')")
            .fetch_one(pool)
            .await
            .context("checking knievel schema existence")?;
    if !schema_exists {
        sqlx::query("CREATE SCHEMA knievel")
            .execute(pool)
            .await
            .context("creating knievel schema")?;
    }

    let pgcrypto_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pgcrypto')")
            .fetch_one(pool)
            .await
            .context("checking pgcrypto extension existence")?;
    if !pgcrypto_exists {
        sqlx::query("CREATE EXTENSION pgcrypto")
            .execute(pool)
            .await
            .context("creating pgcrypto extension")?;
    }

    MIGRATOR
        .run(pool)
        .await
        .context("applying embedded migrations")?;
    Ok(())
}
