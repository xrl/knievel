//! Shared SQL error classification.
//!
//! Centralizes Postgres error decoding so handlers don't have to
//! substring-match `format!("{e}")` to tell a `23505` collision
//! from a `23503` foreign-key miss. Substring matching breaks
//! across Postgres versions, locales, and constraint names — the
//! API audit (`#5` sonnet, `#7` opus, both cross-cutting #1)
//! flagged this in 11 CRUD modules.
//!
//! `classify_pg_error` looks at SQLSTATE first (`23505`, `23503`,
//! `23514`) and the `constraint` name second so callers can tell
//! `external_id` collisions apart from PK collisions or FK misses
//! on a particular column.
//!
//! The previous tuple-returning helper at `crate::batch` is kept
//! as a thin wrapper over this module for `:batchUpsert` callers
//! that haven't been migrated yet.

use sqlx::Error as SqlxError;

/// Coarse Postgres error class. `Other` is any error sqlx surfaces
/// without a database error attached, or one whose SQLSTATE class
/// isn't an integrity violation we know how to tell apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgErrorKind {
    /// SQLSTATE `23505` — unique_violation.
    UniqueViolation { constraint: Option<String> },
    /// SQLSTATE `23503` — foreign_key_violation.
    ForeignKeyViolation { constraint: Option<String> },
    /// SQLSTATE `23514` — check_violation.
    CheckViolation { constraint: Option<String> },
    /// SQLSTATE `23502` — not_null_violation. The column name is
    /// not exposed on sqlx's portable trait object today; callers
    /// who need it should downcast to `PgDatabaseError`.
    NotNullViolation,
    /// Any other sqlx error.
    Other,
}

impl PgErrorKind {
    /// `true` for kinds that map to the API.md `external_id_conflict`
    /// 409 response IFF the constraint name names `external_id`.
    /// Callers that don't care about the constraint name should
    /// treat any unique_violation as a conflict and use
    /// `is_unique_violation()` instead.
    pub fn is_external_id_conflict(&self) -> bool {
        match self {
            PgErrorKind::UniqueViolation {
                constraint: Some(c),
            } => c.contains("external_id"),
            _ => false,
        }
    }

    pub fn is_unique_violation(&self) -> bool {
        matches!(self, PgErrorKind::UniqueViolation { .. })
    }

    pub fn is_fk_violation(&self) -> bool {
        matches!(self, PgErrorKind::ForeignKeyViolation { .. })
    }

    pub fn is_check_violation(&self) -> bool {
        matches!(self, PgErrorKind::CheckViolation { .. })
    }

    /// The constraint name reported by Postgres, if any. Some
    /// errors (e.g. PK collisions on tables with a system-named
    /// PK) carry only the constraint, not the column — that's the
    /// signal callers use to tell `external_id` collisions apart
    /// from PK collisions.
    pub fn constraint(&self) -> Option<&str> {
        match self {
            PgErrorKind::UniqueViolation {
                constraint: Some(c),
            }
            | PgErrorKind::ForeignKeyViolation {
                constraint: Some(c),
            }
            | PgErrorKind::CheckViolation {
                constraint: Some(c),
            } => Some(c.as_str()),
            _ => None,
        }
    }

    /// Tuple form matching the legacy `crate::batch::classify_pg_error`
    /// return shape, used by `:batchUpsert` callers until they
    /// migrate to the structured enum. Returns
    /// `(API.md details[].code, default human message)`.
    pub fn as_batch_detail(&self) -> (&'static str, Option<&'static str>) {
        match self {
            PgErrorKind::ForeignKeyViolation { .. } => {
                ("fk_not_found", Some("foreign key reference does not exist"))
            }
            PgErrorKind::UniqueViolation { .. } => (
                "unique_violation",
                Some("unique constraint violated for this row"),
            ),
            PgErrorKind::CheckViolation { .. } => (
                "validation_failed",
                Some("check constraint violated for this row"),
            ),
            PgErrorKind::NotNullViolation => {
                ("validation_failed", Some("required field is missing"))
            }
            PgErrorKind::Other => ("validation_failed", None),
        }
    }
}

/// Classify an `sqlx::Error` by SQLSTATE. Falls through to
/// `PgErrorKind::Other` when no database error is attached (e.g.
/// connection-level failures, decoding errors).
pub fn classify_pg_error(err: &SqlxError) -> PgErrorKind {
    let Some(db_err) = err.as_database_error() else {
        return PgErrorKind::Other;
    };
    // SQLSTATE is a five-character class+condition string per the
    // Postgres docs. We compare by exact match — class `23` covers
    // every integrity violation (`23xxx`), but each condition has
    // a different shape we want to surface separately.
    let code = match db_err.code() {
        Some(c) => c,
        None => return PgErrorKind::Other,
    };
    let constraint = db_err.constraint().map(str::to_string);
    match code.as_ref() {
        "23505" => PgErrorKind::UniqueViolation { constraint },
        "23503" => PgErrorKind::ForeignKeyViolation { constraint },
        "23514" => PgErrorKind::CheckViolation { constraint },
        "23502" => PgErrorKind::NotNullViolation,
        _ => PgErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_has_no_constraint() {
        let k = PgErrorKind::Other;
        assert_eq!(k.constraint(), None);
        assert!(!k.is_unique_violation());
        assert!(!k.is_fk_violation());
        assert!(!k.is_external_id_conflict());
    }

    #[test]
    fn external_id_constraint_is_recognized() {
        let k = PgErrorKind::UniqueViolation {
            constraint: Some("advertisers_project_id_external_id_key".into()),
        };
        assert!(
            k.is_external_id_conflict(),
            "any unique constraint name containing 'external_id' is treated as the public 409"
        );
        assert!(k.is_unique_violation());
    }

    #[test]
    fn pk_unique_violation_is_not_external_id_conflict() {
        // Postgres' default PK constraint name is `<table>_pkey` —
        // no 'external_id' substring. Surfacing it as 409 would
        // leak internal-id collisions to the caller; the create
        // path should retry / 500 instead.
        let k = PgErrorKind::UniqueViolation {
            constraint: Some("projects_pkey".into()),
        };
        assert!(!k.is_external_id_conflict());
        assert!(k.is_unique_violation());
    }

    #[test]
    fn unique_violation_without_constraint_is_not_external_id_conflict() {
        let k = PgErrorKind::UniqueViolation { constraint: None };
        assert!(!k.is_external_id_conflict());
        assert!(k.is_unique_violation());
    }

    #[test]
    fn batch_detail_shape_matches_legacy() {
        // Legacy `batch::classify_pg_error` returned these strings
        // verbatim — keep them stable for callers that haven't
        // migrated to the structured enum yet.
        assert_eq!(
            PgErrorKind::ForeignKeyViolation { constraint: None }.as_batch_detail(),
            ("fk_not_found", Some("foreign key reference does not exist"))
        );
        assert_eq!(
            PgErrorKind::UniqueViolation { constraint: None }.as_batch_detail(),
            (
                "unique_violation",
                Some("unique constraint violated for this row")
            )
        );
        assert_eq!(
            PgErrorKind::Other.as_batch_detail(),
            ("validation_failed", None)
        );
    }
}
