//! End-to-end criterion bench for the decision pure path
//! (Phase 5.9). Drives `decisions::decide_pure` directly — no
//! HTTP, no Postgres, no `AppState` — so the measurement covers
//! filter + priority + weighted_random + HMAC sign + response
//! shape construction + event row composition. Mirrors what the
//! REQUIREMENTS.md § 9.1 SLO targets actually capture.
//!
//! Sweeps three axes from `bench/results/SCHEMA.md`:
//!
//!   * placement count (1, 4, 10) — REQUIREMENTS.md § 9.1 quotes
//!     p50 @ 1 placement and p99 @ 4 placements.
//!   * post-filter selectivity (1%, 10%, 50%, 100%) — measures
//!     how much of the linear-in-N cost is filter overhead vs.
//!     priority + weighted_random + HMAC sign.
//!   * snapshot size (100, 1k, 10k flights) — keeps the matrix
//!     bounded; the inner-loop bench (`selection_pick`) sweeps
//!     up to 100k for the breakpoint.
//!
//! Plus two side fixtures that exercise paths the matrix would
//! otherwise miss:
//!
//!   * `force_override` — `force.adId` substitution path.
//!   * `blocked` — heavy block-set, no other targeting.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use knievel::decisions::decide_pure;

mod common;
use common::{
    bench_principal, synthesize_blocked_request, synthesize_force_request, synthesize_request,
    synthesize_snapshot, SnapshotShape, NOW_MS, PROJECT_ID, SNAPSHOT_VERSION,
};

const FLIGHT_AXIS: &[usize] = &[100, 1_000, 10_000];
const PLACEMENT_AXIS: &[usize] = &[1, 4, 10];
const SELECTIVITY_AXIS: &[f32] = &[0.01, 0.10, 0.50, 1.00];

fn bench_matrix(c: &mut Criterion) {
    let principal = bench_principal();
    let mut group = c.benchmark_group("decide_pure");

    for &n_flights in FLIGHT_AXIS {
        for &selectivity in SELECTIVITY_AXIS {
            let snap = synthesize_snapshot(SnapshotShape::new(n_flights, 1, selectivity));
            for &n_placements in PLACEMENT_AXIS {
                let req = synthesize_request(n_placements);
                let id = format!("n{n_flights}/p{n_placements}/sel{:.2}", selectivity);
                group.throughput(Throughput::Elements(n_placements as u64));
                group.bench_with_input(
                    BenchmarkId::from_parameter(id),
                    &(snap.clone_box(), req),
                    |b, (snap, req)| {
                        b.iter(|| {
                            let outcome = decide_pure(
                                black_box(snap),
                                SNAPSHOT_VERSION,
                                &principal,
                                PROJECT_ID,
                                black_box(req),
                                NOW_MS,
                                false,
                            )
                            .expect("decide_pure rejected fixture input");
                            black_box(outcome);
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

fn bench_force_override(c: &mut Criterion) {
    let principal = bench_principal();
    // 1k flights @ 100% selectivity is the realistic-ish steady
    // state; force.* substitution short-circuits past selection.
    let snap = synthesize_snapshot(SnapshotShape::new(1_000, 1, 1.0));
    // Pick an ad that's guaranteed to exist (id 100 = flight 1's
    // first ad per `synthesize_snapshot` numbering).
    let req = synthesize_force_request(100);
    c.bench_function("decide_pure/force_override", |b| {
        b.iter(|| {
            let outcome = decide_pure(
                black_box(&snap),
                SNAPSHOT_VERSION,
                &principal,
                PROJECT_ID,
                black_box(&req),
                NOW_MS,
                true,
            )
            .expect("decide_pure rejected fixture input");
            black_box(outcome);
        });
    });
}

fn bench_blocked(c: &mut Criterion) {
    let principal = bench_principal();
    let snap = synthesize_snapshot(SnapshotShape::new(1_000, 1, 1.0));
    // 1k advertisers blocked — every flight in the snapshot has
    // a unique advertiser_id, so this kills the filter's "did
    // this advertiser appear in block" check.
    let req = synthesize_blocked_request(1, 1_000);
    c.bench_function("decide_pure/blocked", |b| {
        b.iter(|| {
            let outcome = decide_pure(
                black_box(&snap),
                SNAPSHOT_VERSION,
                &principal,
                PROJECT_ID,
                black_box(&req),
                NOW_MS,
                false,
            )
            .expect("decide_pure rejected fixture input");
            black_box(outcome);
        });
    });
}

// `ProjectSnapshot` doesn't `Clone`, so the matrix loop calls
// this manual deep-copy. Cheaper than re-synthesizing per
// fixture entry; identical content.
trait CloneBox {
    fn clone_box(&self) -> Self;
}

impl CloneBox for knievel::snapshot::ProjectSnapshot {
    fn clone_box(&self) -> Self {
        knievel::snapshot::ProjectSnapshot {
            project_id: self.project_id.clone(),
            org_id_for_event: self.org_id_for_event.clone(),
            flights: self.flights.clone(),
            ads: self.ads.clone(),
            sites: self.sites.clone(),
            zones: self.zones.clone(),
            click_through_urls: self.click_through_urls.clone(),
            hmac_secret: self.hmac_secret.clone(),
            hmac_secret_previous: self.hmac_secret_previous.clone(),
            allow_force_decision: self.allow_force_decision,
        }
    }
}

criterion_group!(benches, bench_matrix, bench_force_override, bench_blocked);
criterion_main!(benches);
