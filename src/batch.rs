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
/// `details[].code` enum from `API.md`.
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
