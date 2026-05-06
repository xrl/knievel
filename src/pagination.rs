//! Cursor-based list pagination per `API.md` § "Pagination."
//!
//! Wire format: `?limit=N&cursor=<opaque>`. Default limit 50,
//! hard cap 500. The cursor encodes `(kind, last_id)` as
//! `base64url(JSON)` — server validates the cursor's `kind`
//! matches the endpoint receiving it so a cursor minted by
//! `listAdvertisers` can't be replayed against `listCampaigns`
//! (returns `400 invalid_cursor`).
//!
//! Sort key is the bigserial `id` column on every paginated
//! resource, descending — "newest first" matches the existing
//! list-handler behavior. `WHERE id < cursor.last_id` resumes
//! from where the prior page left off; the `LIMIT N+1` peek
//! detects whether more pages exist without a separate COUNT.
//!
//! Filter changes between pages are the caller's responsibility —
//! the cursor only carries `last_id`, so mixing it with a
//! different filter set may skip rows or return duplicates.
//! Documented in `API.md`.

use base64::Engine;
use serde::{Deserialize, Serialize};

pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 500;

#[derive(Debug, Serialize, Deserialize)]
struct CursorState {
    /// Resource literal — server validates this matches the
    /// endpoint to catch cursor-cross-resource replay.
    kind: String,
    /// Last seen primary-key id from the prior page. Next page
    /// resumes with `WHERE id < last_id ORDER BY id DESC`.
    last_id: i64,
}

#[derive(Debug)]
pub enum PaginationError {
    InvalidLimit(String),
    InvalidCursor(String),
}

impl PaginationError {
    pub fn code(&self) -> &'static str {
        match self {
            PaginationError::InvalidLimit(_) => "invalid_limit",
            PaginationError::InvalidCursor(_) => "invalid_cursor",
        }
    }
    pub fn message(&self) -> &str {
        match self {
            PaginationError::InvalidLimit(s) | PaginationError::InvalidCursor(s) => s,
        }
    }
}

/// Resolve `(after_id, effective_limit, bumped_limit)` from
/// caller-supplied `limit` and `cursor` query params.
/// `bumped_limit` is the value to pass to SQL `LIMIT` —
/// `effective_limit + 1` so a single query reveals "there's at
/// least one more row past this page."
///
/// Pass `None` for `cursor` on the first page.
pub fn resolve(
    limit: Option<i64>,
    cursor: Option<&str>,
    expected_kind: &str,
) -> Result<Resolved, PaginationError> {
    let effective_limit = match limit {
        None => DEFAULT_LIMIT,
        Some(n) if n < 1 => {
            return Err(PaginationError::InvalidLimit("limit must be >= 1".into()));
        }
        Some(n) if n > MAX_LIMIT => {
            return Err(PaginationError::InvalidLimit(format!(
                "limit must be <= {MAX_LIMIT}"
            )));
        }
        Some(n) => n,
    };

    let after_id = match cursor {
        None => None,
        Some(s) => Some(decode(s, expected_kind)?.last_id),
    };

    Ok(Resolved {
        after_id,
        effective_limit,
        bumped_limit: effective_limit + 1,
    })
}

#[derive(Debug)]
pub struct Resolved {
    /// `id` of the last row on the previous page, if any.
    /// SQL: `WHERE id < $after_id` when `Some`.
    pub after_id: Option<i64>,
    /// Caller-requested page size after defaulting + bounds-check.
    /// Use this to truncate the rows returned.
    pub effective_limit: i64,
    /// `effective_limit + 1` — value to pass to SQL `LIMIT` so
    /// the "is there another page?" peek works in one query.
    pub bumped_limit: i64,
}

/// Build the next-page cursor from a freshly-fetched page. Pass
/// the rows returned from SQL (including the `+1` peek row) and
/// a closure that extracts the row's `id`. Returns `None` when
/// there's no next page (`rows.len() <= effective_limit`).
///
/// Callers should also truncate `rows` to `effective_limit`
/// before returning to the wire — this function only computes
/// the cursor; it does not mutate the slice.
pub fn next_cursor<T>(
    rows: &[T],
    resolved: &Resolved,
    expected_kind: &str,
    last_id_of: impl Fn(&T) -> i64,
) -> Option<String> {
    if (rows.len() as i64) <= resolved.effective_limit {
        return None;
    }
    let last = &rows[(resolved.effective_limit - 1) as usize];
    Some(encode(&CursorState {
        kind: expected_kind.into(),
        last_id: last_id_of(last),
    }))
}

fn encode(state: &CursorState) -> String {
    let json = serde_json::to_vec(state).expect("CursorState always serializes");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

fn decode(raw: &str, expected_kind: &str) -> Result<CursorState, PaginationError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw.as_bytes())
        .map_err(|_| PaginationError::InvalidCursor("not valid base64url".into()))?;
    let state: CursorState = serde_json::from_slice(&bytes)
        .map_err(|_| PaginationError::InvalidCursor("not a knievel cursor".into()))?;
    if state.kind != expected_kind {
        return Err(PaginationError::InvalidCursor(format!(
            "cursor kind {:?} does not match endpoint {:?}",
            state.kind, expected_kind
        )));
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_cursor() {
        let s = encode(&CursorState {
            kind: "advertisers".into(),
            last_id: 12345,
        });
        let back = decode(&s, "advertisers").unwrap();
        assert_eq!(back.last_id, 12345);
        assert_eq!(back.kind, "advertisers");
    }

    #[test]
    fn cursor_kind_mismatch_rejected() {
        let s = encode(&CursorState {
            kind: "advertisers".into(),
            last_id: 99,
        });
        let e = decode(&s, "campaigns").unwrap_err();
        assert_eq!(e.code(), "invalid_cursor");
        assert!(e.message().contains("advertisers"));
        assert!(e.message().contains("campaigns"));
    }

    #[test]
    fn corrupt_cursor_rejected() {
        let e = decode("not-base64-!!!", "advertisers").unwrap_err();
        assert_eq!(e.code(), "invalid_cursor");
    }

    #[test]
    fn cursor_payload_shape_rejected() {
        // Valid base64url of "hello" — not JSON, not a cursor.
        let bad = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello");
        let e = decode(&bad, "advertisers").unwrap_err();
        assert_eq!(e.code(), "invalid_cursor");
    }

    #[test]
    fn limit_defaults() {
        let r = resolve(None, None, "advertisers").unwrap();
        assert_eq!(r.effective_limit, DEFAULT_LIMIT);
        assert_eq!(r.bumped_limit, DEFAULT_LIMIT + 1);
        assert!(r.after_id.is_none());
    }

    #[test]
    fn limit_zero_rejected() {
        let e = resolve(Some(0), None, "advertisers").unwrap_err();
        assert_eq!(e.code(), "invalid_limit");
    }

    #[test]
    fn limit_negative_rejected() {
        let e = resolve(Some(-1), None, "advertisers").unwrap_err();
        assert_eq!(e.code(), "invalid_limit");
    }

    #[test]
    fn limit_overcap_rejected() {
        let e = resolve(Some(MAX_LIMIT + 1), None, "advertisers").unwrap_err();
        assert_eq!(e.code(), "invalid_limit");
    }

    #[test]
    fn limit_at_cap_accepted() {
        let r = resolve(Some(MAX_LIMIT), None, "advertisers").unwrap();
        assert_eq!(r.effective_limit, MAX_LIMIT);
    }

    #[test]
    fn cursor_round_trip_through_resolve() {
        let c = encode(&CursorState {
            kind: "advertisers".into(),
            last_id: 42,
        });
        let r = resolve(Some(10), Some(&c), "advertisers").unwrap();
        assert_eq!(r.after_id, Some(42));
        assert_eq!(r.effective_limit, 10);
    }

    #[test]
    fn next_cursor_when_more_rows() {
        let rows = vec![10_i64, 9, 8, 7, 6, 5]; // 6 rows, effective limit 5 → more pages
        let r = Resolved {
            after_id: None,
            effective_limit: 5,
            bumped_limit: 6,
        };
        let c = next_cursor(&rows, &r, "advertisers", |x| *x).expect("more pages");
        let decoded = decode(&c, "advertisers").unwrap();
        assert_eq!(decoded.last_id, 6);
    }

    #[test]
    fn next_cursor_when_exact_page() {
        let rows = vec![10_i64, 9, 8, 7, 6]; // 5 rows == limit → no more
        let r = Resolved {
            after_id: None,
            effective_limit: 5,
            bumped_limit: 6,
        };
        assert!(next_cursor(&rows, &r, "advertisers", |x| *x).is_none());
    }

    #[test]
    fn next_cursor_when_partial_page() {
        let rows = vec![10_i64, 9, 8]; // 3 rows < limit → no more
        let r = Resolved {
            after_id: None,
            effective_limit: 5,
            bumped_limit: 6,
        };
        assert!(next_cursor(&rows, &r, "advertisers", |x| *x).is_none());
    }
}
