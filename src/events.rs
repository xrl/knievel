//! Event channel + COPY flusher.
//!
//! Phase 3.21. Decisions and impression/click events are
//! buffered in a bounded `tokio::mpsc` channel and `COPY`'d to
//! `events_raw` in batches every 1–2 s or every 5 k events,
//! whichever comes first. Channel saturation surfaces as
//! `503 event_channel_saturated` on the decision endpoint.
//!
//! Spec refs: `REQUIREMENTS.md` § 7.6, `API.md` § 4.

#![allow(dead_code)]

use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

/// One event row queued for COPY. Field set mirrors the
/// `events_raw` table from migration `0010_events_raw.sql`.
#[derive(Debug, Clone)]
pub struct Event {
    pub ts_ms: i64,
    pub org_id: String,
    pub project_id: String,
    pub kind: EventKind,
    pub placement_id: Option<String>,
    pub site_id: Option<i64>,
    pub zone_id: Option<i64>,
    pub ad_id: Option<i64>,
    pub creative_id: Option<i64>,
    pub flight_id: Option<i64>,
    pub campaign_id: Option<i64>,
    pub advertiser_id: Option<i64>,
    pub url: Option<String>,
    pub referrer_host: Option<String>,
    pub user_agent_hash: Option<Vec<u8>>,
    pub signature_nonce: Option<Vec<u8>>,
    pub dedup_key: Option<Vec<u8>>,
    pub snapshot_version: Option<i64>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EventKind {
    Decision = 0,
    Impression = 1,
    Click = 2,
}

/// Default channel capacity. Sized for ~5 k events / second over
/// a 1 s drain window.
pub const DEFAULT_CAPACITY: usize = 8_192;
/// Drain whichever is sooner.
pub const DRAIN_INTERVAL: Duration = Duration::from_secs(1);
pub const DRAIN_BATCH: usize = 5_000;

/// Sender side. Cloneable; every handler gets one.
#[derive(Clone)]
pub struct EventSender {
    tx: mpsc::Sender<Event>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// Backpressure — buffer is full and the flusher hasn't
    /// caught up. Decision endpoint surfaces this as
    /// `503 event_channel_saturated`.
    ChannelSaturated,
    /// Flusher is gone (process is shutting down).
    FlusherDown,
}

impl EventSender {
    /// Non-blocking send. Returns `ChannelSaturated` immediately
    /// rather than blocking the request handler.
    pub fn try_send(&self, e: Event) -> Result<(), SendError> {
        match self.tx.try_send(e) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(SendError::ChannelSaturated),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SendError::FlusherDown),
        }
    }
}

/// Spawn the flusher task. Returns the `EventSender` handlers
/// share, plus a `JoinHandle` so the parent can `await` graceful
/// drain on shutdown.
pub fn spawn(pool: PgPool, capacity: usize) -> (EventSender, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Event>(capacity);
    let handle = tokio::spawn(flusher_loop(pool, rx));
    (EventSender { tx }, handle)
}

async fn flusher_loop(pool: PgPool, mut rx: mpsc::Receiver<Event>) {
    let mut tick = interval(DRAIN_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut batch: Vec<Event> = Vec::with_capacity(DRAIN_BATCH);
    loop {
        tokio::select! {
            biased;
            // Drain the channel as fast as possible up to the
            // batch limit; then flush.
            n = {
                let want = DRAIN_BATCH - batch.len();
                rx.recv_many(&mut batch, want)
            } => {
                if n == 0 {
                    // Sender side dropped. Flush whatever we
                    // have and exit.
                    if !batch.is_empty() {
                        let _ = flush_batch(&pool, &batch).await;
                        batch.clear();
                    }
                    break;
                }
                if batch.len() >= DRAIN_BATCH {
                    if let Err(e) = flush_batch(&pool, &batch).await {
                        tracing::warn!(error = %e, batch_size = batch.len(), "events COPY failed; dropping batch");
                    }
                    batch.clear();
                }
            }
            _ = tick.tick() => {
                if !batch.is_empty() {
                    if let Err(e) = flush_batch(&pool, &batch).await {
                        tracing::warn!(error = %e, batch_size = batch.len(), "events COPY failed; dropping batch");
                    }
                    batch.clear();
                }
            }
        }
    }
    tracing::info!("event flusher exited cleanly");
}

/// Encode and `COPY` a batch into `events_raw`. Uses
/// per-row INSERTs for v0 simplicity; `COPY` (binary or CSV)
/// is the same shape and a follow-up optimization, deferred
/// until the load-test (5.7) shows the INSERT path bottlenecking.
async fn flush_batch(pool: &PgPool, batch: &[Event]) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await.context("events flusher begin")?;
    // We must bind the org_id GUC to satisfy the events_raw RLS
    // policy. Events from multiple orgs flow through the same
    // batch, so we set the GUC per row in a tiny prologue to
    // each INSERT. Note: this means the flusher writes events
    // for whichever org_id each event carries, regardless of any
    // request-scoped principal.
    for e in batch {
        sqlx::query("SELECT set_config('knievel.org_id', $1, true)")
            .bind(&e.org_id)
            .execute(&mut *tx)
            .await
            .context("events flusher bind org_id")?;
        sqlx::query(
            "INSERT INTO knievel.events_raw
                 (ts, org_id, project_id, kind, placement_id, site_id, zone_id,
                  ad_id, creative_id, flight_id, campaign_id, advertiser_id,
                  url, referrer_host, user_agent_hash,
                  signature_nonce, dedup_key, snapshot_version, is_duplicate)
             VALUES (
                 to_timestamp($1::double precision / 1000.0),
                 $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                 $16, $17, $18,
                 false
             )
             ON CONFLICT (project_id, kind, dedup_key, ts)
             DO UPDATE SET is_duplicate = true",
        )
        .bind(e.ts_ms)
        .bind(&e.org_id)
        .bind(&e.project_id)
        .bind(e.kind as i16)
        .bind(e.placement_id.as_deref())
        .bind(e.site_id)
        .bind(e.zone_id)
        .bind(e.ad_id)
        .bind(e.creative_id)
        .bind(e.flight_id)
        .bind(e.campaign_id)
        .bind(e.advertiser_id)
        .bind(e.url.as_deref())
        .bind(e.referrer_host.as_deref())
        .bind(e.user_agent_hash.as_deref())
        .bind(e.signature_nonce.as_deref())
        .bind(e.dedup_key.as_deref())
        .bind(e.snapshot_version)
        .execute(&mut *tx)
        .await
        .context("events flusher insert")?;
    }
    tx.commit().await.context("events flusher commit")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_event() -> Event {
        Event {
            ts_ms: 1_700_000_000_000,
            org_id: "org_a".into(),
            project_id: "pj_a".into(),
            kind: EventKind::Decision,
            placement_id: Some("header".into()),
            site_id: Some(1),
            zone_id: None,
            ad_id: None,
            creative_id: None,
            flight_id: None,
            campaign_id: None,
            advertiser_id: None,
            url: None,
            referrer_host: None,
            user_agent_hash: None,
            signature_nonce: None,
            dedup_key: None,
            snapshot_version: None,
        }
    }

    #[tokio::test]
    async fn try_send_returns_saturated_when_full() {
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let s = EventSender { tx };
        s.try_send(fake_event()).unwrap();
        // Second send fills the channel beyond capacity.
        let r = s.try_send(fake_event());
        assert_eq!(r, Err(SendError::ChannelSaturated));
    }

    #[tokio::test]
    async fn try_send_returns_flusher_down_when_closed() {
        let (tx, rx) = mpsc::channel::<Event>(8);
        drop(rx);
        let s = EventSender { tx };
        let r = s.try_send(fake_event());
        assert_eq!(r, Err(SendError::FlusherDown));
    }

    #[test]
    fn event_kind_discriminants_match_migration() {
        // The events_raw schema column `kind` is a smallint that
        // stores 0/1/2 — keep the wire enum in sync.
        assert_eq!(EventKind::Decision as i16, 0);
        assert_eq!(EventKind::Impression as i16, 1);
        assert_eq!(EventKind::Click as i16, 2);
    }
}
