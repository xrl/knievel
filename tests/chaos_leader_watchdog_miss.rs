//! Chaos: leader watchdog miss.
//!
//! Phase 4.7 skeleton. Pairs with `REQUIREMENTS.md` § 10.9 row
//! "Leader maintenance failure" and `TESTING.md` § 9 row 8.
//!
//! Asserted behavior: process exits non-zero after a successful-run
//! gap exceeds `watchdog_hours`; k8s reschedules the pod;
//! advisory lock is released; another pod elects.
//!
//! Injection: `docker compose pause <leader>` for
//! `watchdog_hours + 1`. Postgres clock skew is the underlying
//! observable — the watchdog reads `last_successful_run_at`
//! from `knievel.config_version` (or the leader heartbeat
//! table) and compares to now().

#[tokio::test]
#[ignore = "chaos suite — needs the compose harness with two replicas + a way to advance time (or set a low watchdog_hours via env). Activate by flipping #[ignore] once the harness lands."]
async fn leader_watchdog_miss_triggers_failover() {
    // 1. compose up with replicaCount=2; watchdog_hours=10s
    //    via env override
    // 2. identify the leader
    // 3. docker compose pause <leader>
    // 4. wait > watchdog_hours
    // 5. assert: paused leader exits non-zero on resume
    // 6. assert: the other pod /version reports leadership
    // 7. assert: /readyz exposed the watchdog warning during
    //    the gap
}
