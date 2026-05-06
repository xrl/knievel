//! Shared application state.
//!
//! Carries handles every handler may need: the DB pool today,
//! the snapshot, the event channel, the principal-extractor's
//! token store, etc. as those land in Phase 3+.

use sqlx::PgPool;

use crate::snapshot::SnapshotStore;

#[derive(Clone, Default)]
pub struct AppState {
    /// Postgres pool. `None` when no `database.url` is configured —
    /// the dev-bootstrap stage of Phase 2 runs DB-less. Phase 3+
    /// makes the pool mandatory.
    pub db: Option<PgPool>,
    /// In-process snapshot for the decision hot path
    /// (Phase 3.17). Empty store by default; the loader replaces
    /// it on every config_version bump.
    pub snapshot: SnapshotStore,
    // Future: pub events: EventChannel (3.21),
    //         pub leader: LeaderHandle (3.22),
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_db(mut self, db: PgPool) -> Self {
        self.db = Some(db);
        self
    }

    pub fn with_snapshot(mut self, snapshot: SnapshotStore) -> Self {
        self.snapshot = snapshot;
        self
    }
}
