//! Integration test: row-level security on `organizations` and
//! `projects` enforces tenant isolation.
//!
//! Phase 3.1. Spec refs:
//!   - `REQUIREMENTS.md` § 7.1, § 7.1.1 (RLS as defense in depth,
//!     verified at runtime).
//!   - `TESTING.md` § 5.3 (RLS verification at the DB layer).
//!
//! Skipped (with a warning) when `DATABASE_URL` is not set; runs
//! against the CI Postgres service container otherwise.

use anyhow::Result;
use testlib::tenant::begin_bound;

#[tokio::test]
async fn rls_isolates_projects_across_orgs() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!(
            "DATABASE_URL not set; skipping. CI's Postgres service \
             container provides this."
        );
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    // Seed two orgs, each with a project. We bind to each org in
    // turn to satisfy the WITH CHECK clause on the policies. RLS is
    // FORCE'd, so even the table owner is gated.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ('org_a', 'Org A')")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO knievel.projects (id, org_id, name) VALUES ('pj_a1', 'org_a', 'Project A1')",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    {
        let mut tx = begin_bound(&db.pool, "org_b", None).await?;
        sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ('org_b', 'Org B')")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO knievel.projects (id, org_id, name) VALUES ('pj_b1', 'org_b', 'Project B1')",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    // Bound to org A: sees its own org and its own project, not org B's.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let org_count: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.organizations")
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(org_count, 1, "org A session sees only org A");

        let project_count: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.projects")
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(project_count, 1, "org A session sees only org A's project");

        let visible_org: String = sqlx::query_scalar("SELECT id FROM knievel.organizations")
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(visible_org, "org_a");
        tx.rollback().await?;
    }

    // Bound to org B, project B1: sees org B + project B1, never project A1.
    {
        let mut tx = begin_bound(&db.pool, "org_b", Some("pj_b1")).await?;
        let project_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM knievel.projects ORDER BY id")
                .fetch_all(&mut *tx)
                .await?;
        assert_eq!(
            project_ids,
            vec!["pj_b1"],
            "project A1 invisible from org B"
        );
        tx.rollback().await?;
    }

    // Bound to project A1 only (no org_id): sees its own row and
    // its parent org via the inheritance subquery in the org policy.
    {
        let mut tx = begin_bound(&db.pool, "", Some("pj_a1")).await?;
        let project_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM knievel.projects ORDER BY id")
                .fetch_all(&mut *tx)
                .await?;
        assert_eq!(
            project_ids,
            vec!["pj_a1"],
            "project-bound session sees only its own project"
        );
        let org_ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM knievel.organizations ORDER BY id")
                .fetch_all(&mut *tx)
                .await?;
        assert_eq!(
            org_ids,
            vec!["org_a"],
            "project-bound session sees its parent org"
        );
        tx.rollback().await?;
    }

    // WITH CHECK refuses a write that targets the wrong org.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        let result = sqlx::query(
            "INSERT INTO knievel.organizations (id, name) VALUES ('org_b_clone', 'Bad')",
        )
        .execute(&mut *tx)
        .await;
        assert!(
            result.is_err(),
            "WITH CHECK should reject inserting an org whose id != bound org_id"
        );
        tx.rollback().await?;
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
