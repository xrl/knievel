//! Ephemeral database fixture.
//!
//! `ephemeral()` creates a fresh, uniquely-named Postgres database
//! against the cluster reachable via `DATABASE_URL`, provisions the
//! `knievel` schema and `pgcrypto` extension (operator-equivalent
//! steps — see `MIGRATION_RX.md` "One-time provisioning"), runs all
//! migrations, and returns a connected pool.
//!
//! The DB is **not** auto-cleaned: the caller (or CI's job
//! teardown) is responsible. `ephemeral_drop` is provided for
//! tests that want explicit cleanup; in practice CI's per-job
//! Postgres service container is destroyed at job end so leakage
//! is bounded by the job lifetime.

use anyhow::{anyhow, Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env;
use std::path::PathBuf;

pub struct EphemeralDb {
    pub pool: PgPool,
    pub url: String,
    pub name: String,
}

/// Provision a fresh ephemeral test database.
///
/// Requires `DATABASE_URL` to point at a Postgres cluster reachable
/// with `CREATE DATABASE` privileges. The connection string's
/// database name is replaced with a unique throwaway name; the
/// rest (host, port, user, password, query string) is preserved.
pub async fn ephemeral() -> Result<EphemeralDb> {
    let admin_url = env::var("DATABASE_URL")
        .context("DATABASE_URL must point at a Postgres cluster admin connection")?;

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .context("connecting to admin DB")?;

    // uuid v4 simple = 32 hex chars; truncate to 16 to keep the DB
    // name comfortably below Postgres's 63-char NAMEDATALEN limit.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("knievel_test_{}", &suffix[..16]);

    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await
        .with_context(|| format!("creating test database {name}"))?;
    drop(admin);

    let url = replace_dbname_in_url(&admin_url, &name)?;
    // `after_connect` runs on every connection the pool hands out,
    // so sqlx's migrator (which creates `_sqlx_migrations` on its
    // first connection) sees `search_path = knievel, public` and
    // lands the tracking table in the knievel schema rather than
    // public. Mirrors the production `ALTER ROLE knievel_app SET
    // search_path = ...` recipe in MIGRATION_RX.md.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET search_path TO knievel, public")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .context("connecting to ephemeral DB")?;

    // Operator-equivalent provisioning. In production these are
    // run once by a cluster admin per `MIGRATION_RX.md`; tests
    // re-run them per ephemeral DB.
    sqlx::query("CREATE SCHEMA IF NOT EXISTS knievel")
        .execute(&pool)
        .await
        .context("creating knievel schema")?;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&pool)
        .await
        .context("creating pgcrypto extension")?;

    let migrator = sqlx::migrate::Migrator::new(migrations_dir())
        .await
        .context("loading migrations")?;
    migrator.run(&pool).await.context("applying migrations")?;

    Ok(EphemeralDb { pool, url, name })
}

/// Drop the ephemeral database. Optional cleanup; CI tears the
/// service container down regardless.
pub async fn ephemeral_drop(db: EphemeralDb) -> Result<()> {
    let admin_url = env::var("DATABASE_URL")?;
    let pool = db.pool;
    pool.close().await;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await?;
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\"", db.name))
        .execute(&admin)
        .await
        .context("dropping ephemeral DB")?;
    Ok(())
}

/// Where the migrations live, relative to this crate. The crate
/// is at `testlib/`, the migrations at `migrations/` of the repo
/// root.
fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("testlib has a parent dir (the repo root)")
        .join("migrations")
}

fn replace_dbname_in_url(url: &str, new_dbname: &str) -> Result<String> {
    // postgres://user:pass@host:port/dbname?query#frag
    let (base, tail) = match url.find('?') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, ""),
    };
    let last_slash = base
        .rfind('/')
        .ok_or_else(|| anyhow!("malformed DATABASE_URL: no path slash"))?;
    Ok(format!("{}{}{}", &base[..last_slash + 1], new_dbname, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_dbname_replacement() {
        assert_eq!(
            replace_dbname_in_url("postgres://u:p@h:5432/old?sslmode=require", "new").unwrap(),
            "postgres://u:p@h:5432/new?sslmode=require"
        );
        assert_eq!(
            replace_dbname_in_url("postgres://u:p@h/old", "fresh").unwrap(),
            "postgres://u:p@h/fresh"
        );
    }
}
