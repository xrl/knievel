//! Shared application state.
//!
//! Carries handles every handler may need: the DB pool today,
//! the snapshot, the event channel, the principal-extractor's
//! token store, etc. as those land in Phase 3+.

use sqlx::PgPool;

#[derive(Clone, Default)]
pub struct AppState {
    /// Postgres pool. `None` when no `database.url` is configured —
    /// the dev-bootstrap stage of Phase 2 runs DB-less. Phase 3+
    /// makes the pool mandatory.
    pub db: Option<PgPool>,
    // Future: pub snapshot: Arc<ArcSwap<Snapshot>>,
    //         pub events: EventChannel,
    //         pub leader: LeaderHandle,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_db(mut self, db: PgPool) -> Self {
        self.db = Some(db);
        self
    }
}
