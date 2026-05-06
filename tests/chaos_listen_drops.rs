//! Chaos: LISTEN connection drops.
//!
//! Phase 4.7 skeleton. Pairs with `TESTING.md` § 9 row 2.
//!
//! Asserted behavior: snapshot loader reconnects with backoff;
//! poll backstop catches any divergence within 5 s.
//!
//! Injection: side psql connection runs `pg_terminate_backend(<pid>)`
//! against the loader's `LISTEN` connection (identified by
//! `application_name`).

#[tokio::test]
#[ignore = "chaos suite — needs the compose harness + a side psql with pg_terminate_backend privilege. Activate by flipping #[ignore] once the harness lands."]
async fn listen_connection_drops() {
    // 1. compose up
    // 2. observe loader's connection in pg_stat_activity (filter
    //    by application_name = 'knievel-snapshot-loader')
    // 3. side psql: SELECT pg_terminate_backend(pid)
    // 4. management write to mutate snapshot state
    // 5. wait ≤ 5 s, decision sees the new state (poll backstop
    //    catches the missed NOTIFY)
}
