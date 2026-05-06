//! Opaque-token format and hashing.
//!
//! The wire format is `kvl_<env>_<scope>_<id_short>_<secret>`
//! (`REQUIREMENTS.md` § 4.3). `id_short` is the `id` segment that
//! routes the lookup against `api_tokens` — by convention the
//! stored `api_tokens.id` is `tok_<id_short>`. The trailing
//! `secret` segment is verified against `api_tokens.secret_hash`
//! (argon2id). The plaintext secret is never persisted — only the
//! salted hash.
//!
//! Spec refs:
//!   - `REQUIREMENTS.md` § 4.3
//!   - `AUTH.md` "Opaque Tokens"
//!   - `TESTING.md` § 4.1 (`auth::opaque::*` unit tests)

use anyhow::{anyhow, bail, Result};
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

const PREFIX: &str = "kvl_";
pub const ID_DB_PREFIX: &str = "tok_";

/// A parsed but not-yet-verified opaque token. Borrowing keeps the
/// hot path allocation-free; copy the `id`/`secret` strings into
/// owned `String`s only when needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOpaque<'a> {
    pub env: &'a str,
    /// "org" or "project" — validated at parse time.
    pub scope: &'a str,
    /// The short id segment. The matching `api_tokens.id` is
    /// `format!("{}{}", ID_DB_PREFIX, parsed.id_short)`.
    pub id_short: &'a str,
    pub secret: &'a str,
}

impl ParsedOpaque<'_> {
    /// Build the full `api_tokens.id` (e.g. `tok_AbCd...`) for a
    /// DB lookup.
    pub fn db_id(&self) -> String {
        format!("{}{}", ID_DB_PREFIX, self.id_short)
    }
}

/// Parse an opaque token string. Returns `Err` on any structural
/// problem — wrong prefix, wrong segment count, empty segment,
/// invalid scope. Does not touch the DB; verification (argon2id) is
/// a separate step.
pub fn parse(token: &str) -> Result<ParsedOpaque<'_>> {
    let rest = token
        .strip_prefix(PREFIX)
        .ok_or_else(|| anyhow!("opaque token missing kvl_ prefix"))?;

    // splitn(4, '_') keeps any underscores inside the secret as
    // part of the secret segment — the spec lets the random tail be
    // an arbitrary URL-safe string.
    let mut parts = rest.splitn(4, '_');
    let env = parts.next().unwrap_or("");
    let scope = parts.next().unwrap_or("");
    let id_short = parts.next().unwrap_or("");
    let secret = parts.next().unwrap_or("");

    if env.is_empty() || scope.is_empty() || id_short.is_empty() || secret.is_empty() {
        bail!("opaque token must be kvl_<env>_<scope>_<id>_<secret> with non-empty segments");
    }
    if scope != "org" && scope != "project" {
        bail!("opaque token scope must be 'org' or 'project', got '{scope}'");
    }
    Ok(ParsedOpaque {
        env,
        scope,
        id_short,
        secret,
    })
}

/// Compute an argon2id PHC-string hash of `secret`. The returned
/// string embeds the algorithm, parameters, salt, and digest — it
/// is what gets stored in `api_tokens.secret_hash`.
pub fn hash(secret: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let phc = argon
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash failed: {e}"))?;
    Ok(phc.to_string())
}

/// Constant-time verify a candidate secret against a stored
/// argon2id PHC-string hash. `Err` for both "hash unparseable" and
/// "secret didn't match" — handlers map either to a `401`.
pub fn verify(secret: &str, encoded: &str) -> Result<()> {
    let parsed = PasswordHash::new(encoded).map_err(|e| anyhow!("invalid argon2 hash: {e}"))?;
    Argon2::default()
        .verify_password(secret.as_bytes(), &parsed)
        .map_err(|e| anyhow!("opaque secret mismatch: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_happy_path() {
        let p = parse("kvl_prod_org_AbCd_8f2a9c").unwrap();
        assert_eq!(p.env, "prod");
        assert_eq!(p.scope, "org");
        assert_eq!(p.id_short, "AbCd");
        assert_eq!(p.secret, "8f2a9c");
        assert_eq!(p.db_id(), "tok_AbCd");
    }

    #[test]
    fn parse_secret_keeps_underscores() {
        // The random tail may contain underscores; splitn(4, '_')
        // keeps them inside the secret segment.
        let p = parse("kvl_dev_project_aB12_seg_with_underscores").unwrap();
        assert_eq!(p.secret, "seg_with_underscores");
    }

    #[test]
    fn parse_rejects_missing_prefix() {
        assert!(parse("tok_prod_org_AbCd_secret").is_err());
        assert!(parse("prod_org_AbCd_secret").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn parse_rejects_short_token() {
        // Three segments after kvl_ — missing the secret.
        assert!(parse("kvl_prod_org_AbCd").is_err());
    }

    #[test]
    fn parse_rejects_empty_segment() {
        assert!(parse("kvl__org_AbCd_secret").is_err()); // empty env
        assert!(parse("kvl_prod__AbCd_secret").is_err()); // empty scope
        assert!(parse("kvl_prod_org__secret").is_err()); // empty id
    }

    #[test]
    fn parse_rejects_unknown_scope() {
        assert!(parse("kvl_prod_admin_AbCd_secret").is_err());
        assert!(parse("kvl_prod_orgs_AbCd_secret").is_err());
    }

    #[test]
    fn hash_verify_round_trip() {
        let h = hash("hunter2").unwrap();
        assert!(verify("hunter2", &h).is_ok());
        assert!(verify("hunter3", &h).is_err());
        assert!(verify("", &h).is_err());
    }

    #[test]
    fn verify_rejects_unparseable_hash() {
        assert!(verify("anything", "not-a-phc-hash").is_err());
    }

    #[test]
    fn hash_is_unique_per_call() {
        // Salted hash: same input → different output each time.
        let a = hash("hunter2").unwrap();
        let b = hash("hunter2").unwrap();
        assert_ne!(a, b);
        // Both still verify against the original secret.
        assert!(verify("hunter2", &a).is_ok());
        assert!(verify("hunter2", &b).is_ok());
    }
}
