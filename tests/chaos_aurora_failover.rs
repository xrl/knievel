//! Chaos: Aurora failover (simulated).
//!
//! Phase 4.7 skeleton. Pairs with `TESTING.md` § 9 row 4 and
//! `REQUIREMENTS.md` § 7.5 (leader election).
//!
//! Asserted behavior: loader reconnects to the writer endpoint;
//! advisory lock released and re-acquired by another pod within
//! 30 s. The leader handle's watchdog confirms partition / rollup
//! maintenance still ticks.
//!
//! Injection: `docker compose restart knievel-postgres`. With
//! NOTIFY drop on failover (CLAUDE.md gotcha note + REQUIREMENTS.md
//! § 11 risks), this is the closest local approximation of an
//! Aurora failover event.

#[tokio::test]
#[ignore = "chaos suite — needs the compose harness with two knievel replicas (so the leader can fail over). Activate by flipping #[ignore] once the harness lands."]
async fn aurora_failover_simulated() {
    // 1. compose up with replicaCount=2
    // 2. identify the current leader via /version's leader block
    // 3. docker compose restart knievel-postgres
    // 4. assert the previous leader's advisory lock is released
    // 5. assert another pod's /version shows it as the leader
    //    within 30 s
    // 6. assert partition manager still runs (watchdog stays green)
}
