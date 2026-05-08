//! Heap-profile bench for the decision pure path (Phase 5.9).
//!
//! Runs a fixed workload (1k decisions over the standard
//! 1k-flight @ 100% selectivity fixture, single placement) under
//! `dhat-rs`'s heap profiler and emits a JSON summary of total
//! bytes allocated, total alloc count, peak bytes, and peak alloc
//! count. The orchestrator (`xtask bench-all`) consumes the JSON.
//!
//! Wall-clock and instruction counts both ignore allocator
//! pressure; this bench is the only signal that flags "decision
//! grew an unexpected `String::from` inside the hot loop." Run
//! it with the `dhat-heap` feature to swap the global allocator;
//! without the feature, the bench is a no-op printing zeros so
//! the orchestrator's call site doesn't break when the feature
//! is off.
//!
//! Run:
//!
//!     cargo bench --bench dhat_decision --features dhat-heap

#![allow(dead_code, unused_imports)]

use knievel::decisions::decide_pure;

mod common;
use common::{
    bench_principal, synthesize_request, synthesize_snapshot, SnapshotShape, NOW_MS, PROJECT_ID,
    SNAPSHOT_VERSION,
};

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "dhat-heap")]
const ITERATIONS: usize = 1_000;

#[cfg(feature = "dhat-heap")]
fn workload() {
    let snap = synthesize_snapshot(SnapshotShape::new(1_000, 1, 1.0));
    let req = synthesize_request(1);
    let principal = bench_principal();
    for _ in 0..ITERATIONS {
        let outcome = decide_pure(
            &snap,
            SNAPSHOT_VERSION,
            &principal,
            PROJECT_ID,
            &req,
            NOW_MS,
            false,
        )
        .expect("decide_pure rejected fixture input");
        // Drop outcome explicitly so the deallocations happen
        // inside the measurement window.
        drop(outcome);
    }
}

#[cfg(feature = "dhat-heap")]
fn main() {
    let profiler = dhat::Profiler::builder().testing().build();
    workload();
    let stats = dhat::HeapStats::get();
    drop(profiler);

    // Per-decision averages: dhat counts global heap activity, so
    // we attribute it to ITERATIONS to get a per-call view.
    let total_blocks = stats.total_blocks;
    let total_bytes = stats.total_bytes;
    let max_blocks = stats.max_blocks;
    let max_bytes = stats.max_bytes;
    let blocks_per_decision = total_blocks as f64 / ITERATIONS as f64;
    let bytes_per_decision = total_bytes as f64 / ITERATIONS as f64;

    let json = serde_json::json!({
        "bench": "dhat_decision",
        "iterations": ITERATIONS,
        "fixture": "n1000/p1/sel1.00",
        "total_blocks": total_blocks,
        "total_bytes": total_bytes,
        "peak_blocks": max_blocks,
        "peak_bytes": max_bytes,
        "blocks_per_decision": blocks_per_decision,
        "bytes_per_decision": bytes_per_decision,
    });
    println!("DHAT_JSON_BEGIN");
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
    println!("DHAT_JSON_END");
}

#[cfg(not(feature = "dhat-heap"))]
fn main() {
    eprintln!(
        "dhat_decision: rebuild with `--features dhat-heap` to capture heap stats. \
         Emitting an empty placeholder so xtask bench-all keeps moving."
    );
    let json = serde_json::json!({
        "bench": "dhat_decision",
        "iterations": 0,
        "fixture": "n1000/p1/sel1.00",
        "total_blocks": 0,
        "total_bytes": 0,
        "peak_blocks": 0,
        "peak_bytes": 0,
        "blocks_per_decision": 0.0,
        "bytes_per_decision": 0.0,
        "note": "feature dhat-heap was off; rerun with --features dhat-heap"
    });
    println!("DHAT_JSON_BEGIN");
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
    println!("DHAT_JSON_END");
}
