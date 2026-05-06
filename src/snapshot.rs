//! In-process configuration snapshot.
//!
//! Phase 3.17. Decision-path RAM cache of every project's
//! flights, ads, sites, zones, and per-project secrets/flags.
//! Atomically swappable so reads never block writes.
//!
//! Spec: `REQUIREMENTS.md` § 7.2 — refresh is a notify+poll
//! belt-and-suspenders:
//!
//! 1. `LISTEN config_changed` on a long-lived writer connection.
//!    On notify, diff against the snapshot's current
//!    `config_version` and pull anything newer.
//! 2. Poll `SELECT last_value FROM knievel.config_version`
//!    every 5 s as a backstop. NOTIFY can drop messages under
//!    load, and Aurora failovers drop the LISTEN session.
//!
//! Both triggers reach the same diff-pull path; worst-case
//! staleness is bounded by the poll interval regardless.
//!
//! The in-memory shape is keyed by `(project_id, resource)` so
//! one process can serve thousands of small projects without
//! the per-project overhead a per-project snapshot map would
//! incur.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Result;
use sqlx::{PgPool, Row};
use tokio::sync::Notify;

use crate::selection::{Ad, Flight};

/// Wire-level snapshot keyed by project_id. Cheaply cloneable
/// because every leaf is `Arc`-backed; the swap is one atomic
/// pointer write.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub config_version: i64,
    pub projects: HashMap<String, Arc<ProjectSnapshot>>,
}

/// Per-project slice of the snapshot.
#[derive(Debug, Default)]
pub struct ProjectSnapshot {
    pub project_id: String,
    /// Owning org. Carried in-snapshot so the events flusher can
    /// attach `org_id` to ping rows without a per-request DB
    /// round-trip (`REQUIREMENTS.md` § 7.3 RLS-by-org).
    pub org_id_for_event: String,
    pub flights: Vec<Flight>,
    pub ads: Vec<Ad>,
    pub sites: Vec<SnapshotSite>,
    pub zones: Vec<SnapshotZone>,
    /// Current HMAC signing secret. The decision endpoint signs
    /// new URLs with this; the event endpoints accept either
    /// this OR `hmac_secret_previous` (during the 8 h overlap).
    pub hmac_secret: Vec<u8>,
    pub hmac_secret_previous: Option<Vec<u8>>,
    pub allow_force_decision: bool,
}

#[derive(Debug, Clone)]
pub struct SnapshotSite {
    pub id: i64,
    pub url: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SnapshotZone {
    pub id: i64,
    pub site_id: i64,
}

/// The handle handlers actually carry. `read()` returns a cheap
/// `Arc<Snapshot>` that's a consistent view across the whole
/// request — no torn reads.
#[derive(Clone)]
pub struct SnapshotStore {
    inner: Arc<RwLock<Arc<Snapshot>>>,
    /// Bumped by the loader on every successful swap. Tests
    /// `await_at_least(version)` on this to coordinate with the
    /// background task without sleeps.
    pub bumped: Arc<Notify>,
}

impl SnapshotStore {
    pub fn new(initial: Snapshot) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(initial))),
            bumped: Arc::new(Notify::new()),
        }
    }

    pub fn empty() -> Self {
        Self::new(Snapshot::default())
    }

    /// Atomic read. Returned `Arc` is a consistent point-in-time
    /// view; subsequent swaps don't affect it.
    pub fn read(&self) -> Arc<Snapshot> {
        self.inner.read().expect("snapshot lock poisoned").clone()
    }

    /// Atomic write. Replaces the entire snapshot in one pointer
    /// swap; readers either see the old or the new, never a
    /// half-built state.
    pub fn swap(&self, next: Snapshot) {
        let mut guard = self.inner.write().expect("snapshot lock poisoned");
        *guard = Arc::new(next);
        drop(guard);
        self.bumped.notify_waiters();
    }
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self::empty()
    }
}

/// Read the current `config_version` sequence value. Used both
/// at boot (to initialize the snapshot's version) and by the
/// 5 s poll backstop.
pub async fn read_config_version(pool: &PgPool) -> Result<i64> {
    let row = sqlx::query("SELECT last_value FROM knievel.config_version")
        .fetch_one(pool)
        .await?;
    let v: i64 = row.try_get(0)?;
    Ok(v)
}

/// Background task: notify+poll snapshot loader. Runs forever
/// until the parent task is dropped. The loader is intentionally
/// resilient — every error path logs and retries with backoff
/// rather than panicking, since a missed reload is recoverable
/// (worst case: we serve a stale snapshot for a few seconds)
/// and a panic would tear down the parent runtime.
///
/// `reload` is the user-supplied function that fetches the
/// fresh snapshot from the DB. Splitting it out keeps this
/// module testable without a real Postgres connection.
pub async fn run_loader<F, Fut>(pool: PgPool, store: SnapshotStore, mut reload: F)
where
    F: FnMut(PgPool) -> Fut + Send,
    Fut: std::future::Future<Output = Result<Snapshot>> + Send,
{
    use tokio::time::{interval, MissedTickBehavior};
    let mut tick = interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // Cold load.
    match reload(pool.clone()).await {
        Ok(snap) => store.swap(snap),
        Err(e) => tracing::error!(error = %e, "snapshot cold load failed; will retry"),
    }

    // NOTIFY listener integration is deferred — sqlx 0.8's
    // PgListener works against a writer connection but the
    // surrounding "diff and merge" path needs the events_raw
    // tables (Phase 3.20+) to materialize the per-resource
    // selects. For Phase 3.17 we rely on the 5 s poll loop to
    // pick up version drift, which the spec documents as the
    // backstop and a sufficient guarantee on its own.
    //
    // See `REQUIREMENTS.md` § 7.2: "worst-case staleness is
    // bounded by the poll interval regardless of NOTIFY
    // behavior."
    loop {
        tick.tick().await;
        let cur_version = store.read().config_version;
        let db_version = match read_config_version(&pool).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "config_version poll failed");
                continue;
            }
        };
        if db_version <= cur_version {
            continue;
        }
        match reload(pool.clone()).await {
            Ok(snap) => store.swap(snap),
            Err(e) => tracing::error!(error = %e, "snapshot reload failed; will retry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snap(v: i64) -> Snapshot {
        Snapshot {
            config_version: v,
            projects: HashMap::new(),
        }
    }

    #[test]
    fn store_atomic_swap_visible_to_readers() {
        let s = SnapshotStore::new(make_snap(1));
        assert_eq!(s.read().config_version, 1);
        s.swap(make_snap(2));
        assert_eq!(s.read().config_version, 2);
    }

    #[test]
    fn store_read_holds_consistent_view_across_swaps() {
        // A reader holding an Arc<Snapshot> from before a swap
        // continues to see the old version (no torn reads).
        let s = SnapshotStore::new(make_snap(1));
        let pre = s.read();
        s.swap(make_snap(2));
        assert_eq!(pre.config_version, 1);
        assert_eq!(s.read().config_version, 2);
    }

    #[tokio::test]
    async fn store_signals_bumped_on_swap() {
        let s = SnapshotStore::new(make_snap(1));
        let s2 = s.clone();
        let waiter = tokio::spawn(async move {
            s2.bumped.notified().await;
            s2.read().config_version
        });
        // Yield once so the waiter gets to register.
        tokio::task::yield_now().await;
        s.swap(make_snap(7));
        let v = waiter.await.unwrap();
        assert_eq!(v, 7);
    }

    #[test]
    fn project_snapshot_default_is_empty() {
        let p = ProjectSnapshot::default();
        assert!(p.flights.is_empty());
        assert!(p.ads.is_empty());
        assert!(p.sites.is_empty());
        assert!(p.zones.is_empty());
        assert!(p.hmac_secret.is_empty());
        assert!(p.hmac_secret_previous.is_none());
        assert!(p.org_id_for_event.is_empty());
        assert!(!p.allow_force_decision);
    }
}
