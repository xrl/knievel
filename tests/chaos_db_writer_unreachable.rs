//! Chaos / degraded-mode: DB writer unreachable.
//!
//! Phase 4.7 skeleton. Pairs with the first row of
//! `REQUIREMENTS.md` § 10.9 and `TESTING.md` § 9.
//!
//! Asserted behavior (per § 10.9):
//!   - Decision endpoint **continues serving** from the in-memory
//!     snapshot.
//!   - Management writes return `503 / db_writer_unreachable`.
//!   - Impression / click pings still work (events buffer in
//!     channel).
//!
//! Injection: `iptables -A OUTPUT -p tcp --dport 5432 -j DROP`
//! against the compose `knievel-postgres` container, applied from
//! the `chaos-injector` sidecar with `NET_ADMIN` capability.

#[tokio::test]
#[ignore = "chaos suite — needs the iptables-capable injector + compose harness (TESTING.md § 9). Activate by flipping #[ignore] once the harness lands."]
async fn db_writer_unreachable() {
    // 1. compose up the stack with the chaos-injector sidecar
    // 2. issue a baseline decision; assert 200
    // 3. injector: iptables drop on 5432 outbound from knievel
    // 4. assert subsequent management writes return 503
    //    db_writer_unreachable
    // 5. assert decisions still 200 (snapshot fallback)
    // 6. assert /e/i/<sig> still 200/204 (channel buffers)
    // 7. injector: iptables flush
    // 8. assert recovery: writes return 2xx again within 30 s
}
