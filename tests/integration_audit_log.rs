//! Integration test: `audit_log` is append-only and tenant-isolated.
//!
//! Phase 3.4. Spec refs:
//!   - `REQUIREMENTS.md` § 7.3 (audit_log: append-only, 365-day
//!     retention, monthly partitioned)
//!   - `TESTING.md` § 5.2 ("Append-only — `UPDATE` on `audit_log`
//!     is denied by RLS policy")
//!
//! Append-only is enforced by RLS via the absence of `FOR UPDATE`
//! and `FOR DELETE` policies. Postgres' FORCE'd RLS default-denies
//! operations without a matching policy, so UPDATE/DELETE find
//! zero rows visible and silently affect zero rows.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use testlib::tenant::begin_bound;

#[tokio::test]
async fn audit_log_is_append_only_and_tenant_scoped() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    // Seed two orgs.
    for org in ["org_a", "org_b"] {
        let mut tx = begin_bound(&db.pool, org, None).await?;
        sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
            .bind(org)
            .bind(format!("Org {}", org.to_uppercase()))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }

    // Insert one audit row under each org.
    for org in ["org_a", "org_b"] {
        let mut tx = begin_bound(&db.pool, org, None).await?;
        sqlx::query(
            "INSERT INTO knievel.audit_log (org_id, actor, operation, reason)
             VALUES ($1, 'test-actor', 'test.operation', 'fixture')",
        )
        .bind(org)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    // Org A sees one row, never org B's.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.audit_log")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(count, 1, "org A session sees only its own row");

        let visible_org: String = sqlx::query_scalar("SELECT org_id FROM knievel.audit_log")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(visible_org, "org_a");
        tx.rollback().await?;
    }

    // Append-only: UPDATE finds zero rows visible to UPDATE because
    // no FOR UPDATE policy exists. The statement succeeds but
    // affects 0 rows; the original row stays put.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let result =
            sqlx::query("UPDATE knievel.audit_log SET reason = 'tampered' WHERE org_id = 'org_a'")
                .execute(&mut *tx)
                .await?;
        assert_eq!(
            result.rows_affected(),
            0,
            "UPDATE on audit_log must affect zero rows (RLS default-deny)"
        );
        tx.commit().await?;
    }

    // The row's reason is unchanged.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let reason: String = sqlx::query_scalar("SELECT reason FROM knievel.audit_log")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(
            reason, "fixture",
            "audit row content survived UPDATE attempt"
        );
        tx.rollback().await?;
    }

    // Same for DELETE.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let result = sqlx::query("DELETE FROM knievel.audit_log WHERE org_id = 'org_a'")
            .execute(&mut *tx)
            .await?;
        assert_eq!(
            result.rows_affected(),
            0,
            "DELETE on audit_log must affect zero rows (RLS default-deny)"
        );
        tx.commit().await?;
    }

    // Row still present.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.audit_log")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(count, 1, "audit row still present after DELETE attempt");
        tx.rollback().await?;
    }

    // Cross-tenant write attempt — bound to org_a, try to insert
    // a row whose org_id is org_b. WITH CHECK rejects.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let result = sqlx::query(
            "INSERT INTO knievel.audit_log (org_id, actor, operation)
             VALUES ('org_b', 'attacker', 'cross.tenant')",
        )
        .execute(&mut *tx)
        .await;
        assert!(
            result.is_err(),
            "WITH CHECK should reject inserting an audit row whose org_id != bound org_id"
        );
        tx.rollback().await?;
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
