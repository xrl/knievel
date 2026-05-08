//! `audit_log` write helper.
//!
//! Phase 3.4 created the `audit_log` table; today only
//! `tokens.rs` writes to it ad-hoc, and the two existing call
//! sites disagree on `payload_hash` shape: mint correctly
//! SHA-256s its canonical body while revoke binds the raw
//! token id. This helper centralizes the shape so every future
//! audit emission is symmetric.
//!
//! Spec refs:
//!   - `REQUIREMENTS.md` § 7.3 (audit_log schema, append-only,
//!     per-tenant via RLS).
//!   - `AUTH.md` "Audit and observability".
//!
//! `payload_hash` is the SHA-256 hex digest of the canonical
//! `serde_json::to_vec(payload)` when `payload` is `Some`, and
//! `NULL` when `None`. The same canonical-JSON convention as
//! `idempotency::body_hash` so future tooling that joins the two
//! tables can compare hashes directly.

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use crate::auth::Principal;

/// Emit one `audit_log` row inside the caller's transaction. The
/// caller owns the tx (they're typically committing it together
/// with the data mutation that earned the audit row), so this
/// helper never commits or rolls back — it just inserts.
///
/// `payload` is hashed and discarded; the request body itself
/// never lands in audit_log. Pass `None` when there is no
/// caller-supplied body to hash (e.g. revoke takes a path-only
/// id; the actor + operation columns already pin the audit
/// shape).
///
/// `resource_kind` and `resource_id` are reserved for future
/// schema additions — today they're folded into the payload hash
/// alongside whatever else the caller passes, so callers can
/// audit-trace a specific row by SHA-ing `(kind, id, body)`
/// off-line and matching the digest. When the columns land in a
/// future migration, this helper picks them up without changing
/// its public signature.
pub async fn emit<S: Serialize>(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Principal,
    action: &str,
    resource_kind: &str,
    resource_id: &str,
    payload: Option<&S>,
) -> Result<()> {
    let payload_hash = match payload {
        Some(body) => Some(hash_payload(resource_kind, resource_id, body)?),
        None => None,
    };
    sqlx::query(
        "INSERT INTO knievel.audit_log
            (org_id, project_id, actor, operation, payload_hash)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&actor.org_id)
    .bind(actor.project_id.as_deref())
    .bind(&actor.actor_id)
    .bind(action)
    .bind(payload_hash)
    .execute(&mut **tx)
    .await
    .context("audit_log insert")?;
    Ok(())
}

/// Hex SHA-256 of canonical `(kind, id, body)`. Exposed for
/// callers that want to compute the hash without touching the DB
/// (tests, off-line audit replay).
pub fn hash_payload<S: Serialize>(
    resource_kind: &str,
    resource_id: &str,
    payload: &S,
) -> Result<String> {
    let body = serde_json::to_vec(payload).context("serializing audit payload")?;
    let mut h = Sha256::new();
    h.update(resource_kind.as_bytes());
    h.update(b"\0");
    h.update(resource_id.as_bytes());
    h.update(b"\0");
    h.update(&body);
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Body {
        a: u32,
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = hash_payload("token", "tok_abc", &Body { a: 1 }).unwrap();
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_distinguishes_kind() {
        let a = hash_payload("token", "id", &Body { a: 1 }).unwrap();
        let b = hash_payload("project", "id", &Body { a: 1 }).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn hash_distinguishes_id() {
        let a = hash_payload("token", "tok_a", &Body { a: 1 }).unwrap();
        let b = hash_payload("token", "tok_b", &Body { a: 1 }).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn hash_distinguishes_payload() {
        let a = hash_payload("token", "id", &Body { a: 1 }).unwrap();
        let b = hash_payload("token", "id", &Body { a: 2 }).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_deterministic() {
        let a = hash_payload("token", "id", &Body { a: 7 }).unwrap();
        let b = hash_payload("token", "id", &Body { a: 7 }).unwrap();
        assert_eq!(a, b);
    }
}
