//! Criterion micro-benchmark for the HMAC-signed event URL
//! verifier (`TESTING.md` § 8). Measures the per-request cost of
//! verifying an `/e/i/{signed}` URL against the current secret
//! and (optionally) the rotation-window predecessor.
//!
//! Run:
//!
//!     cargo bench --bench hmac_verify
//!
//! The verifier is on every event-endpoint request, so its cost
//! is multiplied across the highest-volume path in the system.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

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

fn bench_verify(c: &mut Criterion) {
    let secret_now = b"secret-current-32-bytes-of-data!!!".to_vec();
    let secret_prev = b"secret-previous-32-bytes-of-data!!".to_vec();
    let signed = sign(&payload(), &secret_now);

    c.bench_function("hmac::verify hot (current secret)", |b| {
        b.iter(|| {
            let _ = verify(black_box(&signed), &secret_now, None, NOW_SECS, TTL_SECS);
        });
    });

    c.bench_function("hmac::verify cold (current + previous fallback)", |b| {
        // Sign with the previous secret to force the fallback path.
        let signed_prev = sign(&payload(), &secret_prev);
        b.iter(|| {
            let _ = verify(
                black_box(&signed_prev),
                &secret_now,
                Some(&secret_prev),
                NOW_SECS,
                TTL_SECS,
            );
        });
    });
}

criterion_group!(benches, bench_verify);
criterion_main!(benches);
