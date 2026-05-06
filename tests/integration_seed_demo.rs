//! Integration test: `knievel-cli seed-demo` populates a working
//! demo project end-to-end and is idempotent on re-run.
//!
//! Phase 4.2. Refs: `REQUIREMENTS.md` § 8 item 4; `AUTH.md` "Local
//! Development"; `MIGRATION_RX.md` "Local Development for RX
//! Engineers."
//!
//! Skipped (with a warning) when `DATABASE_URL` is not set.

use anyhow::Result;
use knievel::cli::seed_demo::{run, SeedDemoArgs};

#[tokio::test]
async fn seed_demo_seeds_a_decision_ready_project() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    let out = run(SeedDemoArgs {
        database_url: db.url.clone(),
        org_external_id: "demo-org".into(),
        project_external_id: "demo-project".into(),
        token: None,
        write_token_to: None,
    })
    .await?;

    // Sanity: every id is non-empty / non-zero.
    assert!(out.org_id.starts_with("org_"));
    assert!(out.project_id.starts_with("pj_"));
    assert!(out.advertiser_id > 0);
    assert!(out.campaign_id > 0);
    assert!(out.flight_id > 0);
    assert!(out.ad_id > 0);
    assert!(out.creative_id > 0);
    assert!(out.site_id > 0);
    assert!(out.zone_id > 0);
    assert!(out.priority_id > 0);
    assert!(out.ad_type_id > 0);
    assert!(out.token.starts_with("kvl_dev_org_"));

    // Verify the demo data is wired up correctly: the ad references
    // the flight which references the campaign which references the
    // advertiser; and the creative is linked to the ad.
    {
        let mut tx =
            testlib::tenant::begin_bound(&db.pool, &out.org_id, Some(&out.project_id)).await?;

        let (ad_flight_id, ad_creative_id): (i64, Option<i64>) =
            sqlx::query_as("SELECT flight_id, creative_id FROM knievel.ads WHERE id = $1")
                .bind(out.ad_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(ad_flight_id, out.flight_id);
        assert_eq!(ad_creative_id, Some(out.creative_id));

        let (flight_campaign_id, flight_priority_id, flight_ad_types): (i64, i64, Vec<i64>) =
            sqlx::query_as(
                "SELECT campaign_id, priority_id, ad_types
             FROM knievel.flights WHERE id = $1",
            )
            .bind(out.flight_id)
            .fetch_one(&mut *tx)
            .await?;
        assert_eq!(flight_campaign_id, out.campaign_id);
        assert_eq!(flight_priority_id, out.priority_id);
        assert_eq!(flight_ad_types, vec![out.ad_type_id]);

        let zone_site_id: i64 =
            sqlx::query_scalar("SELECT site_id FROM knievel.zones WHERE id = $1")
                .bind(out.zone_id)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(zone_site_id, out.site_id);

        // The bootstrap token is org-scoped + org-admin role and
        // its id matches the wire-format id_short.
        let id_short = out
            .token
            .split('_')
            .nth(3)
            .expect("token has id_short segment");
        let (token_org, token_scope, token_role): (String, String, String) =
            sqlx::query_as("SELECT org_id, scope, role FROM knievel.api_tokens WHERE id = $1")
                .bind(format!("tok_{id_short}"))
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(token_org, out.org_id);
        assert_eq!(token_scope, "org");
        assert_eq!(token_role, "org-admin");

        tx.rollback().await?;
    }

    // Idempotency: re-running with the same external_ids reuses
    // every row by external_id. The token column is rotated by
    // upsert if a fresh `--token` is passed; passing `None`
    // generates a brand-new bearer (which lands as a new token row
    // — that's intentional, the previous random token stays
    // active). We assert the resource ids are unchanged.
    let out2 = run(SeedDemoArgs {
        database_url: db.url.clone(),
        org_external_id: "demo-org".into(),
        project_external_id: "demo-project".into(),
        token: None,
        write_token_to: None,
    })
    .await?;
    assert_eq!(out2.org_id, out.org_id);
    assert_eq!(out2.project_id, out.project_id);
    assert_eq!(out2.advertiser_id, out.advertiser_id);
    assert_eq!(out2.campaign_id, out.campaign_id);
    assert_eq!(out2.flight_id, out.flight_id);
    assert_eq!(out2.ad_id, out.ad_id);
    assert_eq!(out2.creative_id, out.creative_id);
    assert_eq!(out2.site_id, out.site_id);
    assert_eq!(out2.zone_id, out.zone_id);

    // Idempotency on a supplied --token: re-running with the same
    // token rotates the secret_hash for that row but doesn't add
    // a new row.
    let supplied = "kvl_dev_org_seedtok_supplied".to_string();
    let _out3 = run(SeedDemoArgs {
        database_url: db.url.clone(),
        org_external_id: "demo-org".into(),
        project_external_id: "demo-project".into(),
        token: Some(supplied.clone()),
        write_token_to: None,
    })
    .await?;
    let _out4 = run(SeedDemoArgs {
        database_url: db.url.clone(),
        org_external_id: "demo-org".into(),
        project_external_id: "demo-project".into(),
        token: Some(supplied.clone()),
        write_token_to: None,
    })
    .await?;
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, &out.org_id, None).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM knievel.api_tokens WHERE id = 'tok_seedtok'",
        )
        .fetch_one(&mut *tx)
        .await?;
        assert_eq!(count, 1, "supplied --token rotates rather than duplicates");
        tx.rollback().await?;
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
