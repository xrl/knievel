//! iai-callgrind bench for HMAC verify (Phase 5.9). Companion to
//! the wall-clock `hmac_verify.rs` bench — same two cases, same
//! fixtures, but reporting deterministic instruction counts.
//!
//! The verifier runs on every event-endpoint hit
//! (`/e/i/{signed}` + `/e/c/{signed}`), so its per-request cost
//! is multiplied across the highest-volume path in the system.
//! Tracking instruction count over time makes signature-perf
//! regressions unambiguous (a 10% wall-clock regression on a
//! noisy runner is hard to call; a 10% instruction regression
//! is a real change in the code).
//!
//! Run:
//!
//!     sudo apt-get install -y valgrind
//!     cargo bench --bench iai_hmac

use std::sync::OnceLock;

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};

use knievel::hmac::{sign, verify, SignaturePayload};

const TTL_SECS: u64 = 60 * 60 * 24 * 30;
const NOW_SECS: u64 = 1_700_000_001;

fn payload() -> SignaturePayload {
    SignaturePayload {
        project_id: "pj_AbCdEfGhIj".into(),
        ad_id: 12_345,
        creative_id: 67_890,
        placement_id_hash: [0u8; 16],
        issued_at_secs: 1_700_000_000,
        nonce: [0u8; 8],
    }
}

fn secret_now() -> &'static [u8] {
    b"secret-current-32-bytes-of-data!!!"
}
fn secret_prev() -> &'static [u8] {
    b"secret-previous-32-bytes-of-data!!"
}

fn signed_now() -> &'static String {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| sign(&payload(), secret_now()))
}

fn signed_prev() -> &'static String {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| sign(&payload(), secret_prev()))
}

#[library_benchmark]
fn iai_hmac_verify_hot() {
    let _ = verify(
        black_box(signed_now()),
        secret_now(),
        None,
        NOW_SECS,
        TTL_SECS,
    );
}

#[library_benchmark]
fn iai_hmac_verify_cold() {
    let _ = verify(
        black_box(signed_prev()),
        secret_now(),
        Some(secret_prev()),
        NOW_SECS,
        TTL_SECS,
    );
}

library_benchmark_group!(
    name = hmac;
    benchmarks =
        iai_hmac_verify_hot,
        iai_hmac_verify_cold,
);

main!(library_benchmark_groups = hmac);
