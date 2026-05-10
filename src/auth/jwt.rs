//! JWT validator + JWKS cache.
//!
//! Phase 3.26 + follow-up. Stateless JWT validation per `AUTH.md`
//! "JWTs":
//!
//!   1. Header carries a `kid` and an `alg` in the per-issuer
//!      allow-list (default: `RS256`, `ES256`, `PS256`). `alg:
//!      none` and `HS*` algorithms are rejected unconditionally.
//!   2. Signature verifies against the JWK matching `kid`
//!      (`JwtVerifier::verify`, follow-up).
//!   3. `iss` matches a configured issuer; `aud` contains the
//!      configured audience.
//!   4. `exp` is in the future (30 s clock-skew tolerance);
//!      `nbf`/`iat` not in the future.
//!   5. The `knievel` claim parses into the standard authz
//!      shape — `scope`, `org_id`, optional `project_id`, `role`.
//!
//! Two entrypoints:
//!   * `validate(token, policies, now_secs)` — pure / sync /
//!     signature-stub. Used by unit tests and the `system::version`
//!     visibility surface.
//!   * `JwtVerifier::verify(token).await` — the runtime path
//!     wired into `BearerAuth`. Adds JWKS fetch + RS256/ES256
//!     signature verification on top of `validate`.
//!
//! Spec refs: `AUTH.md` "JWTs", "JWKS handling", "Startup
//! Linting and Effective-Policy Visibility."

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::auth::{Principal, Role, Scope};
use crate::config::JwtIssuerConfig;

/// Errors raised by `validate`. Maps directly to the `code` /
/// `detail` field surfaced on `401 invalid_token` per
/// `AUTH.md`.
#[derive(Debug, PartialEq, Eq)]
pub enum JwtError {
    /// Three-segment shape failed.
    Malformed,
    /// `alg` rejected at the header level.
    AlgorithmRejected,
    /// `kid` missing from the header.
    MissingKid,
    /// Signature verification failed against every candidate
    /// JWK for that issuer's kid.
    Signature,
    /// `iss` not in the configured allow-list.
    Issuer,
    /// `aud` doesn't contain the configured audience for the
    /// matched issuer.
    Audience,
    /// `exp` in the past, or `nbf`/`iat` in the future.
    Expired,
    /// The `knievel` claim is missing or doesn't parse.
    ClaimMissing,
    ClaimMalformed,
}

/// Per-issuer policy. Multiple issuers supported for federation
/// — see `AUTH.md` "JWKS handling".
#[derive(Clone, Debug)]
pub struct IssuerPolicy {
    pub issuer: String,
    pub audience: String,
    /// Algorithms we accept for *this issuer's* tokens.
    pub algorithms: Vec<String>,
    /// JWKS URL. None = derive from
    /// `{issuer}/.well-known/openid-configuration`.
    pub jwks_url: Option<String>,
    /// Where the authz claim lives. Default: `knievel`.
    pub claim: String,
    /// Optional rule-driven mapping of standard claims onto
    /// the authz fields when the issuer can't carry a custom
    /// claim. v0 stub field — wired in 3.26 follow-up.
    pub claim_mapping: Vec<ClaimMappingRule>,
}

#[derive(Clone, Debug)]
pub struct ClaimMappingRule {
    pub from: String,
    pub to: String,
    pub regex: Option<String>,
}

/// Default algorithm allow-list. RSA + ECDSA + RSA-PSS only.
/// HS* and `none` are explicitly rejected — we never want to
/// accept symmetric or unsigned tokens.
pub fn default_algorithms() -> Vec<String> {
    vec!["RS256".into(), "ES256".into(), "PS256".into()]
}

/// JWKS cache. Cloneable; the inner cache is `RwLock`-guarded.
/// Real fetches use the `JwksFetcher` trait so wiremock can
/// stand in during tests.
#[derive(Clone, Default)]
pub struct JwksCache {
    inner: Arc<RwLock<HashMap<String, CachedJwks>>>,
}

#[derive(Clone, Debug)]
struct CachedJwks {
    fetched_at_secs: u64,
    keys: Vec<JsonWebKey>,
}

impl JwksCache {
    pub fn new() -> Self {
        Self::default()
    }
    /// Default cache TTL per spec (`AUTH.md` "JWKS handling": 1 h).
    pub const TTL: Duration = Duration::from_secs(60 * 60);

    /// Returns the cached keys for `issuer` if present and within
    /// the TTL window. Stale entries are treated as misses; the
    /// caller is expected to re-fetch and `insert` the result.
    pub fn get(&self, issuer: &str) -> Option<Vec<JsonWebKey>> {
        let g = self.inner.read().ok()?;
        let entry = g.get(issuer)?;
        let now = now_secs();
        if now.saturating_sub(entry.fetched_at_secs) > Self::TTL.as_secs() {
            return None;
        }
        Some(entry.keys.clone())
    }

    pub fn insert(&self, issuer: &str, keys: Vec<JsonWebKey>) {
        let now = now_secs();
        if let Ok(mut g) = self.inner.write() {
            g.insert(
                issuer.into(),
                CachedJwks {
                    fetched_at_secs: now,
                    keys,
                },
            );
        }
    }
}

/// Minimal JWK shape. Real signature verification will need
/// `kty`, `n`, `e` (RSA) or `crv`, `x`, `y` (ECDSA); for the
/// 3.26 scaffold we only carry `kid` so the lookup-by-kid path
/// can be exercised in tests.
#[derive(Clone, Debug, Deserialize)]
pub struct JsonWebKey {
    pub kid: String,
    pub kty: String,
    pub alg: Option<String>,
    pub n: Option<String>,
    pub e: Option<String>,
}

/// JWT header. `kid` and `alg` are the only fields we touch at
/// the validation layer — the rest are carried verbatim to the
/// signature-verify step.
#[derive(Clone, Debug, Deserialize)]
pub struct JwtHeader {
    pub alg: String,
    pub kid: Option<String>,
    pub typ: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StandardClaims {
    pub iss: String,
    pub aud: serde_json::Value,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub nbf: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
    pub sub: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KnievelClaim {
    pub scope: String,
    pub org_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub role: String,
}

/// Parse the three-segment JWT and validate at the header +
/// claim level. Signature verification is staged behind the
/// JWKS fetch, which lands in the 3.26 follow-up — this scaffold
/// asserts the algorithm allow-list, parses the claim, and
/// constructs a `Principal`.
pub fn validate(
    token: &str,
    policies: &[IssuerPolicy],
    now_secs: i64,
) -> Result<Principal, JwtError> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return Err(JwtError::Malformed);
    }
    let header_bytes = URL_SAFE_NO_PAD
        .decode(parts[0])
        .map_err(|_| JwtError::Malformed)?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| JwtError::Malformed)?;
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| JwtError::Malformed)?;

    // Algorithm allow-list check happens before issuer lookup
    // because `alg: none` is rejected unconditionally — never
    // letting us accept an unsigned token regardless of issuer
    // policy.
    let alg_lower = header.alg.to_ascii_lowercase();
    if alg_lower == "none" {
        return Err(JwtError::AlgorithmRejected);
    }
    if alg_lower.starts_with("hs") {
        return Err(JwtError::AlgorithmRejected);
    }
    if header.kid.is_none() {
        return Err(JwtError::MissingKid);
    }

    let standard: StandardClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| JwtError::Malformed)?;
    let policy = policies
        .iter()
        .find(|p| p.issuer == standard.iss)
        .ok_or(JwtError::Issuer)?;

    if !policy.algorithms.contains(&header.alg) {
        return Err(JwtError::AlgorithmRejected);
    }
    if !audience_contains(&standard.aud, &policy.audience) {
        return Err(JwtError::Audience);
    }
    let skew = 30;
    if let Some(exp) = standard.exp {
        if now_secs - skew > exp {
            return Err(JwtError::Expired);
        }
    }
    if let Some(nbf) = standard.nbf {
        if now_secs + skew < nbf {
            return Err(JwtError::Expired);
        }
    }
    if let Some(iat) = standard.iat {
        if now_secs + skew < iat {
            return Err(JwtError::Expired);
        }
    }

    // Pull the authz claim by name. v0 supports the standard
    // `knievel` claim only; claim_mapping rules land in the
    // follow-up.
    let payload: serde_json::Value =
        serde_json::from_slice(&payload_bytes).map_err(|_| JwtError::Malformed)?;
    let raw_claim = payload
        .get(&policy.claim)
        .ok_or(JwtError::ClaimMissing)?
        .clone();
    let claim: KnievelClaim =
        serde_json::from_value(raw_claim).map_err(|_| JwtError::ClaimMalformed)?;

    let scope = match claim.scope.as_str() {
        "org" => Scope::Org,
        "project" => Scope::Project,
        _ => return Err(JwtError::ClaimMalformed),
    };
    let role: Role = claim.role.parse().map_err(|_| JwtError::ClaimMalformed)?;

    Ok(Principal {
        actor_id: format!("jwt:{}", standard.sub.unwrap_or_default()),
        org_id: claim.org_id,
        project_id: claim.project_id,
        scope,
        role,
        token_type: crate::auth::TokenType::Jwt,
    })
}

fn audience_contains(aud: &serde_json::Value, want: &str) -> bool {
    match aud {
        serde_json::Value::String(s) => s == want,
        serde_json::Value::Array(items) => items
            .iter()
            .any(|v| v.as_str().map(|s| s == want).unwrap_or(false)),
        _ => false,
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Runtime JWT verifier. Owns the configured issuer policies, the
/// JWKS cache, and an HTTP client used for `.well-known/openid-
/// configuration` discovery + JWKS fetches.
///
/// Wired into `AppState` at boot (`server::build_state`). Cloning
/// is cheap — the cache + http client are `Arc`-shared so handlers
/// don't pay a per-request setup cost.
#[derive(Clone)]
pub struct JwtVerifier {
    inner: Arc<JwtVerifierInner>,
}

impl Default for JwtVerifier {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

struct JwtVerifierInner {
    policies: Vec<JwtIssuerConfig>,
    cache: JwksCache,
    http: reqwest::Client,
}

/// Errors produced while attempting JWKS discovery / fetch. Mapped
/// to `JwtError::Signature` at the verifier surface so callers see
/// a single "couldn't verify" outcome and don't have to discriminate
/// between transport-layer and crypto-layer failures.
#[derive(Debug)]
enum JwksError {
    DiscoveryFailed,
    FetchFailed,
    ParseFailed,
    KeyNotFound,
}

impl JwtVerifier {
    pub fn new(policies: Vec<JwtIssuerConfig>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(JwtVerifierInner {
                policies,
                cache: JwksCache::new(),
                http,
            }),
        }
    }

    /// True when at least one issuer policy is configured. Used by
    /// `verify_bearer` to skip the JWT path entirely on
    /// deployments that haven't enabled it (and therefore should
    /// hit the opaque path or return 401).
    pub fn is_enabled(&self) -> bool {
        !self.inner.policies.is_empty()
    }

    /// Configured issuer policies, exposed for `system::version`
    /// effective-policy visibility.
    pub fn policies(&self) -> &[JwtIssuerConfig] {
        &self.inner.policies
    }

    /// End-to-end verification: parse → match policy → fetch JWKS
    /// → signature-verify → claim extraction.
    pub async fn verify(&self, token: &str) -> Result<Principal, JwtError> {
        // Cheap header parse first — gives us alg/kid for fast-fail
        // on `none`/HS*/missing-kid before any HTTP.
        let header = jsonwebtoken::decode_header(token).map_err(|_| JwtError::Malformed)?;
        if matches!(
            header.alg,
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
        ) {
            return Err(JwtError::AlgorithmRejected);
        }
        let kid = header.kid.as_deref().ok_or(JwtError::MissingKid)?;

        // Decode payload (no signature check yet) so we can find
        // the matching policy by `iss`.
        let parts: Vec<&str> = token.splitn(3, '.').collect();
        if parts.len() != 3 {
            return Err(JwtError::Malformed);
        }
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| JwtError::Malformed)?;
        let standard: StandardClaims =
            serde_json::from_slice(&payload_bytes).map_err(|_| JwtError::Malformed)?;
        let policy = self
            .inner
            .policies
            .iter()
            .find(|p| p.issuer == standard.iss)
            .ok_or(JwtError::Issuer)?;

        // Per-issuer algorithm allow-list. The header already
        // passed the global HS*/none reject; this narrows further.
        let alg_str = format!("{:?}", header.alg);
        if !policy.algorithms.iter().any(|a| a == &alg_str) {
            return Err(JwtError::AlgorithmRejected);
        }

        let jwk = self
            .lookup_jwk(&policy.issuer, &policy.jwks_url, kid)
            .await
            .map_err(|err| {
                // WARN: JWKS-fetch failure is a deployment problem
                // (issuer URL wrong, network egress blocked, JWKS
                // rotated and cache stale). Default INFO operators
                // need to see this.
                tracing::warn!(?err, issuer = %policy.issuer, "JWKS lookup failed");
                JwtError::Signature
            })?;

        let key = build_decoding_key(&jwk).ok_or(JwtError::Signature)?;

        // jsonwebtoken handles signature + iss + aud + exp + nbf
        // + iat with leeway in one shot. The claim extraction below
        // re-parses the payload as serde_json::Value so we can pull
        // the configured (`policy.claim`) custom claim by name —
        // jsonwebtoken's typed claims path can't model that without
        // a generic per-issuer claims type.
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&policy.issuer]);
        validation.set_audience(&[&policy.audience]);
        validation.leeway = 30;
        let token_data = jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
            .map_err(|err| {
                use jsonwebtoken::errors::ErrorKind::*;
                match err.kind() {
                    InvalidAudience => JwtError::Audience,
                    InvalidIssuer => JwtError::Issuer,
                    ExpiredSignature | ImmatureSignature => JwtError::Expired,
                    InvalidSignature | RsaFailedSigning | InvalidAlgorithm => JwtError::Signature,
                    _ => {
                        tracing::debug!(?err, "JWT decode failed");
                        JwtError::Signature
                    }
                }
            })?;

        let raw_claim = token_data
            .claims
            .get(&policy.claim)
            .ok_or(JwtError::ClaimMissing)?
            .clone();
        let claim: KnievelClaim =
            serde_json::from_value(raw_claim).map_err(|_| JwtError::ClaimMalformed)?;
        let scope = match claim.scope.as_str() {
            "org" => Scope::Org,
            "project" => Scope::Project,
            _ => return Err(JwtError::ClaimMalformed),
        };
        let role: Role = claim.role.parse().map_err(|_| JwtError::ClaimMalformed)?;

        Ok(Principal {
            actor_id: format!("jwt:{}", standard.sub.unwrap_or_default()),
            org_id: claim.org_id,
            project_id: claim.project_id,
            scope,
            role,
            token_type: crate::auth::TokenType::Jwt,
        })
    }

    async fn lookup_jwk(
        &self,
        issuer: &str,
        configured_jwks_url: &str,
        kid: &str,
    ) -> Result<JsonWebKey, JwksError> {
        if let Some(keys) = self.inner.cache.get(issuer) {
            if let Some(k) = keys.iter().find(|k| k.kid == kid).cloned() {
                return Ok(k);
            }
            // Cached but kid not present — issuer may have rotated;
            // fall through to a fresh fetch.
        }
        let keys = self.fetch_jwks(issuer, configured_jwks_url).await?;
        let k = keys
            .iter()
            .find(|k| k.kid == kid)
            .cloned()
            .ok_or(JwksError::KeyNotFound)?;
        self.inner.cache.insert(issuer, keys);
        Ok(k)
    }

    async fn fetch_jwks(
        &self,
        issuer: &str,
        configured_jwks_url: &str,
    ) -> Result<Vec<JsonWebKey>, JwksError> {
        let jwks_url = if !configured_jwks_url.is_empty() {
            configured_jwks_url.to_string()
        } else {
            let trimmed = issuer.trim_end_matches('/');
            let cfg_url = format!("{trimmed}/.well-known/openid-configuration");
            let cfg: serde_json::Value = self
                .inner
                .http
                .get(&cfg_url)
                .send()
                .await
                .map_err(|_| JwksError::DiscoveryFailed)?
                .error_for_status()
                .map_err(|_| JwksError::DiscoveryFailed)?
                .json()
                .await
                .map_err(|_| JwksError::DiscoveryFailed)?;
            cfg.get("jwks_uri")
                .and_then(|v| v.as_str())
                .ok_or(JwksError::DiscoveryFailed)?
                .to_string()
        };

        let body: serde_json::Value = self
            .inner
            .http
            .get(&jwks_url)
            .send()
            .await
            .map_err(|_| JwksError::FetchFailed)?
            .error_for_status()
            .map_err(|_| JwksError::FetchFailed)?
            .json()
            .await
            .map_err(|_| JwksError::FetchFailed)?;
        let keys_raw = body.get("keys").cloned().ok_or(JwksError::ParseFailed)?;
        serde_json::from_value(keys_raw).map_err(|_| JwksError::ParseFailed)
    }
}

fn build_decoding_key(jwk: &JsonWebKey) -> Option<DecodingKey> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk.n.as_deref()?;
            let e = jwk.e.as_deref()?;
            DecodingKey::from_rsa_components(n, e).ok()
        }
        // ES256 keys carry `crv` + `x` + `y`. Add when a real issuer
        // ships them; Keycloak's default is RS256 so this isn't on
        // the hot path for v0.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(header: serde_json::Value, payload: serde_json::Value) -> String {
        let h = URL_SAFE_NO_PAD.encode(header.to_string());
        let p = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{h}.{p}.")
    }

    fn policy() -> IssuerPolicy {
        IssuerPolicy {
            issuer: "https://issuer.test".into(),
            audience: "knievel".into(),
            algorithms: default_algorithms(),
            jwks_url: None,
            claim: "knievel".into(),
            claim_mapping: vec![],
        }
    }

    #[test]
    fn rejects_alg_none() {
        let t = make_token(
            serde_json::json!({"alg": "none", "kid": "k1"}),
            serde_json::json!({}),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::AlgorithmRejected)
        ));
    }

    #[test]
    fn rejects_hs256() {
        let t = make_token(
            serde_json::json!({"alg": "HS256", "kid": "k1"}),
            serde_json::json!({}),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::AlgorithmRejected)
        ));
    }

    #[test]
    fn rejects_missing_kid() {
        let t = make_token(
            serde_json::json!({"alg": "RS256"}),
            serde_json::json!({"iss": "x", "aud": "y"}),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::MissingKid)
        ));
    }

    #[test]
    fn rejects_unknown_issuer() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({"iss": "https://other.test", "aud": "knievel"}),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::Issuer)
        ));
    }

    #[test]
    fn rejects_wrong_audience() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({"iss": "https://issuer.test", "aud": "not-knievel"}),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::Audience)
        ));
    }

    #[test]
    fn audience_array_membership() {
        // aud may be a string or an array of strings (RFC 7519 § 4.1.3).
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "iss": "https://issuer.test",
                "aud": ["other", "knievel"],
                "knievel": {"scope": "org", "org_id": "o", "role": "editor"}
            }),
        );
        let p = validate(&t, &[policy()], 0).unwrap();
        assert_eq!(p.org_id, "o");
        assert_eq!(p.role, Role::Editor);
    }

    #[test]
    fn rejects_expired_with_clock_skew() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "iss": "https://issuer.test",
                "aud": "knievel",
                "exp": 100,
                "knievel": {"scope": "org", "org_id": "o", "role": "editor"}
            }),
        );
        // now = 200; skew = 30; 200-30 = 170 > 100 → Expired.
        assert!(matches!(
            validate(&t, &[policy()], 200),
            Err(JwtError::Expired)
        ));
        // Within skew: now = 110 → 110-30 = 80 ≤ 100 → ok.
        let p = validate(&t, &[policy()], 110).unwrap();
        assert_eq!(p.role, Role::Editor);
    }

    #[test]
    fn rejects_missing_claim() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "iss": "https://issuer.test",
                "aud": "knievel",
            }),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::ClaimMissing)
        ));
    }

    #[test]
    fn rejects_malformed_claim() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "iss": "https://issuer.test",
                "aud": "knievel",
                "knievel": {"scope": "bogus", "org_id": "o", "role": "editor"}
            }),
        );
        assert!(matches!(
            validate(&t, &[policy()], 0),
            Err(JwtError::ClaimMalformed)
        ));
    }

    #[test]
    fn happy_path_yields_principal() {
        let t = make_token(
            serde_json::json!({"alg": "RS256", "kid": "k1"}),
            serde_json::json!({
                "iss": "https://issuer.test",
                "aud": "knievel",
                "sub": "user-1",
                "knievel": {
                    "scope": "project",
                    "org_id": "o",
                    "project_id": "pj_1",
                    "role": "admin"
                }
            }),
        );
        let p = validate(&t, &[policy()], 0).unwrap();
        assert_eq!(p.actor_id, "jwt:user-1");
        assert_eq!(p.org_id, "o");
        assert_eq!(p.project_id.as_deref(), Some("pj_1"));
        assert_eq!(p.scope, Scope::Project);
        assert_eq!(p.role, Role::Admin);
    }

    #[test]
    fn jwks_cache_round_trip() {
        let c = JwksCache::new();
        assert!(c.get("iss").is_none());
        c.insert(
            "iss",
            vec![JsonWebKey {
                kid: "k1".into(),
                kty: "RSA".into(),
                alg: Some("RS256".into()),
                n: None,
                e: None,
            }],
        );
        let keys = c.get("iss").unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kid, "k1");
    }
}
