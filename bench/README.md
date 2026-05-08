# `bench/` — knievel decision benchmarks

Phase 5.9. Two-tier suite covering the decision hot path:

| Tier | Tool | Output | Run from |
|---|---|---|---|
| Inner-loop micro | criterion | `bench/results/v<X>.json` | Cloud session via `xtask bench-all` |
| Inner-loop deterministic | iai-callgrind | same | Cloud session via `xtask bench-all` |
| Heap profile | dhat-rs | same | Cloud session via `xtask bench-all` |
| Macro HTTP load | vegeta + sampler | `$OUT/run.json` (see § Macro) | Operator on real infra |

The historical record lives in `bench/results/v<MAJ>.<MIN>.{md,json}`.
Schema is pinned in `bench/results/SCHEMA.md`. Regression policy
is documented in `TESTING.md` § 8.

## Quick start (cloud session)

```sh
sudo apt-get install -y valgrind
cargo install iai-callgrind-runner --version 0.16.1   # one-time
cargo xtask bench-all                       # captures v<workspace>.json
cargo xtask bench-all --against 0.1         # for v0.2+: regression diff
```

`iai-callgrind-runner` is a separate binary the iai-callgrind
crate shells out to under the hood — version must match the
crate dep in `Cargo.toml`. If the runner isn't on `$PATH`, the
iai benches fail with "Failed to run benchmarks: No such file
or directory"; fall back with `--skip-iai` until the runner is
installed.

`bench-all` reads the workspace version from `Cargo.toml`,
derives `bench/results/v<MAJ>.<MIN>.{md,json}`, runs the full
suite, and writes both files. Idempotent: re-running on the same
workspace version overwrites the in-progress entry. Pass
`--skip-iai` if valgrind isn't installable; `--skip-dhat` if
heap profile output isn't needed.

## What's measured

### `selection_pick` (criterion, inner loop)

`filter` → `priority` → `weighted_random` for one decision.
Sweeps flight count (10, 100, 1k, 10k, 100k) × selectivity
(1%, 10%, 50%, 100%). Real production snapshots see ~10%
selectivity post-targeting; the matrix exists so we can
attribute cost between filter and the priority + weighted-random
tail.

### `hmac_verify` (criterion, inner loop)

`/e/i/{signed}` and `/e/c/{signed}` per-request verifier cost.
Two cases: hot (current secret matches) and cold (falls through
to previous secret during 8-hour rotation overlap).

### `decision_handler` (criterion, end-to-end pure path)

`decide_pure` — the full request path from `DecisionsRequest` to
`DecisionsResponse`, in process, no Postgres, no HTTP. Sweeps:

- placement count (1, 4, 10) — REQUIREMENTS.md § 9.1 quotes p50
  @ 1 placement, p99 @ 4 placements
- post-filter selectivity (1%, 10%, 50%, 100%)
- snapshot size (100, 1k, 10k flights)

Side fixtures: `force_override` (force.adId substitution) and
`blocked` (heavy block-set, measures block-list cost).

### `iai_decision` + `iai_hmac` (iai-callgrind)

Deterministic CPU instruction / cache-miss counts. **Hardware-
independent** — identical source on identical rustc emits
identical instruction counts whether a 2-vCPU runner or a
workstation. That property makes `bench/results/v<X>.json`
deltas authoritative across releases regardless of which runner
ran them.

Smaller fixture matrix because callgrind is ~10–50× slower:

| Bench | Snapshot | Selectivity | Placements |
|---|---|---|---|
| `iai_decide_n100_p1_sel100` | 100 flights | 100% | 1 |
| `iai_decide_n1k_p1_sel10` | 1k flights | 10% | 1 |
| `iai_decide_n1k_p1_sel100` | 1k flights | 100% | 1 |
| `iai_decide_n1k_p4_sel100` | 1k flights | 100% | 4 |
| `iai_decide_force_override` | 1k flights | 100% | 1 (force.adId) |

### `dhat_decision` (dhat-rs)

Heap profile over 1k decisions on the n1000/p1/sel1.00 fixture.
Reports total bytes allocated, total alloc count, peak bytes,
peak alloc count, and per-decision averages.

Gated behind the `dhat-heap` cargo feature so production builds
don't swap the global allocator. The orchestrator passes
`--features dhat-heap` automatically; ad-hoc:

```sh
cargo bench --bench dhat_decision --features dhat-heap
```

## No-Postgres invariant

The in-process benches build `Snapshot` directly from the `pub`
types in `src/snapshot.rs`. They never call `snapshot::run_loader`
(which takes a `PgPool`), never construct `AppState`, never
spawn the events flusher. To prove this, run with no DB
configured:

```sh
unset DATABASE_URL
cargo xtask bench-all --skip-iai --skip-dhat   # criterion only
```

Should complete cleanly. Any Postgres connection on the bench
path is a regression in the `decide_pure` extraction — fix the
harness, not the test.

## Macro (operator-run)

`bench/macro/loadgen.sh` drives `POST /v1/projects/{p}/decisions`
against a real knievel + Postgres with `vegeta` and samples
`/proc/<pid>/...` over the run. Inputs (env vars):

```sh
KNIEVEL_HOST=https://knievel-bench.internal:8080
KNIEVEL_TOKEN=$(knievel-cli mint-token …)
KNIEVEL_PROJECT=pj_xxxxxxxxxxxx
KNIEVEL_PID=12345                    # for /proc/<pid> sampling
DURATION=60s                         # default
RATE=5000/s                          # default
OUT=./bench-out/$(date -u +%Y%m%dT%H%M%SZ)
bash bench/macro/loadgen.sh
```

Outputs in `$OUT/`:

- `target.txt`, `body.json` — vegeta inputs
- `raw.bin` — vegeta binary results
- `summary.txt`, `histogram.txt` — vegeta reports
- `proc_samples.csv` — 1 row/sec: ts, %cpu, RSS_kb, VmPeak_kb,
  threads, voluntary_ctxt_switches, nonvoluntary_ctxt_switches
- `env.json` — host fingerprint via `cargo xtask bench-env`
- `run.json` — unified summary; paste into the `macro` slot of
  `bench/results/v<X>.json`

`bench/macro/seed.sh` synthesizes a 100k-flight project for the
load test. Run it once against the target DB before invoking
`loadgen.sh`.

## Adding new benches

1. Add the file under `benches/<name>.rs`.
2. Use `mod common;` to pull in the shared fixtures.
3. Add a `[[bench]]` entry to `Cargo.toml` with `harness = false`.
4. If it captures a new signal, extend `bench/results/SCHEMA.md`
   and the orchestrator's `collect_*_stats` walker.
5. The next `cargo xtask bench-all` run will pick it up.

## Releasing

The release-tagging PR (`xtask release-tag`) carries the new
bench entry. Procedure:

1. On the release branch, run `cargo xtask bench-all` in a cloud
   session.
2. Run `cargo xtask bench-all --against v<prev>` and paste the
   regression diff into the PR description.
3. Operator runs the macro tier separately and pastes
   `run.json`'s summary into the `macro` slot of
   `bench/results/v<X>.json`.
4. Merge.
