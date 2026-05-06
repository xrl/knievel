//! Chaos: JWKS endpoint unreachable.
//!
//! Phase 4.7 skeleton. Pairs with `REQUIREMENTS.md` § 10.9
//! row "JWKS endpoint unreachable" and `TESTING.md` § 9 row 6.
//!
//! Asserted behavior: cached keys serve until TTL expires; after
//! TTL, JWT validation starts failing for that issuer (other
//! issuers unaffected); cache refresh attempts continue at
//! backoff. `kid` cache miss → 401 for that issuer.
//!
//! Injection: iptables rule on the wiremock container's port to
//! block egress; the wiremock service hosts the JWKS document
//! when reachable.

#[tokio::test]
#[ignore = "chaos suite — needs wiremock for JWKS and an iptables-capable injector. Activate by flipping #[ignore] once the harness lands."]
async fn jwks_unreachable() {
    // 1. compose up with wiremock serving JWKS for issuer A
    // 2. valid JWT for issuer A → 200
    // 3. injector: iptables drop on wiremock:8080
    // 4. valid JWT for issuer A still → 200 (cached) until TTL
    // 5. wait for TTL + small jitter
    // 6. assert: kid cache miss → 401 for issuer A
    // 7. assert: opaque tokens (and any second issuer) still work
}
