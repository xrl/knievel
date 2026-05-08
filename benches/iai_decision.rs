//! iai-callgrind benches for the decision pure path (Phase 5.9).
//!
//! Measures CPU instructions retired, L1 data-cache misses, last-
//! level cache misses, and branch mispredictions per
//! `decide_pure` call. Unlike criterion's wall-clock, these
//! counters are **deterministic across hardware** — identical
//! source on identical rustc emits identical instruction counts
//! whether you run on a 2 vCPU GitHub Actions runner or a
//! workstation. That's the property that makes
//! `bench/results/v<X>.json` deltas authoritative across
//! releases regardless of which runner ran them.
//!
//! Smaller fixture matrix than `decision_handler.rs` because
//! callgrind is ~10–50× slower than wall-clock execution.
//!
//! Run:
//!
//!     sudo apt-get install -y valgrind
//!     cargo bench --bench iai_decision

use std::sync::OnceLock;

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};

use knievel::auth::Principal;
use knievel::decisions::{decide_pure, DecisionsRequest};
use knievel::snapshot::ProjectSnapshot;

mod common;
use common::{
    bench_principal, synthesize_force_request, synthesize_request, synthesize_snapshot,
    SnapshotShape, NOW_MS, PROJECT_ID, SNAPSHOT_VERSION,
};

// Lazy-initialized fixtures so the cost of building the snapshot
// + request is amortized across the (single) measured call.
// callgrind counts every instruction in the function body, so
// keeping fixture construction outside the measurement window is
// the only way to attribute counts cleanly to `decide_pure`.

fn snap_100() -> &'static ProjectSnapshot {
    static S: OnceLock<ProjectSnapshot> = OnceLock::new();
    S.get_or_init(|| synthesize_snapshot(SnapshotShape::new(100, 1, 1.0)))
}
fn snap_1k_sel10() -> &'static ProjectSnapshot {
    static S: OnceLock<ProjectSnapshot> = OnceLock::new();
    S.get_or_init(|| synthesize_snapshot(SnapshotShape::new(1_000, 1, 0.10)))
}
fn snap_1k_sel100() -> &'static ProjectSnapshot {
    static S: OnceLock<ProjectSnapshot> = OnceLock::new();
    S.get_or_init(|| synthesize_snapshot(SnapshotShape::new(1_000, 1, 1.0)))
}

fn req_1() -> &'static DecisionsRequest {
    static R: OnceLock<DecisionsRequest> = OnceLock::new();
    R.get_or_init(|| synthesize_request(1))
}
fn req_4() -> &'static DecisionsRequest {
    static R: OnceLock<DecisionsRequest> = OnceLock::new();
    R.get_or_init(|| synthesize_request(4))
}
fn req_force() -> &'static DecisionsRequest {
    static R: OnceLock<DecisionsRequest> = OnceLock::new();
    R.get_or_init(|| synthesize_force_request(100))
}

fn principal() -> &'static Principal {
    static P: OnceLock<Principal> = OnceLock::new();
    P.get_or_init(bench_principal)
}

#[library_benchmark]
fn iai_decide_n100_p1_sel100() {
    let outcome = decide_pure(
        black_box(snap_100()),
        SNAPSHOT_VERSION,
        principal(),
        PROJECT_ID,
        black_box(req_1()),
        NOW_MS,
        false,
    )
    .expect("decide_pure rejected fixture input");
    black_box(outcome);
}

#[library_benchmark]
fn iai_decide_n1k_p1_sel10() {
    let outcome = decide_pure(
        black_box(snap_1k_sel10()),
        SNAPSHOT_VERSION,
        principal(),
        PROJECT_ID,
        black_box(req_1()),
        NOW_MS,
        false,
    )
    .expect("decide_pure rejected fixture input");
    black_box(outcome);
}

#[library_benchmark]
fn iai_decide_n1k_p1_sel100() {
    let outcome = decide_pure(
        black_box(snap_1k_sel100()),
        SNAPSHOT_VERSION,
        principal(),
        PROJECT_ID,
        black_box(req_1()),
        NOW_MS,
        false,
    )
    .expect("decide_pure rejected fixture input");
    black_box(outcome);
}

#[library_benchmark]
fn iai_decide_n1k_p4_sel100() {
    let outcome = decide_pure(
        black_box(snap_1k_sel100()),
        SNAPSHOT_VERSION,
        principal(),
        PROJECT_ID,
        black_box(req_4()),
        NOW_MS,
        false,
    )
    .expect("decide_pure rejected fixture input");
    black_box(outcome);
}

#[library_benchmark]
fn iai_decide_force_override() {
    let outcome = decide_pure(
        black_box(snap_1k_sel100()),
        SNAPSHOT_VERSION,
        principal(),
        PROJECT_ID,
        black_box(req_force()),
        NOW_MS,
        true,
    )
    .expect("decide_pure rejected fixture input");
    black_box(outcome);
}

library_benchmark_group!(
    name = decide;
    benchmarks =
        iai_decide_n100_p1_sel100,
        iai_decide_n1k_p1_sel10,
        iai_decide_n1k_p1_sel100,
        iai_decide_n1k_p4_sel100,
        iai_decide_force_override,
);

main!(library_benchmark_groups = decide);
