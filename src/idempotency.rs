//! `Idempotency-Key` replay store.
//!
//! Phase 3.5. Spec refs:
//!   - `API.md` "Idempotency" (24 h replay window, `(project, key,
//!     route, body-hash)` keying, `Idempotent-Replay: true` on
//!     replay, `409 idempotency_conflict` on body mismatch)
//!   - `TESTING.md` § 6.4 (`create_idempotency_key_replay`,
//!     `create_idempotency_key_mismatch_body`)
//!
//! Body hashing canonicalizes via `serde_json::to_vec`, which
//! produces compact output without whitespace. Caller-side
//! whitespace differences therefore produce identical hashes;
//! field-order differences (rare for `serde`-emitted JSON) do not.
//! Full canonical form (recursive key sort) is a follow-up.
//!
//! Cleanup of expired rows is the leader's job (Phase 3.22).

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

pub const HEADER: &str = "Idempotency-Key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// No prior row for this `(tenant, key, route)`. Caller proceeds
    /// with the real handler and stores the response on success.
    Fresh,
    /// Prior row found; body hashes match. Caller returns the
    /// stored response with `Idempotent-Replay: true`.
    Replay { status: u16, body: Vec<u8> },
    /// Prior row found; body hash differs. Caller returns
    /// `409 idempotency_conflict`.
    Conflict,
}

/// Compute the canonical body hash for an idempotency lookup. The
/// body is serialized via `serde_json::to_vec` (no whitespace,
/// stable for the type's `Serialize` impl) and SHA-256'd. Hex-
/// encoded for storage.
pub fn body_hash<T: Serialize>(body: &T) -> Result<String> {
    let canonical = serde_json::to_vec(body)?;
    let digest = Sha256::digest(&canonical);
    Ok(hex::encode(digest))
}

/// Look up an existing idempotency row. Caller has already opened
/// a tenant-bound transaction (`db::begin_bound`); RLS scopes the
/// query to the bound tenant.
pub async fn check(
    tx: &mut Transaction<'_, Postgres>,
    org_id: &str,
    project_id: Option<&str>,
    key: &str,
    route: &str,
    body_hash: &str,
) -> Result<CheckResult> {
    let row: Option<(String, i32, Vec<u8>)> = sqlx::query_as(
        "SELECT body_hash, response_status, response_body
         FROM knievel.idempotency_keys
         WHERE org_id = $1
           AND coalesce(project_id, '') = coalesce($2, '')
           AND key = $3
           AND route = $4
           AND expires_at > now()",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(key)
    .bind(route)
    .fetch_optional(&mut **tx)
    .await?;

    match row {
        None => Ok(CheckResult::Fresh),
        Some((stored_hash, status, body)) => {
            if stored_hash == body_hash {
                Ok(CheckResult::Replay {
                    status: status as u16,
                    body,
                })
            } else {
                Ok(CheckResult::Conflict)
            }
        }
    }
}

/// Insert a fresh idempotency row. `ON CONFLICT DO NOTHING` so a
/// concurrent successful replay (rare race) wins quietly.
///
/// The wide arg list mirrors the lookup tuple — packing into a
/// struct would just shift the verbosity to the caller, since each
/// field comes from a different source (path, header, request
/// body, response).
#[allow(clippy::too_many_arguments)]
pub async fn store(
    tx: &mut Transaction<'_, Postgres>,
    org_id: &str,
    project_id: Option<&str>,
    key: &str,
    route: &str,
    body_hash: &str,
    status: u16,
    body: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO knievel.idempotency_keys
            (org_id, project_id, key, route, body_hash,
             response_status, response_body)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT DO NOTHING",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(key)
    .bind(route)
    .bind(body_hash)
    .bind(status as i32)
    .bind(body)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Body {
        a: u32,
        b: String,
    }

    #[test]
    fn body_hash_is_stable_across_whitespace() {
        // Two semantically-identical inputs that differ only in
        // whitespace produce the same hash, because we serialize
        // through `serde_json::to_vec` rather than hashing raw
        // bytes.
        let h1 = body_hash(&serde_json::json!({"a": 1, "b": "x"})).unwrap();
        let raw_with_spaces = "{ \"a\":1 , \"b\":\"x\" }";
        let parsed: serde_json::Value = serde_json::from_str(raw_with_spaces).unwrap();
        let h2 = body_hash(&parsed).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn body_hash_is_64_hex_chars() {
        let h = body_hash(&serde_json::json!({})).unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn body_hash_distinguishes_different_bodies() {
        let h1 = body_hash(&Body {
            a: 1,
            b: "x".into(),
        })
        .unwrap();
        let h2 = body_hash(&Body {
            a: 2,
            b: "x".into(),
        })
        .unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_object_hashes_consistently() {
        let h1 = body_hash(&serde_json::json!({})).unwrap();
        let h2 = body_hash(&serde_json::json!({})).unwrap();
        assert_eq!(h1, h2);
    }
}
