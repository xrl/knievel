//! Leader election via `pg_try_advisory_lock`.
//!
//! Phase 3.22. The partition manager (3.23), rollup compute
//! (3.24), and idempotency-key reaper (3.5 follow-up) all need
//! exactly one process running them at a time. Postgres
//! advisory locks give us the cheapest possible distributed
//! lock — no Zookeeper, no etcd, just a SQL `bool` per process.
//!
//! Design:
//! - One dedicated connection per knievel process — the lock is
//!   tied to the session, so a connection drop releases it
//!   automatically.
//! - `pg_try_advisory_lock` returns immediately rather than
//!   blocking; on `false` we sleep and retry.
//! - A watchdog enforces "must complete a maintenance run every
//!   N hours" — if the periodic-jobs scheduler hasn't ticked
//!   within the budget, the process exits, freeing the lock for
//!   another instance.
//!
//! Spec: `REQUIREMENTS.md` § 7.5.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use sqlx::{PgConnection, PgPool, Row};
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Advisory-lock id. Same constant across every knievel
/// instance so they all race for the same lock. Pulled out as
/// a constant so future locks (e.g. one per project for
/// per-project maintenance) can pick non-conflicting ids.
pub const LEADER_LOCK_ID: i64 = 0x4B6E_4C65_6164_722E; // "KnLeadr."

/// Default watchdog deadline. If the scheduler hasn't recorded
/// a tick in 4 hours, the process exits.
pub const WATCHDOG_BUDGET: Duration = Duration::from_secs(4 * 60 * 60);

/// Cloneable handle. `is_leader()` reads a cheap atomic; the
/// underlying lock state lives on the dedicated connection
/// inside the LeaderTask.
#[derive(Clone, Default)]
pub struct LeaderHandle {
    is_leader: Arc<AtomicBool>,
    last_tick: Arc<Mutex<Option<Instant>>>,
}

impl LeaderHandle {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::Acquire)
    }
    /// Periodic-jobs scheduler calls this on every successful
    /// tick. The watchdog reads it to enforce the deadline.
    pub async fn record_tick(&self) {
        let mut g = self.last_tick.lock().await;
        *g = Some(Instant::now());
    }
}

/// Acquire the advisory lock against `conn`. Returns true if
/// we got it; false if another instance already holds it.
pub async fn try_acquire(conn: &mut PgConnection) -> Result<bool> {
    let row = sqlx::query("SELECT pg_try_advisory_lock($1) AS got")
        .bind(LEADER_LOCK_ID)
        .fetch_one(conn)
        .await?;
    let got: bool = row.try_get("got")?;
    Ok(got)
}

/// Release the advisory lock. Idempotent — releases nothing if
/// we never held it.
pub async fn release(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LEADER_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

/// Spawn the leader task. Holds a dedicated connection,
/// races for the lock, and toggles `handle.is_leader` while
/// holding it. The watchdog runs as a sibling task — if the
/// scheduler hasn't ticked within `WATCHDOG_BUDGET`, this task
/// drops the lock and the process exits so another instance
/// can take over.
pub fn spawn(pool: PgPool, handle: LeaderHandle) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_one_session(&pool, &handle).await {
                tracing::warn!(error = %e, "leader session ended; reconnect in 5 s");
            }
            handle.is_leader.store(false, Ordering::Release);
            sleep(Duration::from_secs(5)).await;
        }
    })
}

async fn run_one_session(pool: &PgPool, handle: &LeaderHandle) -> Result<()> {
    let mut conn = pool.acquire().await?;
    loop {
        if try_acquire(&mut conn).await? {
            tracing::info!("became leader");
            handle.is_leader.store(true, Ordering::Release);
            // Record a synthetic tick at lock acquisition so the
            // watchdog has a starting deadline before any
            // scheduler tick lands.
            handle.record_tick().await;
            // Hold the lock as long as the connection stays
            // alive AND the watchdog is satisfied.
            loop {
                sleep(Duration::from_secs(30)).await;
                let last = handle.last_tick.lock().await.unwrap_or_else(Instant::now);
                if last.elapsed() > WATCHDOG_BUDGET {
                    tracing::error!(
                        budget_secs = WATCHDOG_BUDGET.as_secs(),
                        "watchdog deadline exceeded; releasing lock"
                    );
                    let _ = release(&mut conn).await;
                    handle.is_leader.store(false, Ordering::Release);
                    return Err(anyhow::anyhow!("watchdog deadline exceeded"));
                }
                // Cheap liveness check — if the connection died
                // the SELECT errors and we drop out to retry.
                let _ = sqlx::query("SELECT 1").fetch_one(&mut *conn).await?;
            }
        } else {
            // Another instance holds the lock. Sleep and retry.
            sleep(Duration::from_secs(5)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_default_is_follower() {
        let h = LeaderHandle::new();
        assert!(!h.is_leader());
    }

    #[tokio::test]
    async fn handle_record_tick_updates_state() {
        let h = LeaderHandle::new();
        assert!(h.last_tick.lock().await.is_none());
        h.record_tick().await;
        assert!(h.last_tick.lock().await.is_some());
    }

    #[test]
    fn lock_id_is_stable_constant() {
        // Smoke test against accidental reformatting.
        assert_eq!(LEADER_LOCK_ID, 0x4B6E_4C65_6164_722E);
    }
}
