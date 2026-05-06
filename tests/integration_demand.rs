//! Integration test: demand-chain + inventory + taxonomy tables
//! enforce per-project RLS isolation.
//!
//! Phase 3.7. One round-trip per resource: insert under project A,
//! confirm invisibility from project B's session and WITH CHECK
//! rejection of a wrong-project insert.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use testlib::tenant::begin_bound;

#[tokio::test]
async fn rls_isolates_demand_inventory_taxonomy_across_projects() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }

    let db = testlib::db::ephemeral().await?;

    // Two orgs, one project each.
    seed_org_and_project(&db.pool, "org_a", "pj_a").await?;
    seed_org_and_project(&db.pool, "org_b", "pj_b").await?;

    // Insert one row of each project-scoped resource under
    // (org_a, pj_a). begin_bound(org, project) sets both GUCs
    // so the RLS policies (which key on knievel.project_id) let
    // the writes through.
    {
        let mut tx = begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
        sqlx::query(
            "INSERT INTO knievel.advertisers (org_id, project_id, name)
             VALUES ('org_a', 'pj_a', 'Acme')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.campaigns (org_id, project_id, advertiser_id, name)
             VALUES ('org_a', 'pj_a',
                     (SELECT id FROM knievel.advertisers WHERE name='Acme'),
                     'Spring')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.creative_templates (org_id, project_id, name, schema)
             VALUES ('org_a', 'pj_a', 'card_v1', '{}'::jsonb)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.creatives
                 (org_id, project_id, advertiser_id, kind, image_url,
                  width, height, click_through_url)
             VALUES ('org_a', 'pj_a',
                     (SELECT id FROM knievel.advertisers WHERE name='Acme'),
                     'image', 'https://x', 100, 100, 'https://t')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.flights
                 (org_id, project_id, campaign_id, name, priority_id, ad_types)
             VALUES ('org_a', 'pj_a',
                     (SELECT id FROM knievel.campaigns WHERE name='Spring'),
                     'F', 1, '{1}'::bigint[])",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.ads (org_id, project_id, flight_id, creative_id)
             VALUES ('org_a', 'pj_a',
                     (SELECT id FROM knievel.flights WHERE name='F'),
                     (SELECT id FROM knievel.creatives WHERE kind='image' LIMIT 1))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.sites (org_id, project_id, name, url)
             VALUES ('org_a', 'pj_a', 'Main', 'https://main.example')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.zones (org_id, project_id, site_id, name)
             VALUES ('org_a', 'pj_a',
                     (SELECT id FROM knievel.sites WHERE name='Main'),
                     'Header')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.channels (org_id, project_id, name)
             VALUES ('org_a', 'pj_a', 'Web')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.priorities (org_id, project_id, name, tier)
             VALUES ('org_a', 'pj_a', 'House', 1)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO knievel.ad_types (org_id, project_id, name, width, height)
             VALUES ('org_a', 'pj_a', '300x250', 300, 250)",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    // From pj_b's session, every project-scoped table reports zero rows.
    let tables = [
        "advertisers",
        "campaigns",
        "creative_templates",
        "creatives",
        "flights",
        "ads",
        "sites",
        "zones",
        "channels",
        "priorities",
        "ad_types",
    ];
    {
        let mut tx = begin_bound(&db.pool, "org_b", Some("pj_b")).await?;
        for t in tables {
            let n: i64 = sqlx::query_scalar(&format!("SELECT count(*)::bigint FROM knievel.{t}"))
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(n, 0, "pj_b session must not see knievel.{t} rows");
        }
        tx.rollback().await?;
    }

    // WITH CHECK rejects a wrong-project write (bound to pj_b,
    // try to insert into pj_a).
    {
        let mut tx = begin_bound(&db.pool, "org_a", Some("pj_b")).await?;
        let r = sqlx::query(
            "INSERT INTO knievel.advertisers (org_id, project_id, name)
             VALUES ('org_a', 'pj_a', 'sneak')",
        )
        .execute(&mut *tx)
        .await;
        assert!(r.is_err(), "WITH CHECK must reject wrong-project insert");
        tx.rollback().await?;
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

async fn seed_org_and_project(pool: &sqlx::PgPool, org: &str, proj: &str) -> Result<()> {
    let mut tx = begin_bound(pool, org, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org)
        .bind(format!("Org {org}"))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(proj)
        .bind(org)
        .bind(format!("Project {proj}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
