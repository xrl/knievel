//! Integration test: migrations apply cleanly to a fresh DB and
//! `knievel.config_version` behaves as expected.
//!
//! Phase 1.6 + 1.9. Spec ref: `REQUIREMENTS.md` § 7.2,
//! `TESTING.md` § 5.1.
//!
//! Skipped (with a warning) when `DATABASE_URL` is not set, so a
//! contributor without a Postgres handy can still run
//! `cargo nextest run` for the unit tier.

use anyhow::Result;

#[tokio::test]
async fn migrations_apply_and_config_version_increments() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!(
            "DATABASE_URL not set; skipping. CI's Postgres service \
             container provides this."
        );
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    // Sequence's first nextval should be 1 (matches START WITH 1
    // in 0001_init.sql).
    let v: i64 = sqlx::query_scalar("SELECT nextval('knievel.config_version')")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(v, 1, "first nextval = START WITH value");

    // Subsequent nextvals increment monotonically by 1.
    let v: i64 = sqlx::query_scalar("SELECT nextval('knievel.config_version')")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(v, 2, "second nextval = 2");

    // last_value reflects the most recent nextval.
    let last: i64 = sqlx::query_scalar("SELECT last_value FROM knievel.config_version")
        .fetch_one(&db.pool)
        .await?;
    assert_eq!(last, 2);

    // Migration tracking row exists in the knievel schema (proof
    // that search_path scoping worked as intended).
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM knievel._sqlx_migrations",
    )
    .fetch_one(&db.pool)
    .await?;
    assert!(count >= 1, "at least one migration tracked, got {count}");

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
