//! Shared bench fixtures (Phase 5.9).
//!
//! All benches in `benches/` lean on this module to synthesize a
//! `ProjectSnapshot` + `DecisionsRequest` of a given shape. Keeps
//! the per-bench files focused on what they're measuring rather
//! than re-deriving the fixture matrix.
//!
//! The fixture matrix is documented in `bench/results/v0.1.md`;
//! anything benched against an axis (flights, placements,
//! selectivity) lives here so cross-bench comparisons are
//! apples-to-apples.

#![allow(dead_code)]

use std::collections::HashMap;

use knievel::auth::{Principal, Role, Scope, TokenType};
use knievel::decisions::{DecisionPlacement, DecisionsRequest, ForceOverride};
use knievel::selection::{Ad, Flight};
use knievel::snapshot::{ProjectSnapshot, SnapshotSite};

/// Fixed project id used by every fixture so HMAC tags are
/// deterministic across runs.
pub const PROJECT_ID: &str = "pj_bench0000001";
/// Fixed org id paired with `PROJECT_ID`.
pub const ORG_ID: &str = "org_bench";
/// All synthetic flights target this site id; placements ask for
/// the same.
pub const SITE_ID: i64 = 1;
/// All synthetic flights and placements use this single ad type.
pub const AD_TYPE: i64 = 1;
/// Stable HMAC secret for sign+verify benches.
pub const HMAC_SECRET: &[u8] = b"benchmark-secret-32-bytes-of-data!";
/// Stable timestamp for benches so HMAC `issued_at_secs` doesn't
/// drift between runs.
pub const NOW_MS: i64 = 1_700_000_000_000;
/// `config_version` returned in the synthetic snapshot.
pub const SNAPSHOT_VERSION: i64 = 42;

/// Shape knob for `synthesize_snapshot`. `selectivity` is the
/// fraction of flights that pass the placement's targeting filter
/// (rest are made to mismatch on `site_id` so they're dropped in
/// the filter step). Real production snapshots see ~10% post-filter
/// eligibility; we sweep 1%, 10%, 50%, 100% to measure how much
/// of selection's cost is filter overhead.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotShape {
    pub n_flights: usize,
    pub ads_per_flight: usize,
    /// 0.0..=1.0 — fraction of flights that match the placement.
    pub selectivity: f32,
}

impl SnapshotShape {
    pub const fn new(n_flights: usize, ads_per_flight: usize, selectivity: f32) -> Self {
        Self {
            n_flights,
            ads_per_flight,
            selectivity,
        }
    }
}

/// Build a `ProjectSnapshot` of `shape` flights/ads. The
/// non-matching flights target a distinct site id so the filter
/// drops them — this measures the realistic case where
/// selection's hot path scans flights that never make it past
/// filter, not the optimistic case where every row matches.
pub fn synthesize_snapshot(shape: SnapshotShape) -> ProjectSnapshot {
    let SnapshotShape {
        n_flights,
        ads_per_flight,
        selectivity,
    } = shape;
    let n_match = ((n_flights as f32) * selectivity).round() as usize;

    let mut flights = Vec::with_capacity(n_flights);
    let mut ads = Vec::with_capacity(n_flights * ads_per_flight);
    for i in 0..n_flights {
        let id = (i + 1) as i64;
        let prio = (i % 5) as i32 + 1;
        let target_site = if i < n_match { SITE_ID } else { SITE_ID + 999 };
        flights.push(Flight {
            id,
            campaign_id: id * 10,
            advertiser_id: id * 100,
            priority_tier: prio,
            start_ms: None,
            end_ms: None,
            site_ids: vec![target_site],
            zone_ids: vec![],
            ad_types: vec![AD_TYPE],
            is_active: true,
        });
        for j in 0..ads_per_flight {
            let ad_id = id * 100 + (j as i64);
            let weight = ((i + j) % 50 + 1) as i32;
            ads.push(Ad {
                id: ad_id,
                flight_id: id,
                weight,
                is_active: true,
            });
        }
    }

    ProjectSnapshot {
        project_id: PROJECT_ID.to_string(),
        org_id_for_event: ORG_ID.to_string(),
        flights,
        ads,
        sites: vec![SnapshotSite {
            id: SITE_ID,
            url: "bench.example".into(),
            aliases: vec![],
        }],
        zones: vec![],
        click_through_urls: HashMap::new(),
        hmac_secret: HMAC_SECRET.to_vec(),
        hmac_secret_previous: None,
        allow_force_decision: true,
    }
}

/// Build a `DecisionsRequest` with `n_placements` placements, all
/// targeting the synthetic site/ad-type. Placement ids are
/// `slot_0`, `slot_1`, ... so HMAC `placement_id_hash` is stable.
pub fn synthesize_request(n_placements: usize) -> DecisionsRequest {
    let placements = (0..n_placements)
        .map(|i| DecisionPlacement {
            id: format!("slot_{i}"),
            site_id: Some(SITE_ID),
            site_url: None,
            site_external_id: None,
            zone_ids: None,
            ad_types: vec![AD_TYPE],
            count: Some(1),
            force: None,
        })
        .collect();
    DecisionsRequest {
        context: None,
        placements,
        block: None,
        force_reason: None,
    }
}

/// Like `synthesize_request` but with a single placement carrying
/// a `force.adId`. Used by the force-override bench.
pub fn synthesize_force_request(forced_ad_id: i64) -> DecisionsRequest {
    DecisionsRequest {
        context: None,
        placements: vec![DecisionPlacement {
            id: "forced_slot".into(),
            site_id: Some(SITE_ID),
            site_url: None,
            site_external_id: None,
            zone_ids: None,
            ad_types: vec![AD_TYPE],
            count: Some(1),
            force: Some(ForceOverride {
                ad_id: Some(forced_ad_id),
                campaign_id: None,
                flight_id: None,
                creative_id: None,
            }),
        }],
        block: None,
        force_reason: Some("benchmark".into()),
    }
}

/// Like `synthesize_request` but with a heavy block-set covering
/// the first `n_blocked` advertiser ids. Measures filter cost
/// when the block list dominates.
pub fn synthesize_blocked_request(n_placements: usize, n_blocked: usize) -> DecisionsRequest {
    let mut req = synthesize_request(n_placements);
    let blocked = (1..=(n_blocked as i64)).map(|i| i * 100).collect();
    req.block = Some(knievel::decisions::DecisionBlock {
        creative_ids: None,
        advertiser_ids: Some(blocked),
        campaign_ids: None,
    });
    req
}

/// Stable principal used by every bench. The pure decide path
/// reads `principal.org_id` for events and `actor_id` for the
/// audit row (handler-only); benches accept both as opaque.
pub fn bench_principal() -> Principal {
    Principal {
        token_type: TokenType::Opaque,
        scope: Scope::Org,
        org_id: ORG_ID.into(),
        project_id: None,
        role: Role::Admin,
        actor_id: "tok_bench".into(),
    }
}
