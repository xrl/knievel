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

    // The Postgres docker image creates `POSTGRES_USER` as a
    // SUPERUSER, and Postgres superusers bypass RLS unconditionally
    // — even with `FORCE ROW LEVEL SECURITY` — which silently
    // defeats every cross-tenant isolation test. Drop superuser
    // (keep CREATEDB so we can still mint ephemerals) so the role
    // is gated by FORCE'd policies, matching production where the
    // app role is non-superuser per `MIGRATION_RX.md`.
    //
    // Many test processes call `ephemeral()` concurrently; the
    // SELECT/ALTER pair has to be serialized at the cluster
    // level or the second-and-subsequent ALTERs trip the
    // mid-flight loss of privilege. `pg_advisory_xact_lock` with
    // a deterministic key is the standard recipe — first caller
    // does the work, the rest no-op once they see rolsuper
    // already false. The lock releases automatically when the
    // wrapping transaction commits.
    {
        let mut tx = admin.begin().await.context("opening role-downgrade tx")?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('knievel_testlib_role_setup')::bigint)")
            .execute(&mut *tx)
            .await
            .context("acquiring role-setup advisory lock")?;
        let is_super: bool =
            sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
                .fetch_one(&mut *tx)
                .await
                .context("checking admin role attributes")?;
        if is_super {
            // Wrap the ALTER in a savepoint so a failure (some
            // managed Postgres tiers refuse self-alter even from
            // a SUPERUSER; observed once on a CI Postgres 16 run)
            // can be caught and we can fall through if the role
            // has somehow already reached the desired state. With
            // the savepoint we keep the outer txn alive for the
            // commit below.
            //
            // Substitute the literal role name rather than using
            // the `CURRENT_USER` keyword. Postgres 16.13 rejects
            // `ALTER ROLE CURRENT_USER NOSUPERUSER` with
            // "permission denied to alter role" even when the
            // session is a verified superuser (`rolsuper=true`,
            // `is_superuser` GUC `on`); using the literal role
            // name takes the bug-prone code path out of the loop.
            let role_name: String = sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(&mut *tx)
                .await
                .context("reading current_user for role name")?;
            // Quote-escape to handle role names with special
            // chars / case-sensitivity. Use double-quote escaping
            // — Postgres identifier rules.
            let quoted = format!("\"{}\"", role_name.replace('"', "\"\""));
            let alter_sql = format!("ALTER ROLE {quoted} NOSUPERUSER CREATEDB");
            sqlx::query("SAVEPOINT role_downgrade")
                .execute(&mut *tx)
                .await
                .context("opening role-downgrade savepoint")?;
            let altered = sqlx::query(&alter_sql).execute(&mut *tx).await;
            match altered {
                Ok(_) => {
                    sqlx::query("RELEASE SAVEPOINT role_downgrade")
                        .execute(&mut *tx)
                        .await
                        .context("releasing role-downgrade savepoint")?;
                }
                Err(e) => {
                    sqlx::query("ROLLBACK TO SAVEPOINT role_downgrade")
                        .execute(&mut *tx)
                        .await
                        .context("rolling back role-downgrade savepoint")?;
                    let still: bool = sqlx::query_scalar(
                        "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
                    )
                    .fetch_one(&mut *tx)
                    .await
                    .context("re-checking role state after ALTER failed")?;
                    if still {
                        // Diagnostic dump — Postgres is rejecting
                        // self-alter despite rolsuper=true. Most
                        // Postgres-tier configurations should let
                        // a SUPERUSER ALTER themselves; if this
                        // fires we want to see the actual session
                        // state to debug.
                        let diag: (String, String, bool, bool, bool, bool, String, String) =
                            sqlx::query_as(
                                "SELECT current_user::text, session_user::text,
                                    rolsuper, rolinherit, rolcreaterole, rolcreatedb,
                                    current_setting('is_superuser')::text,
                                    version()::text
                             FROM pg_roles WHERE rolname = current_user",
                            )
                            .fetch_one(&mut *tx)
                            .await
                            .unwrap_or_default();
                        return Err(e).context(format!(
                            "downgrading admin role from SUPERUSER \
                             (current_user={}, session_user={}, \
                             rolsuper={}, rolinherit={}, rolcreaterole={}, rolcreatedb={}, \
                             is_superuser_guc={}, pg_version={:?})",
                            diag.0, diag.1, diag.2, diag.3, diag.4, diag.5, diag.6, diag.7
                        ));
                    }
                    // Else: the role is NOSUPERUSER (somehow
                    // already in the right state — a parallel
                    // process beat us to the alter, or the
                    // managed Postgres pre-provisioned the
                    // role). Continue.
                }
            }
        }
        tx.commit().await.context("committing role-downgrade tx")?;
    }

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
///
/// `pool.close()` waits for the test's own pool to drain, but
/// API tests typically clone the pool into `AppState` (held by
/// `poem::test::TestClient`), and those cloned handles can keep
/// backends alive past `close()`. Postgres then refuses
/// `DROP DATABASE` with "is being accessed by other users".
/// We pre-empt that by terminating every backend on the
/// ephemeral database before issuing the drop.
pub async fn ephemeral_drop(db: EphemeralDb) -> Result<()> {
    let admin_url = env::var("DATABASE_URL")?;
    let pool = db.pool;
    pool.close().await;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await?;
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = $1 AND pid <> pg_backend_pid() \
           AND usename = current_user",
    )
    .bind(&db.name)
    .execute(&admin)
    .await
    .context("terminating lingering backends on ephemeral DB")?;
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
