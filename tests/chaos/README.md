# Chaos / degraded-mode suite

Phase 4.7 deliverable. Pairs 1:1 with `REQUIREMENTS.md` § 10.9 and
`TESTING.md` § 9. Runs **nightly** (and on `workflow_dispatch`),
not per-PR — chaos failures open issues but don't block tags.

## What's here

```
tests/chaos/
├── README.md                          (this file)
├── compose.yaml                       (the harness — Phase 4.7+)
└── binaries/
    ├── chaos_db_writer_unreachable.rs
    ├── chaos_listen_drops.rs
    ├── chaos_notify_overflow.rs
    ├── chaos_aurora_failover.rs
    ├── chaos_event_channel_saturation.rs
    ├── chaos_jwks_unreachable.rs
    ├── chaos_pool_exhaustion.rs
    ├── chaos_leader_watchdog_miss.rs
    └── chaos_minio_midflight.rs
```

Each binary corresponds to one row of REQUIREMENTS.md § 10.9 and
one row of TESTING.md § 9. The skeleton lives in
`tests/chaos_*.rs` (one file per scenario, named to land at the
top of the binary list when sorted) so a reader can grep for the
failure mode and find the runnable test.

## Today (skeleton)

Each binary contains exactly one `#[tokio::test]` named after the
scenario, marked `#[ignore]`, with the injection mechanism named in
the `#[ignore]` reason. The skeleton stays so:

1. The grep-by-name from REQUIREMENTS.md § 10.9 / TESTING.md § 9
   works on day 1.
2. Activating a scenario is a focused PR — flip `#[ignore]`,
   wire the injection.
3. A new degraded-mode row in § 10.9 is required to land with a
   paired chaos test, per TESTING.md § 9 ("Every row of § 10.9 is
   paired with a chaos test here. New degraded-mode rows require a
   paired test before merging.").

## Injection mechanisms

The harness depends on infrastructure-level fault injection that
isn't realistic from a pure Rust integration test:

| Injection | Tool / mechanism |
|---|---|
| Drop traffic to a service | `iptables -A OUTPUT -p tcp --dport 5432 -j DROP` against the compose container |
| Throttle bandwidth | `tc qdisc add dev eth0 root tbf rate 1bit burst 1500 latency 1s` |
| Force-close a connection | side connection runs `pg_terminate_backend(<pid>)` |
| Kill a service | `docker compose kill <svc>` |
| Block an upstream | iptables rule on the wiremock container's port |
| Pause a process | `docker compose pause <svc>` for time-skew scenarios |

The compose `tests/chaos/compose.yaml` (lands with the first
scenario activation) layers these onto the Phase 4.1 reference
stack with two extras: a `chaos-injector` sidecar with
`NET_ADMIN` capabilities (so it can run `iptables` / `tc`), and
a `wiremock` service so the JWKS-unreachable scenario can flip
between served and unreachable on demand.

## Why nightly, not per-PR

Chaos tests are slow (multi-second sleeps to observe failure
states) and tightly coupled to OS-level tools (`iptables`, `tc`)
that need root inside the harness. Per `TESTING.md` § 12.8 they
open issues via `peter-evans/create-issue-from-file` rather than
gating release.

## Refs

- `REQUIREMENTS.md` § 10.9 (the row-by-row contract every test
  pairs with).
- `TESTING.md` § 9 (the suite-level brief), § 12.8 (failure
  policy: open issue, don't block).
