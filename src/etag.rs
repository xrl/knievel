//! `If-Match` precondition helper.
//!
//! Phase 6 known gap from CLAUDE.md: every PATCH handler bumps the
//! row's `etag`/`updated_at` but none of them read the `If-Match`
//! request header today, so optimistic concurrency on PATCH is
//! best-effort. This helper lands the substrate; per-module
//! agents wire it in front of each PATCH's UPDATE statement.
//!
//! Spec refs: `API.md` "Headers" → `If-Match`, RFC 7232 § 3.1
//! (server-side enforcement is opt-in for v0).
//!
//! Behavior:
//! - Header absent → `Ok(())`. Per CLAUDE.md the v0 contract is
//!   "PATCH bumps etag; If-Match enforcement is opt-in," so a
//!   missing header is not an error.
//! - Header present and matches → `Ok(())`.
//! - Header present and differs → `Err(IfMatchError::Mismatch)`,
//!   which the caller maps to RFC 9457 `precondition_failed` 412.
//! - Wildcard `*` matches any current etag (RFC 7232 § 3.1).
//! - Multiple comma-separated etags in the header all eligible
//!   per RFC 7232; any match is sufficient.
//! - Quoted ("strong") and unquoted forms are both accepted —
//!   knievel etags are opaque hex strings without the quoting
//!   wrapper, but RFC 7232 says quoted is canonical, so callers
//!   that quote pass too.

use poem::http::{header, HeaderMap};

/// Result of an `If-Match` precondition check that the caller
/// should surface as a `412 precondition_failed` envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfMatchError {
    /// Header was present but no listed etag matched the row's
    /// current etag. Caller returns `412`.
    Mismatch,
}

impl IfMatchError {
    pub fn code(self) -> &'static str {
        "precondition_failed"
    }
    pub fn message(self) -> &'static str {
        "If-Match header does not match the current resource etag"
    }
}

/// Read `If-Match` from a `HeaderMap` and compare against the
/// row's current etag. Returns `Ok(())` when absent or matching;
/// `Err(IfMatchError::Mismatch)` when the header is present but
/// no listed etag matches (caller maps to 412).
pub fn check_if_match(headers: &HeaderMap, current_etag: &str) -> Result<(), IfMatchError> {
    let Some(raw) = headers.get(header::IF_MATCH) else {
        return Ok(());
    };
    let Ok(value) = raw.to_str() else {
        // Non-ASCII If-Match bytes — treat as mismatch (would be
        // suspect anyway since etags are hex).
        return Err(IfMatchError::Mismatch);
    };
    check_if_match_value(Some(value), current_etag)
}

/// Compare a raw `If-Match` value (possibly absent) against the
/// row's current etag. Used by poem-openapi handlers that extract
/// `If-Match` as `Header<Option<String>>` instead of receiving a
/// raw `HeaderMap`.
pub fn check_if_match_value(value: Option<&str>, current_etag: &str) -> Result<(), IfMatchError> {
    let Some(value) = value else {
        return Ok(());
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    // RFC 7232 § 3.1: `*` matches any existing representation.
    if trimmed == "*" {
        return Ok(());
    }
    for tag in trimmed.split(',') {
        let candidate = tag.trim().trim_matches('"');
        if candidate == current_etag {
            return Ok(());
        }
    }
    Err(IfMatchError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poem::http::{HeaderMap, HeaderValue};

    #[test]
    fn absent_header_passes() {
        let headers = HeaderMap::new();
        assert!(check_if_match(&headers, "abc123").is_ok());
        assert!(check_if_match_value(None, "abc123").is_ok());
        assert!(check_if_match_value(Some(""), "abc123").is_ok());
    }

    #[test]
    fn matching_etag_passes() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("abc123"));
        assert!(check_if_match(&headers, "abc123").is_ok());
    }

    #[test]
    fn mismatched_etag_fails() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("abc123"));
        assert_eq!(
            check_if_match(&headers, "def456"),
            Err(IfMatchError::Mismatch)
        );
    }

    #[test]
    fn wildcard_matches_any_etag() {
        assert!(check_if_match_value(Some("*"), "abc123").is_ok());
        assert!(check_if_match_value(Some("*"), "anything").is_ok());
    }

    #[test]
    fn quoted_form_is_accepted() {
        // RFC 7232 specifies quoted as canonical; knievel emits
        // unquoted hex but accepts either on the way in.
        assert!(check_if_match_value(Some("\"abc123\""), "abc123").is_ok());
    }

    #[test]
    fn comma_separated_list_matches_any() {
        // RFC 7232 § 3.1: "If-Match: \"x\", \"y\""
        assert!(check_if_match_value(Some("\"old\", \"abc123\""), "abc123").is_ok());
        assert_eq!(
            check_if_match_value(Some("\"old\", \"older\""), "abc123"),
            Err(IfMatchError::Mismatch)
        );
    }

    #[test]
    fn error_carries_public_code_and_message() {
        let e = IfMatchError::Mismatch;
        assert_eq!(e.code(), "precondition_failed");
        assert!(e.message().contains("If-Match"));
    }
}
