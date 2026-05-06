//! Chaos: NOTIFY queue overflow.
//!
//! Phase 4.7 skeleton. Pairs with `TESTING.md` § 9 row 3.
//!
//! Asserted behavior: loader handles dropped notifies; poll
//! backstop reconciles state.
//!
//! Injection: spam `pg_notify('knievel_config', '<bogus>')` from a
//! side connection at thousands per second; Postgres caps the
//! NOTIFY queue at 8GB and after that NOTIFYs return errors.

#[tokio::test]
#[ignore = "chaos suite — needs the compose harness + a side connection running a tight pg_notify loop. Activate by flipping #[ignore] once the harness lands."]
async fn notify_queue_overflow() {
    // 1. compose up
    // 2. side conn: loop pg_notify('knievel_config', repeat('x', 7500))
    //    until queue saturates
    // 3. legit management write
    // 4. assert decision sees the change within 5 s (poll backstop
    //    catches it even when NOTIFY was dropped)
}
