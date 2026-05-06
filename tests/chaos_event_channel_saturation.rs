//! Chaos: event channel saturation.
//!
//! Phase 4.7 skeleton. Pairs with `REQUIREMENTS.md` § 10.9 row 4
//! and `TESTING.md` § 9 row 5.
//!
//! Asserted behavior: decision endpoint **fails fast** at 503 —
//! events would otherwise drop silently; pings (`/e/...`) still
//! succeed at signature-verify level but may be dropped if the
//! channel is fully wedged. Recovery clears the failure.
//!
//! Injection: `tc qdisc add dev eth0 root tbf rate 1bit
//! burst 1500 latency 1s` from the chaos-injector to throttle the
//! flusher's outbound traffic to Postgres. With the channel
//! capacity at 10000 events, a few seconds of decision load fills
//! it.

#[tokio::test]
#[ignore = "chaos suite — needs the tc-capable injector + compose harness. Activate by flipping #[ignore] once the harness lands."]
async fn event_channel_saturation() {
    // 1. compose up
    // 2. injector: tc throttle on knievel→postgres traffic to 1 bit/s
    // 3. drive decisions at high rate from the load harness
    // 4. assert: channel reaches capacity within ~10 s
    // 5. assert: subsequent decisions return 503
    //    event_channel_saturated
    // 6. injector: tc qdisc del root
    // 7. assert: channel drains, decisions return to 200
}
