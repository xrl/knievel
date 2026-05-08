//! Shared batch upsert infrastructure.
//!
//! Phase 3.14. Every project-scoped resource that declares
//! `:batchUpsert` (advertisers, campaigns, flights, ads, sites,
//! zones) shares this error envelope and the per-row diagnostic
//! shape from `API.md` "Write contract."
//!
//! Each batch handler runs in exactly one Postgres transaction.
//! If any row fails validation, the transaction rolls back and the
//! handler returns `422 batch_partial_failure` with a
//! deterministic `details[]` listing every offending row by index.
//! Successful batches commit and return `200` with the upserted
//! rows.
//!
//! Spec refs: `API.md` "Write contract" (§§ "Idempotency"–end of
//! conventions block), `TESTING.md` § 6.4 (batch contract).

use poem_openapi::Object;
use sqlx::{Postgres, Transaction};
use std::future::Future;

/// One per-row failure within a `:batchUpsert`. `index` is the
/// position of the offending row in the request array (0-based).
/// `code` is one of: `fk_not_found`, `external_id_conflict`,
/// `validation_failed`, `unique_violation`, `if_match_mismatch`.
#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchErrorDetail {
    pub index: i32,
    pub field: Option<String>,
    pub code: String,
    pub message: String,
}

/// Body of a `batch_partial_failure` envelope. Mirrors
/// `API.md` "Write contract" exactly so a future code-gen
/// consumer can deserialize this in any language.
#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchErrorBody {
    pub code: String,
    pub message: String,
    pub details: Vec<BatchErrorDetail>,
}

#[derive(Object, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchErrorEnvelope {
    pub error: BatchErrorBody,
}

impl BatchErrorEnvelope {
    pub fn partial_failure(total: usize, details: Vec<BatchErrorDetail>) -> Self {
        let n = details.len();
        Self {
            error: BatchErrorBody {
                code: "batch_partial_failure".into(),
                message: format!("{n} of {total} rows failed validation"),
                details,
            },
        }
    }
}

/// Classify a Postgres error string into the canonical
/// `details[].code` enum from `API.md`. **Deprecated** — substring
/// matching is fragile across Postgres versions and locales. New
/// code should call `crate::sql::classify_pg_error(&sqlx_err)` and
/// invoke `.as_batch_detail()` on the returned `PgErrorKind`. This
/// shim is kept for `:batchUpsert` callers that haven't migrated
/// yet (Phase 3.14 macro extraction will close the gap).
pub fn classify_pg_error(msg: &str) -> (&'static str, Option<&'static str>) {
    if msg.contains("foreign key") {
        ("fk_not_found", Some("foreign key reference does not exist"))
    } else if msg.contains("duplicate key") || msg.contains("unique constraint") {
        (
            "unique_violation",
            Some("unique constraint violated for this row"),
        )
    } else {
        ("validation_failed", None)
    }
}

/// Run a per-row closure under its own SAVEPOINT so a row failure
/// rolls back only that row and the outer transaction can keep
/// going. This is the substrate for the per-row diagnostic shape
/// `API.md` "Write contract" mandates: every offending row gets a
/// `details[]` entry, callers don't short-circuit on the first
/// error.
///
/// The closure is invoked exactly once per input row with a
/// mutable borrow of the outer transaction. Inside the closure,
/// callers run their own `INSERT ... RETURNING` (or whatever the
/// per-row work is) and return `Result<T, sqlx::Error>`. If the
/// closure errors, this helper rolls back to the row's savepoint
/// — the transaction stays alive for the next row.
///
/// Module agents migrate their `:batchUpsert` handlers off the
/// `break;`-on-first-error pattern by wrapping their per-row
/// SQL in this helper and accumulating
/// `Vec<Result<T, sqlx::Error>>` into a `BatchErrorEnvelope`.
///
/// Returns a vec the same length as `rows` whose i-th entry is
/// the i-th row's outcome.
pub async fn run_batch_with_savepoints<R, T, F>(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[R],
    mut per_row: F,
) -> Vec<Result<T, sqlx::Error>>
where
    F: for<'a, 'b> FnMut(
        &'a mut Transaction<'b, Postgres>,
        usize,
        &'a R,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'a>,
    >,
{
    let mut out: Vec<Result<T, sqlx::Error>> = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        // Savepoint names must be valid SQL identifiers; the index
        // is the only varying part so we render it directly. No
        // attacker-controlled input lands here.
        let sp = format!("knievel_batch_row_{idx}");
        if let Err(e) = sqlx::query(&format!("SAVEPOINT {sp}"))
            .execute(&mut **tx)
            .await
        {
            // The savepoint itself failed — bubble that out as the
            // row's error. The outer tx is in trouble; subsequent
            // rows will likely also fail but we keep going so
            // callers can report what they got.
            out.push(Err(e));
            continue;
        }

        let row_result = per_row(tx, idx, row).await;

        match &row_result {
            Ok(_) => {
                if let Err(e) = sqlx::query(&format!("RELEASE SAVEPOINT {sp}"))
                    .execute(&mut **tx)
                    .await
                {
                    out.push(Err(e));
                    continue;
                }
            }
            Err(_) => {
                // Postgres has marked the tx (or this savepoint
                // sub-tx) as aborted; ROLLBACK TO SAVEPOINT
                // restores the outer tx to a non-aborted state
                // ready for the next row.
                if let Err(e) = sqlx::query(&format!("ROLLBACK TO SAVEPOINT {sp}"))
                    .execute(&mut **tx)
                    .await
                {
                    out.push(Err(e));
                    continue;
                }
                if let Err(e) = sqlx::query(&format!("RELEASE SAVEPOINT {sp}"))
                    .execute(&mut **tx)
                    .await
                {
                    out.push(Err(e));
                    continue;
                }
            }
        }
        out.push(row_result);
    }
    out
}
