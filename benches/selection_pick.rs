//! Criterion micro-benchmark for the decision selection inner
//! loop (`TESTING.md` § 8). Measures `filter` → `priority` →
//! `weighted_random` for representative pool sizes — the
//! library's own test fixtures aren't pub, so the bench
//! constructs its own minimal inputs.
//!
//! Run:
//!
//!     cargo bench --bench selection_pick
//!
//! Targets a single decision request against a snapshot of N
//! flights × M ads, scaled across realistic input sizes.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use knievel::selection::{filter, priority, weighted_random, Ad, BlockSet, Flight, Placement};

fn make_flight(id: i64, prio: i32) -> Flight {
    Flight {
        id,
        campaign_id: id * 10,
        advertiser_id: id * 100,
        priority_tier: prio,
        start_ms: None,
        end_ms: None,
        site_ids: vec![1],
        zone_ids: vec![],
        ad_types: vec![1],
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

fn synthesize(n_flights: usize, ads_per_flight: usize) -> (Vec<Flight>, Vec<Ad>) {
    let mut flights = Vec::with_capacity(n_flights);
    let mut ads = Vec::with_capacity(n_flights * ads_per_flight);
    for i in 0..n_flights {
        let id = (i + 1) as i64;
        // Distribute priorities across 5 tiers per
        // `taxonomy::seed_default_taxonomy`.
        let prio = (i % 5) as i32 + 1;
        flights.push(make_flight(id, prio));
        for j in 0..ads_per_flight {
            let ad_id = id * 100 + (j as i64);
            // Mixed weights so weighted_random has work to do.
            let weight = ((i + j) % 50 + 1) as i32;
            ads.push(make_ad(ad_id, id, weight));
        }
    }
    (flights, ads)
}

fn bench_select(c: &mut Criterion) {
    let placement = Placement {
        site_id: 1,
        zone_ids: vec![],
        ad_types: vec![1],
        count: 1,
    };
    let block = BlockSet::default();

    let mut group = c.benchmark_group("selection_pick");
    for &(n_flights, ads_per_flight) in &[(10, 1), (100, 1), (1_000, 1), (10_000, 1), (100_000, 1)]
    {
        let (flights, ads) = synthesize(n_flights, ads_per_flight);
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::from_parameter(n_flights),
            &(flights, ads),
            |b, (flights, ads)| {
                b.iter(|| {
                    let cands = filter(black_box(flights), black_box(ads), &placement, &block, 0);
                    let top = priority(&cands);
                    let _ = weighted_random(&top, 1, 0);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_select);
criterion_main!(benches);
