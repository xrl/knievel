//! API tests: read-only taxonomy + project-creation seeding.
//!
//! Phase 3.13. Asserts that creating a project seeds default
//! channels / priorities / ad_types under that project, and the
//! read endpoints return them tenant-scoped.
//!
//! Additional coverage (Fix(taxonomy) audit): re-seed idempotency via
//! `ON CONFLICT DO NOTHING`, `(project_id, name)` uniqueness enforcement,
//! and the defensive GUC check in `seed_default_taxonomy`.
//!
//! Skipped when `DATABASE_URL` is not set.

use anyhow::Result;
use poem::test::TestClient;
use poem::EndpointExt;

struct Fixture {
    db: testlib::db::EphemeralDb,
    org_a_admin: String,
    org_b_admin: String,
}

async fn setup() -> Result<Fixture> {
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;
    seed_org(&db.pool, "org_b").await?;
    let org_a_admin = mint_token(&db.pool, "tok_aadm", "org_a", "org-admin").await?;
    let org_b_admin = mint_token(&db.pool, "tok_badm", "org_b", "org-admin").await?;
    Ok(Fixture {
        db,
        org_a_admin,
        org_b_admin,
    })
}

async fn seed_org(pool: &sqlx::PgPool, org: &str) -> Result<()> {
    let mut tx = testlib::tenant::begin_bound(pool, org, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org)
        .bind(format!("Org {org}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn mint_token(pool: &sqlx::PgPool, tok_id: &str, org_id: &str, role: &str) -> Result<String> {
    let id_short = tok_id.strip_prefix("tok_").expect("tok_ prefix");
    let secret = format!("s{}", id_short);
    let hash = knievel::auth::opaque::hash(&secret)?;
    let mut tx = testlib::tenant::begin_bound(pool, org_id, None).await?;
    sqlx::query(
        "INSERT INTO knievel.api_tokens (id, org_id, scope, role, name, secret_hash)
         VALUES ($1, $2, 'org', $3, $4, $5)",
    )
    .bind(tok_id)
    .bind(org_id)
    .bind(role)
    .bind(format!("{role} fixture"))
    .bind(&hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(format!("kvl_test_org_{id_short}_{secret}"))
}

fn build_app(pool: sqlx::PgPool) -> impl poem::Endpoint {
    let state = knievel::state::AppState::new().with_db(pool);
    knievel::server::routes().data(state)
}

#[tokio::test]
async fn project_creation_seeds_default_taxonomy() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let f = setup().await?;
    let cli = TestClient::new(build_app(f.db.pool.clone()));

    // Create the project — handler seeds taxonomy in the same
    // transaction as the project insert.
    let resp = cli
        .post("/v1/orgs/org_a/projects")
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .body_json(&serde_json::json!({"name": "P"}))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let pj = body["id"].as_str().unwrap().to_string();

    // Channels: 3 seeded.
    let resp = cli
        .get(format!("/v1/projects/{pj}/channels"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Web"));
    assert!(names.contains(&"Mobile"));

    // Priorities: 3 seeded, ordered by tier ascending.
    let resp = cli
        .get(format!("/v1/projects/{pj}/priorities"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], serde_json::json!("House"));
    assert_eq!(items[0]["tier"].as_i64(), Some(1));

    // Ad types: 4 seeded, with width/height.
    let resp = cli
        .get(format!("/v1/projects/{pj}/ad-types"))
        .header("Authorization", format!("Bearer {}", f.org_a_admin))
        .send()
        .await;
    resp.assert_status_is_ok();
    let body: serde_json::Value = resp.json().await.value().deserialize();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Medium Rectangle"));

    // Cross-tenant: org_b admin can't read org_a's seeded
    // priorities (project_id mismatch resolves to wrong_tenant).
    let resp = cli
        .get(format!("/v1/projects/{pj}/priorities"))
        .header("Authorization", format!("Bearer {}", f.org_b_admin))
        .send()
        .await;
    resp.assert_status(poem::http::StatusCode::FORBIDDEN);

    testlib::db::ephemeral_drop(f.db).await?;
    Ok(())
}

/// Calling `seed_default_taxonomy` twice on the same project must
/// be idempotent — the second call should produce no duplicates
/// because all inserts use `ON CONFLICT (project_id, name) DO NOTHING`.
#[tokio::test]
async fn seed_default_taxonomy_is_idempotent() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_seed").await?;

    // Create a project directly via the pool so we can call
    // seed_default_taxonomy manually twice.
    let pj = "pj_seed_idem";
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_seed", None).await?;
        sqlx::query("INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)")
            .bind(pj)
            .bind("org_seed")
            .bind("Idem test project")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('knievel.project_id', $1, true)")
            .bind(pj)
            .execute(&mut *tx)
            .await?;
        // First seed.
        knievel::taxonomy::seed_default_taxonomy(&mut tx, "org_seed", pj).await?;
        // Second seed — must be a no-op due to ON CONFLICT DO NOTHING.
        knievel::taxonomy::seed_default_taxonomy(&mut tx, "org_seed", pj).await?;
        tx.commit().await?;
    }

    // Verify counts: exactly 3 channels, 3 priorities, 4 ad_types.
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_seed", Some(pj)).await?;
        let (channel_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM knievel.channels WHERE project_id = $1")
                .bind(pj)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(
            channel_count, 3,
            "expected exactly 3 channels after double-seed"
        );

        let (priority_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM knievel.priorities WHERE project_id = $1")
                .bind(pj)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(
            priority_count, 3,
            "expected exactly 3 priorities after double-seed"
        );

        let (ad_type_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM knievel.ad_types WHERE project_id = $1")
                .bind(pj)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(
            ad_type_count, 4,
            "expected exactly 4 ad_types after double-seed"
        );
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

/// Inserting a channel with a duplicate (project_id, name) must fail
/// with a unique_violation (SQLSTATE 23505).  The migration
/// 0015_taxonomy_unique_names.sql added this constraint.
#[tokio::test]
async fn channel_name_must_be_unique_per_project() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_uniq").await?;

    let pj = "pj_uniq_ch";
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_uniq", None).await?;
        sqlx::query("INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)")
            .bind(pj)
            .bind("org_uniq")
            .bind("Unique-name project")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('knievel.project_id', $1, true)")
            .bind(pj)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO knievel.channels (org_id, project_id, name) VALUES ($1, $2, 'Web')",
        )
        .bind("org_uniq")
        .bind(pj)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    // Second insert of the same (project_id, name) must be rejected.
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_uniq", Some(pj)).await?;
        let result = sqlx::query(
            "INSERT INTO knievel.channels (org_id, project_id, name) VALUES ($1, $2, 'Web')",
        )
        .bind("org_uniq")
        .bind(pj)
        .execute(&mut *tx)
        .await;
        let err = result.expect_err("expected unique_violation for duplicate channel name");
        let kind = knievel::sql::classify_pg_error(&err);
        assert!(
            kind.is_unique_violation(),
            "expected UniqueViolation, got {kind:?}"
        );
        let constraint = kind.constraint().unwrap_or("");
        assert!(
            constraint.contains("channels_project_id_name_key"),
            "unexpected constraint name: {constraint}"
        );
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

/// `seed_default_taxonomy` must raise an error when `knievel.project_id`
/// is not set on the transaction.  This guards against callers that
/// skip the tenant-binding step.
#[tokio::test]
async fn seed_default_taxonomy_requires_project_id_guc() -> Result<()> {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_guc").await?;

    let pj = "pj_guc_check";
    // Create the project (with proper binding), then commit so the row exists.
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_guc", None).await?;
        sqlx::query("INSERT INTO knievel.projects (id, org_id, name) VALUES ($1, $2, $3)")
            .bind(pj)
            .bind("org_guc")
            .bind("GUC check project")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }

    // Now open a tx with org_id but WITHOUT project_id — seed must fail.
    {
        let mut tx = testlib::tenant::begin_bound(&db.pool, "org_guc", None).await?;
        let result = knievel::taxonomy::seed_default_taxonomy(&mut tx, "org_guc", pj).await;
        assert!(
            result.is_err(),
            "expected seed_default_taxonomy to fail when knievel.project_id is unset"
        );
    }

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
