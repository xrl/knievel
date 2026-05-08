//! Integration tests for the foundation helpers (PR series:
//! `Foundation: shared helpers for the API resource fix campaign`).
//!
//! Covers:
//! - `crate::sql::classify_pg_error` — exercises the SQLSTATE
//!   23505 / 23503 paths against real Postgres errors via a
//!   tenant-bound transaction.
//! - `crate::handlers::open_org_tx` — exercises both the happy
//!   path and `wrong_tenant` rejection.
//! - `crate::audit::emit` — exercises the helper end-to-end and
//!   asserts the row lands with the expected payload_hash shape.
//! - `crate::batch::run_batch_with_savepoints` — exercises a
//!   mixed batch where one row collides on a unique constraint
//!   and the rest commit successfully.
//!
//! Skipped when `DATABASE_URL` is unset; CI runs against the
//! service container.

use anyhow::Result;
use knievel::auth::{Principal, Role, Scope, TokenType};
use knievel::sql::{classify_pg_error, PgErrorKind};
use sqlx::PgPool;
use testlib::tenant::begin_bound;

fn skip_if_no_db() -> bool {
    if std::env::var("DATABASE_URL").is_err() {
        eprintln!("DATABASE_URL not set; skipping.");
        return true;
    }
    false
}

async fn seed_org(pool: &PgPool, org_id: &str) -> Result<()> {
    let mut tx = begin_bound(pool, org_id, None).await?;
    sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ($1, $2)")
        .bind(org_id)
        .bind(format!("Org {}", org_id.to_uppercase()))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn fake_principal(org_id: &str, role: Role) -> Principal {
    Principal {
        token_type: TokenType::Opaque,
        scope: Scope::Org,
        org_id: org_id.into(),
        project_id: None,
        role,
        actor_id: format!("tok_test_{org_id}"),
    }
}

#[tokio::test]
async fn classify_pg_error_recognizes_unique_violation() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    // Insert the same org twice; the second one must trip the PK
    // unique violation. organizations.id is the PK so the
    // constraint name is `organizations_pkey` — not external_id —
    // confirming `is_external_id_conflict()` correctly says no.
    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    let res =
        sqlx::query("INSERT INTO knievel.organizations (id, name) VALUES ('org_a', 'duplicate')")
            .execute(&mut *tx)
            .await;
    let err = res.expect_err("second insert must fail with unique violation");
    let kind = classify_pg_error(&err);
    assert!(matches!(kind, PgErrorKind::UniqueViolation { .. }));
    assert!(kind.is_unique_violation());
    assert!(
        !kind.is_external_id_conflict(),
        "PK collision is NOT external_id_conflict (constraint = {:?})",
        kind.constraint()
    );
    let _ = tx.rollback().await;

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn classify_pg_error_recognizes_external_id_collision() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    // Two projects with the same external_id under one org → 23505
    // on the (org_id, external_id) unique constraint, whose name
    // contains `external_id`.
    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    sqlx::query(
        "INSERT INTO knievel.projects (id, org_id, external_id, name)
         VALUES ('pj_one', 'org_a', 'sharedExt', 'one')",
    )
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query(
        "INSERT INTO knievel.projects (id, org_id, external_id, name)
         VALUES ('pj_two', 'org_a', 'sharedExt', 'two')",
    )
    .execute(&mut *tx)
    .await;
    let err = res.expect_err("external_id collision must fail");
    let kind = classify_pg_error(&err);
    assert!(kind.is_unique_violation());
    assert!(
        kind.is_external_id_conflict(),
        "constraint name should contain 'external_id', got {:?}",
        kind.constraint()
    );
    let _ = tx.rollback().await;

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn open_org_tx_happy_path_binds_org_guc() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    let principal = fake_principal("org_a", Role::OrgAdmin);
    let mut tx = knievel::handlers::open_org_tx(&db.pool, &principal, "org_a", Role::OrgAdmin)
        .await
        .expect("open_org_tx must succeed for matching tenant + adequate role");
    // The GUC is bound — fetching it via current_setting echoes the
    // value the helper set.
    let bound: String = sqlx::query_scalar("SELECT current_setting('knievel.org_id', true)::text")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(bound, "org_a", "knievel.org_id GUC must be bound");
    let _ = tx.rollback().await;

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn open_org_tx_rejects_wrong_tenant() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;
    seed_org(&db.pool, "org_b").await?;

    let principal = fake_principal("org_a", Role::OrgAdmin);
    let res = knievel::handlers::open_org_tx(&db.pool, &principal, "org_b", Role::OrgAdmin).await;
    let err = res.expect_err("cross-tenant open_org_tx must fail");
    assert_eq!(err.code(), "wrong_tenant");

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn open_org_tx_rejects_role_below_minimum() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    let principal = fake_principal("org_a", Role::Reader);
    let res = knievel::handlers::open_org_tx(&db.pool, &principal, "org_a", Role::OrgAdmin).await;
    let err = res.expect_err("Reader cannot open org_tx at OrgAdmin minimum");
    assert_eq!(err.code(), "role_insufficient");

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn audit_emit_inserts_row_with_payload_hash() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    let principal = fake_principal("org_a", Role::OrgAdmin);
    let body = serde_json::json!({"name": "ci sync"});

    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    knievel::audit::emit(
        &mut tx,
        &principal,
        "tokens.mint",
        "token",
        "tok_abc123",
        Some(&body),
    )
    .await?;
    tx.commit().await?;

    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    let (operation, payload_hash, actor): (String, Option<String>, String) = sqlx::query_as(
        "SELECT operation, payload_hash, actor
           FROM knievel.audit_log
          WHERE operation = 'tokens.mint'",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    assert_eq!(operation, "tokens.mint");
    assert_eq!(actor, principal.actor_id);
    let want_hash = knievel::audit::hash_payload("token", "tok_abc123", &body)?;
    assert_eq!(
        payload_hash.as_deref(),
        Some(want_hash.as_str()),
        "payload_hash must be SHA-256 of canonical (kind, id, body)"
    );

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn audit_emit_with_none_payload_writes_null_hash() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    let principal = fake_principal("org_a", Role::OrgAdmin);

    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    knievel::audit::emit(
        &mut tx,
        &principal,
        "tokens.revoke",
        "token",
        "tok_abc123",
        None::<&serde_json::Value>,
    )
    .await?;
    tx.commit().await?;

    let mut tx = begin_bound(&db.pool, "org_a", None).await?;
    let payload_hash: Option<String> = sqlx::query_scalar(
        "SELECT payload_hash FROM knievel.audit_log
          WHERE operation = 'tokens.revoke'",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    assert!(
        payload_hash.is_none(),
        "payload_hash must be NULL when payload is None, got {payload_hash:?}"
    );

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}

#[tokio::test]
async fn run_batch_with_savepoints_isolates_row_failures() -> Result<()> {
    if skip_if_no_db() {
        return Ok(());
    }
    let db = testlib::db::ephemeral().await?;
    seed_org(&db.pool, "org_a").await?;

    // Seed a project to anchor advertiser inserts.
    {
        let mut tx = begin_bound(&db.pool, "org_a", None).await?;
        sqlx::query(
            "INSERT INTO knievel.projects (id, org_id, external_id, name)
             VALUES ('pj_a', 'org_a', 'pj-a-ext', 'A')",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    let mut tx = begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
    // Pre-seed one advertiser so the second batch row (same
    // external_id) collides on (project_id, external_id).
    sqlx::query(
        "INSERT INTO knievel.advertisers (org_id, project_id, external_id, name, is_active)
         VALUES ('org_a', 'pj_a', 'dup', 'pre', true)",
    )
    .execute(&mut *tx)
    .await?;

    // 4 rows: 0 ok, 1 collide, 2 ok, 3 ok. Without savepoints the
    // collision aborts the transaction and rows 2-3 fail too;
    // with savepoints, only row 1 errors.
    let rows = vec![
        ("ok-0", "first"),
        ("dup", "second"),
        ("ok-2", "third"),
        ("ok-3", "fourth"),
    ];
    let outcomes = knievel::batch::run_batch_with_savepoints(&mut tx, &rows, |tx, _idx, row| {
        let (ext, name) = *row;
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO knievel.advertisers (org_id, project_id, external_id, name, is_active)
                 VALUES ('org_a', 'pj_a', $1, $2, true)",
            )
            .bind(ext)
            .bind(name)
            .execute(&mut **tx)
            .await
            .map(|_| ())
        })
    })
    .await;
    tx.commit().await?;

    assert_eq!(outcomes.len(), 4);
    assert!(outcomes[0].is_ok(), "row 0 should succeed");
    assert!(outcomes[1].is_err(), "row 1 should collide on external_id");
    assert!(
        outcomes[2].is_ok(),
        "row 2 should succeed despite row 1 collision (savepoint isolation)"
    );
    assert!(outcomes[3].is_ok(), "row 3 should succeed");

    // The committed table reflects 4 rows total (the 1 pre-seed +
    // rows 0, 2, 3).
    let mut tx = begin_bound(&db.pool, "org_a", Some("pj_a")).await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM knievel.advertisers")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    assert_eq!(
        total, 4,
        "1 pre-seed + 3 successful batch rows = 4 advertisers"
    );

    testlib::db::ephemeral_drop(db).await?;
    Ok(())
}
