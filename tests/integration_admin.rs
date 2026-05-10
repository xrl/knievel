//! Integration test: `knievel-cli admin create-org` provisions a
//! tenant idempotently and mints a usable org-admin token, without
//! seeding any demo project / advertiser / campaign / flight / ad /
//! creative / site / zone rows.
//!
//! Skipped (with a warning) when `DATABASE_URL` is not set; CI's
//! Postgres service container provides this.

use anyhow::Result;
use knievel::cli::admin::{create_org, CreateOrgArgs};

#[tokio::test]
async fn admin_create_org_provisions_tenant_without_demo_fixture() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    let out = create_org(CreateOrgArgs {
        database_url: db.url.clone(),
        external_id: "rx".into(),
        name: "Real X".into(),
        token: None,
        write_token_to: None,
    })
    .await?;

    assert!(out.org_id.starts_with("org_"));
    assert!(out.token.starts_with("kvl_dev_org_"));
    assert!(out.org_was_new, "first call inserts a fresh org row");

    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, &out.org_id, None).await?;

        // The org row exists with the supplied display name.
        let org_name: String =
            sqlx::query_scalar("SELECT name FROM knievel.organizations WHERE id = $1")
                .bind(&out.org_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(org_name, "Real X");

        // Exactly one bootstrap token row exists and is org-admin scoped.
        let (token_scope, token_role, token_name): (String, String, String) =
            sqlx::query_as("SELECT scope, role, name FROM knievel.api_tokens WHERE org_id = $1")
                .bind(&out.org_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(token_scope, "org");
        assert_eq!(token_role, "org-admin");
        assert_eq!(token_name, "create-org bootstrap");

        // No demo-fixture rows landed: the production-shaped path
        // touches `organizations` and `api_tokens` only.
        let project_count: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.projects WHERE org_id = $1")
                .bind(&out.org_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(project_count, 0, "create-org must not seed a project");

        tx.rollback().await?;
    }

    // Idempotency: re-running with the same external_id reuses the
    // existing org row and rotates a freshly-supplied token rather
    // than inserting a second row.
    let supplied = "kvl_dev_org_rxbootp_supplied".to_string();
    let out2 = create_org(CreateOrgArgs {
        database_url: db.url.clone(),
        external_id: "rx".into(),
        name: "Real X (renamed in CLI but preserved on row)".into(),
        token: Some(supplied.clone()),
        write_token_to: None,
    })
    .await?;
    assert_eq!(out2.org_id, out.org_id, "re-run reuses derived org id");
    assert!(!out2.org_was_new, "re-run reports the org as reused");
    assert_eq!(out2.token, supplied);

    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, &out.org_id, None).await?;

        // Display name is preserved verbatim — operator-edited names
        // aren't clobbered by subsequent create-org invocations.
        let org_name: String =
            sqlx::query_scalar("SELECT name FROM knievel.organizations WHERE id = $1")
                .bind(&out.org_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(org_name, "Real X");

        // The supplied token rotated the hash on its row; the
        // randomly-generated first token is still in place too,
        // matching seed-demo's behaviour for unsupplied tokens.
        let token_count: i64 =
            sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.api_tokens WHERE org_id = $1")
                .bind(&out.org_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(
            token_count, 2,
            "first random + second supplied = 2 token rows"
        );

        tx.rollback().await?;
    }

    // A second invocation with the same supplied token rotates the
    // hash on the existing row rather than inserting a duplicate.
    let _out3 = create_org(CreateOrgArgs {
        database_url: db.url.clone(),
        external_id: "rx".into(),
        name: "Real X".into(),
        token: Some(supplied.clone()),
        write_token_to: None,
    })
    .await?;
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, &out.org_id, None).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM knievel.api_tokens WHERE id = 'tok_rxbootp'",
        )
        .fetch_one(&mut *tx)
        .await?;
        assert_eq!(count, 1, "supplied --token rotates rather than duplicates");
        tx.rollback().await?;
    }

    // Different external_ids derive distinct org ids.
    let other = create_org(CreateOrgArgs {
        database_url: db.url.clone(),
        external_id: "acme-corp".into(),
        name: "Acme Corp".into(),
        token: None,
        write_token_to: None,
    })
    .await?;
    assert_ne!(
        other.org_id, out.org_id,
        "different external_ids derive different org ids"
    );

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
