//! Ad selection algorithm.
//!
//! Phase 3.15. Pure-Rust, no DB. Operates over in-memory snapshot
//! shapes that the snapshot loader (Phase 3.17) will populate.
//! The decision endpoint (Phase 3.18) wires this onto a request
//! context and the snapshot.
//!
//! Spec: `REQUIREMENTS.md` § 6.1 — selection is seven steps:
//!
//!   1. Filter to flights active at request time (date window).
//!   2. Filter to ads matching `siteId`/`zoneIds`/`adTypes`.
//!   3. Apply `force.*` overrides (debug only).
//!   4. Apply `block.*` exclusions.
//!   5. Group by priority tier; highest non-empty tier wins.
//!   6. Within tier: weighted random by ad weight.
//!   7. Mint HMAC-signed click and impression URLs.
//!
//! Steps 1, 2, 4, 5, 6 live here. Step 3 (force) is a handler
//! concern (audit, role gate). Step 7 (HMAC) lives in `hmac.rs`
//! (Phase 3.16).

#![allow(dead_code)]

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// One row out of the flight side of the snapshot. Field set is
/// the strict minimum needed for selection — when the snapshot
/// (Phase 3.17) lands it will use a richer struct that converts
/// into this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flight {
    pub id: i64,
    pub campaign_id: i64,
    pub advertiser_id: i64,
    pub priority_tier: i32,
    /// Inclusive start, in epoch millis. `None` means "no lower
    /// bound" (always-on).
    pub start_ms: Option<i64>,
    /// Inclusive end. `None` means "no upper bound."
    pub end_ms: Option<i64>,
    /// Empty = "any site in the project."
    pub site_ids: Vec<i64>,
    pub zone_ids: Vec<i64>,
    pub ad_types: Vec<i64>,
    pub is_active: bool,
}

/// One row out of the ad side of the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ad {
    pub id: i64,
    pub flight_id: i64,
    pub weight: i32,
    pub is_active: bool,
}

/// Request context driving the filter step.
#[derive(Clone, Debug, Default)]
pub struct Placement {
    pub site_id: i64,
    pub zone_ids: Vec<i64>,
    pub ad_types: Vec<i64>,
    pub count: u32,
}

/// `block.*` exclusions from the request body. All ids are
/// project-scoped.
#[derive(Clone, Debug, Default)]
pub struct BlockSet {
    pub creative_ids: Vec<i64>,
    pub advertiser_ids: Vec<i64>,
    pub campaign_ids: Vec<i64>,
}

/// One eligible ad after filter+priority+block, before weighted
/// random.
#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    pub ad: &'a Ad,
    pub flight: &'a Flight,
}

/// Step 1 + 2 + 4: filter flights by date window and placement
/// targeting, then ads by `is_active`, then exclude blocked
/// advertisers/campaigns. (Creative-level blocks are evaluated
/// at the decision-handler boundary because the snapshot's ad
/// row carries `creative_id` only at the consumer, not here.)
pub fn filter<'a>(
    flights: &'a [Flight],
    ads: &'a [Ad],
    placement: &Placement,
    block: &BlockSet,
    now_ms: i64,
) -> Vec<Candidate<'a>> {
    let mut out = Vec::new();
    for f in flights {
        if !f.is_active {
            continue;
        }
        if !active_at(f, now_ms) {
            continue;
        }
        if !targets_placement(f, placement) {
            continue;
        }
        if block.advertiser_ids.contains(&f.advertiser_id) {
            continue;
        }
        if block.campaign_ids.contains(&f.campaign_id) {
            continue;
        }
        for ad in ads.iter().filter(|a| a.is_active && a.flight_id == f.id) {
            out.push(Candidate { ad, flight: f });
        }
    }
    out
}

/// Step 5: keep only the highest non-empty priority tier. Lower
/// numeric tier = higher priority (matches the v0 default
/// `house=1, standard=2, backfill=3` ordering).
pub fn priority<'a>(candidates: &[Candidate<'a>]) -> Vec<Candidate<'a>> {
    let Some(top) = candidates.iter().map(|c| c.flight.priority_tier).min() else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|c| c.flight.priority_tier == top)
        .cloned()
        .collect()
}

/// Step 6: weighted-random selection of `count` ads with a seeded
/// RNG. Sampling is *with replacement* across calls only when
/// the caller asks for `count > 1`; within a single call we sample
/// without replacement (an ad won't be returned twice for the
/// same placement). Returns up to `count` distinct ads.
pub fn weighted_random<'a>(
    candidates: &[Candidate<'a>],
    count: u32,
    seed: u64,
) -> Vec<Candidate<'a>> {
    if candidates.is_empty() || count == 0 {
        return Vec::new();
    }
    let mut rng = StdRng::seed_from_u64(seed);
    // Copy candidates into a working pool we can mutate. Using
    // indices is enough — we modify a parallel weight vector and
    // mark exhausted slots with weight 0.
    let mut weights: Vec<i64> = candidates
        .iter()
        .map(|c| c.ad.weight.max(0) as i64)
        .collect();
    let target = count.min(candidates.len() as u32) as usize;
    let mut picks: Vec<Candidate<'a>> = Vec::with_capacity(target);
    for _ in 0..target {
        let total: i64 = weights.iter().sum();
        if total <= 0 {
            break;
        }
        let mut roll = rng.gen_range(0..total);
        let mut chosen = None;
        for (i, w) in weights.iter().enumerate() {
            if *w == 0 {
                continue;
            }
            if roll < *w {
                chosen = Some(i);
                break;
            }
            roll -= *w;
        }
        let Some(i) = chosen else { break };
        picks.push(candidates[i].clone());
        weights[i] = 0;
    }
    picks
}

fn active_at(f: &Flight, now_ms: i64) -> bool {
    if let Some(start) = f.start_ms {
        if now_ms < start {
            return false;
        }
    }
    if let Some(end) = f.end_ms {
        if now_ms > end {
            return false;
        }
    }
    true
}

fn targets_placement(f: &Flight, p: &Placement) -> bool {
    // Empty arrays in the flight = "any" (REQUIREMENTS.md § 6.1).
    if !f.site_ids.is_empty() && !f.site_ids.contains(&p.site_id) {
        return false;
    }
    if !f.zone_ids.is_empty() && !p.zone_ids.iter().any(|z| f.zone_ids.contains(z)) {
        return false;
    }
    // ad_types on the placement is required and non-empty per
    // API.md § 1; we just need *one* match.
    if !f.ad_types.is_empty() && !p.ad_types.iter().any(|t| f.ad_types.contains(t)) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_flight(id: i64, prio: i32, sites: &[i64], zones: &[i64], ats: &[i64]) -> Flight {
        Flight {
            id,
            campaign_id: id * 10,
            advertiser_id: id * 100,
            priority_tier: prio,
            start_ms: None,
            end_ms: None,
            site_ids: sites.to_vec(),
            zone_ids: zones.to_vec(),
            ad_types: ats.to_vec(),
            is_active: true,
        }
    }

    fn make_ad(id: i64, flight_id: i64, weight: i32) -> Ad {
        Ad {
            id,
            flight_id,
            weight,
            is_active: true,
        }
    }

    #[test]
    fn filter_drops_inactive_flights() {
        let mut f = make_flight(1, 2, &[], &[], &[16]);
        f.is_active = false;
        let flights = [f];
        let ads = vec![make_ad(10, 1, 1)];
        let p = Placement {
            site_id: 1,
            ad_types: vec![16],
            count: 1,
            ..Default::default()
        };
        let block = BlockSet::default();
        let r = filter(&flights, &ads, &p, &block, 0);
        assert!(r.is_empty());
    }

    #[test]
    fn filter_respects_date_window() {
        let mut f = make_flight(1, 2, &[], &[], &[16]);
        f.start_ms = Some(100);
        f.end_ms = Some(200);
        let flights = [f];
        let ads = vec![make_ad(10, 1, 1)];
        let p = Placement {
            site_id: 1,
            ad_types: vec![16],
            count: 1,
            ..Default::default()
        };
        let block = BlockSet::default();
        assert!(filter(&flights, &ads, &p, &block, 50).is_empty());
        assert_eq!(filter(&flights, &ads, &p, &block, 150).len(), 1);
        assert!(filter(&flights, &ads, &p, &block, 500).is_empty());
    }

    #[test]
    fn filter_targets_site_zone_ad_type() {
        let flights = [
            make_flight(1, 2, &[1], &[10], &[16]),
            make_flight(2, 2, &[2], &[10], &[16]),
            make_flight(3, 2, &[1], &[20], &[16]),
            make_flight(4, 2, &[1], &[10], &[99]),
            make_flight(5, 2, &[], &[], &[16]), // any-any
        ];
        let ads: Vec<Ad> = (1..=5).map(|i| make_ad(i * 10, i, 1)).collect();
        let p = Placement {
            site_id: 1,
            zone_ids: vec![10],
            ad_types: vec![16],
            count: 1,
        };
        let block = BlockSet::default();
        let r = filter(&flights, &ads, &p, &block, 0);
        let ids: Vec<i64> = r.iter().map(|c| c.flight.id).collect();
        assert_eq!(ids, vec![1, 5]);
    }

    #[test]
    fn filter_drops_blocked_advertisers_and_campaigns() {
        let flights = [
            make_flight(1, 2, &[], &[], &[16]),
            make_flight(2, 2, &[], &[], &[16]),
        ];
        // flights[0].advertiser_id = 100, flights[1].advertiser_id = 200.
        let ads = vec![make_ad(10, 1, 1), make_ad(20, 2, 1)];
        let p = Placement {
            site_id: 1,
            ad_types: vec![16],
            count: 1,
            ..Default::default()
        };
        let block = BlockSet {
            advertiser_ids: vec![200],
            ..Default::default()
        };
        let r = filter(&flights, &ads, &p, &block, 0);
        let ids: Vec<i64> = r.iter().map(|c| c.flight.id).collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn priority_keeps_only_top_tier() {
        let flights = [
            make_flight(1, 1, &[], &[], &[16]), // top
            make_flight(2, 2, &[], &[], &[16]),
            make_flight(3, 1, &[], &[], &[16]), // top
        ];
        let ads = vec![make_ad(10, 1, 1), make_ad(20, 2, 1), make_ad(30, 3, 1)];
        let p = Placement {
            site_id: 1,
            ad_types: vec![16],
            count: 1,
            ..Default::default()
        };
        let block = BlockSet::default();
        let cands = filter(&flights, &ads, &p, &block, 0);
        let top = priority(&cands);
        let mut tier_ids: Vec<i64> = top.iter().map(|c| c.flight.id).collect();
        tier_ids.sort();
        assert_eq!(tier_ids, vec![1, 3]);
    }

    #[test]
    fn priority_empty_returns_empty() {
        let cands: Vec<Candidate<'_>> = Vec::new();
        assert!(priority(&cands).is_empty());
    }

    #[test]
    fn weighted_random_seeded_is_deterministic() {
        let f1 = make_flight(1, 1, &[], &[], &[16]);
        let f2 = make_flight(2, 1, &[], &[], &[16]);
        let ads = [make_ad(10, 1, 1), make_ad(20, 2, 9)];
        let cands = vec![
            Candidate {
                ad: &ads[0],
                flight: &f1,
            },
            Candidate {
                ad: &ads[1],
                flight: &f2,
            },
        ];
        let pick_a = weighted_random(&cands, 1, 42);
        let pick_b = weighted_random(&cands, 1, 42);
        assert_eq!(pick_a[0].ad.id, pick_b[0].ad.id);
    }

    #[test]
    fn weighted_random_count_sampling_without_replacement() {
        let f = make_flight(1, 1, &[], &[], &[16]);
        let ads: Vec<Ad> = (1..=4).map(|i| make_ad(i * 10, 1, 1)).collect();
        let cands: Vec<Candidate<'_>> = ads
            .iter()
            .map(|a| Candidate { ad: a, flight: &f })
            .collect();
        let picks = weighted_random(&cands, 3, 7);
        assert_eq!(picks.len(), 3);
        let mut ids: Vec<i64> = picks.iter().map(|c| c.ad.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3, "ads must be distinct within one placement");
    }

    #[test]
    fn weighted_random_zero_weights_returns_nothing() {
        let f = make_flight(1, 1, &[], &[], &[16]);
        let ads = [make_ad(10, 1, 0), make_ad(20, 1, 0)];
        let cands: Vec<Candidate<'_>> = ads
            .iter()
            .map(|a| Candidate { ad: a, flight: &f })
            .collect();
        assert!(weighted_random(&cands, 5, 1).is_empty());
    }

    #[test]
    fn priority_dominates_weight() {
        // High-weight low-priority ad must lose to any active
        // higher-priority ad — invariant from REQUIREMENTS.md
        // § 6.1 step 5.
        let flights = [
            make_flight(1, 1, &[], &[], &[16]),
            make_flight(2, 5, &[], &[], &[16]),
        ];
        let ads = vec![
            make_ad(10, 1, 1),         // tier 1, weight 1
            make_ad(20, 2, 1_000_000), // tier 5, huge weight
        ];
        let p = Placement {
            site_id: 1,
            ad_types: vec![16],
            count: 1,
            ..Default::default()
        };
        let block = BlockSet::default();
        let cands = filter(&flights, &ads, &p, &block, 0);
        let top = priority(&cands);
        let pick = weighted_random(&top, 1, 12345);
        assert_eq!(pick[0].ad.id, 10);
    }
}
