//! Chaos: connection-pool exhaustion.
//!
//! Phase 4.7 skeleton. Pairs with `REQUIREMENTS.md` § 10.9 row
//! "All Postgres connections exhausted" and `TESTING.md` § 9
//! row 7.
//!
//! Asserted behavior: all endpoints return 503 except `/healthz`
//! and `/metrics`; status code is `503 / db_pool_exhausted`.
//! Recovery clears once connections are released.
//!
//! Injection: open `database.maxConnections` side connections
//! from a separate test process and hold them open with
//! `pg_sleep(60)` so the knievel pool can't acquire any.

#[tokio::test]
#[ignore = "chaos suite — needs a side-process connection holder. Activate by flipping #[ignore] once the harness lands."]
async fn pool_exhaustion() {
    // 1. compose up; max_connections in config sets the pool ceiling
    // 2. side: open N=max_connections connections, each runs
    //    SELECT pg_sleep(60)
    // 3. assert: /healthz returns 200
    // 4. assert: /metrics returns 200
    // 5. assert: /v1/projects/{p}/decisions returns 503 with
    //    error.code = db_pool_exhausted
    // 6. release the side connections
    // 7. assert: endpoints return to 200 within a few seconds
}
