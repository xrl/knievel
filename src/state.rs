//! Shared application state.
//!
//! Carries handles every handler may need: the DB pool, the
//! snapshot, the event channel, the leader-election handle, and
//! per-process feature flags. Background subsystems
//! (partition manager, rollup compute) hang off the leader
//! handle and don't need their own slot.

#![allow(dead_code)]

use std::sync::Arc;

use sqlx::PgPool;

use crate::config::AdminUiConfig;
use crate::events::EventSender;
use crate::image_upload::ImageStore;
use crate::leader::LeaderHandle;
use crate::snapshot::SnapshotStore;

#[derive(Clone, Default)]
pub struct AppState {
    /// Postgres pool. `None` when no `database.url` is configured —
    /// the dev-bootstrap stage of Phase 2 runs DB-less. Phase 3+
    /// makes the pool mandatory in production but leaves the
    /// option to support unit tests that build an `AppState`
    /// without a DB.
    pub db: Option<PgPool>,
    /// In-process snapshot for the decision hot path
    /// (Phase 3.17). Empty store by default; the loader replaces
    /// it on every config_version bump.
    pub snapshot: SnapshotStore,
    /// Event channel sender (Phase 3.21). `None` in tests that
    /// don't exercise the events flusher; production always has
    /// one. Handlers must skip emission cleanly when this is
    /// absent rather than failing the request.
    pub events: Option<EventSender>,
    /// Leader-election handle (Phase 3.22). Default-constructed
    /// is the "follower forever" handle, which is what tests want.
    pub leader: LeaderHandle,
    /// Object storage for creative images (Phase 3.29). `None`
    /// in tests that don't exercise upload; production injects
    /// the configured backend (S3/MinIO/in-memory).
    pub image_store: Option<Arc<dyn ImageStore>>,
    /// Decision-time flags. Today: the global force-override
    /// kill switch (`API.md` § 1: "decisions.force_overrides_enabled
    /// = false disables the path entirely"). Defaults to true so
    /// the per-project `allow_force_decision` flag stays the
    /// authoritative gate; flipping the kill switch is an
    /// emergency operator action.
    pub decisions: DecisionFlags,
    /// Admin-UI runtime config. Surfaced to the SPA via
    /// `GET /admin/config.json` (Phase 7.4) and consumed by the
    /// Phase 7.11 `StaticFilesEndpoint` mount. Empty defaults
    /// run as a headless API with no admin console served.
    pub admin_ui: AdminUiConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct DecisionFlags {
    pub force_overrides_enabled: bool,
}

impl Default for DecisionFlags {
    fn default() -> Self {
        Self {
            force_overrides_enabled: true,
        }
    }
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

    pub fn with_events(mut self, events: EventSender) -> Self {
        self.events = Some(events);
        self
    }

    pub fn with_leader(mut self, leader: LeaderHandle) -> Self {
        self.leader = leader;
        self
    }

    pub fn with_image_store(mut self, image_store: Arc<dyn ImageStore>) -> Self {
        self.image_store = Some(image_store);
        self
    }

    pub fn with_decisions(mut self, flags: DecisionFlags) -> Self {
        self.decisions = flags;
        self
    }

    pub fn with_admin_ui(mut self, admin_ui: AdminUiConfig) -> Self {
        self.admin_ui = admin_ui;
        self
    }

    /// Return the DB pool or `Err(String)` with a fixed message that
    /// callers can map to a 500 `no_db` envelope.
    ///
    /// Eliminates the four identical `match state.0.db.as_ref() { … }`
    /// blocks across `OrgApi`'s handlers. Adding more handlers that
    /// need the pool should use this rather than re-inlining the match.
    pub fn require_db(&self) -> Result<&sqlx::PgPool, (&'static str, &'static str)> {
        self.db
            .as_ref()
            .ok_or(("no_db", "no database configured"))
    }
}
