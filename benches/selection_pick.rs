//! Criterion micro-benchmark for the decision selection inner
//! loop (`TESTING.md` § 8). Measures `filter` → `priority` →
//! `weighted_random` for representative pool sizes — the
//! lower bound on what `decide_pure` does per placement, before
//! HMAC sign + response shape construction.
//!
//! Phase 5.7 — original sweep over flight count at 100% match.
//! Phase 5.9 — extends with selectivity (1%, 10%, 50%, 100%) so
//! we can decompose filter cost from priority + weighted_random
//! cost.
//!
//! Run:
//!
//!     cargo bench --bench selection_pick

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use knievel::selection::{filter, priority, weighted_random, BlockSet, Placement};

mod common;
use common::{synthesize_snapshot, SnapshotShape, AD_TYPE, NOW_MS, SITE_ID};

const FLIGHT_AXIS: &[usize] = &[10, 100, 1_000, 10_000, 100_000];
const SELECTIVITY_AXIS: &[f32] = &[0.01, 0.10, 0.50, 1.00];

fn bench_select(c: &mut Criterion) {
    let placement = Placement {
        site_id: SITE_ID,
        zone_ids: vec![],
        ad_types: vec![AD_TYPE],
        count: 1,
    };
    let block = BlockSet::default();

    let mut group = c.benchmark_group("selection_pick");
    for &n_flights in FLIGHT_AXIS {
        for &selectivity in SELECTIVITY_AXIS {
            let snap = synthesize_snapshot(SnapshotShape::new(n_flights, 1, selectivity));
            let id = format!("n{n_flights}/sel{:.2}", selectivity);
            group.throughput(Throughput::Elements(1));
            group.bench_with_input(
                BenchmarkId::from_parameter(id),
                &(snap.flights, snap.ads),
                |b, (flights, ads)| {
                    b.iter(|| {
                        let cands = filter(
                            black_box(flights),
                            black_box(ads),
                            &placement,
                            &block,
                            NOW_MS,
                        );
                        let top = priority(&cands);
                        let _ = weighted_random(&top, 1, 0);
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_select);
criterion_main!(benches);
