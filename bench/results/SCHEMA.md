# Bench results JSON schema

Phase 5.9. Pins the contract for `bench/results/v<MAJ>.<MIN>.json`
so historical entries diff cleanly across releases. Adding a key
is fine; renaming or removing a key is a breaking change to the
historical record and requires bumping `schema_version`.

The companion `.md` file is human-readable but the `.json` is
canonical — that's what `cargo xtask bench-all --against vX.Y`
reads.

## Top-level shape

```json,ignore
{
  "schema_version":  1,
  "knievel_version": "0.1.x",
  "captured_at_utc": "2026-05-08T12:34:56Z",
  "knievel_sha":     "<git short sha>",
  "env":             { … host fingerprint … },
  "micro_criterion": { … per-bench wall-clock … },
  "micro_iai":       { … per-bench instruction counts … },
  "heap_dhat":       { … heap totals … },
  "macro":           null
}
```

`null` is used for "not captured" — the macro slot stays null
until an operator runs `bench/macro/loadgen.sh`. The orchestrator
treats null as "skip in regression diff" rather than "regressed
to zero."

## `env` — host fingerprint

Flat object; one entry per signal so diffs are line-by-line.

| key | meaning |
|---|---|
| `captured_at_utc` | ISO-8601 UTC timestamp |
| `kernel` | `uname -r` |
| `rustc` | `rustc -V` |
| `os_release` | `/etc/os-release` PRETTY_NAME |
| `cpu_model` | first `model name` from `/proc/cpuinfo` |
| `cpu_cores_logical` | count of `processor` lines |
| `cpu_mhz_max` | max `cpu MHz` value reported |
| `cpu_governor` | `/sys/.../cpu0/cpufreq/scaling_governor` |
| `mem_total_kb` | `MemTotal` from `/proc/meminfo` |
| `hostname` | `hostname(1)` |
| `container.cgroup_v2_cpu_max` | `cgroup v2` cpu.max |
| `container.cgroup_v2_memory_max` | `cgroup v2` memory.max |

A run captured under a CPU governor other than `performance`
should be flagged in the `.md` companion's prose; the JSON
records the value but the regression policy doesn't gate on it.

## `micro_criterion` — wall-clock

Keyed on `<bench_id>` where `<bench_id>` is the criterion-side
identifier (e.g. `selection_pick/n1000/sel0.10` or
`decide_pure/n1000/p1/sel1.00`).

```json,ignore
{
  "selection_pick/n1000/sel0.10": {
    "mean_ns":    14000.0,
    "median_ns":  13900.0,
    "std_dev_ns": 250.0
  },
  …
}
```

Source: `target/criterion/<group>/<bench>/new/estimates.json`.
The orchestrator walks the directory tree and pulls the headline
estimates per bench.

## `micro_iai` — deterministic instruction counts

Keyed on the `#[library_benchmark]` function name. The events
block carries the raw callgrind counters — at minimum
`Instructions` (Ir), `Dr`, `Dw`, `I1mr`, `D1mr`, `D1mw`,
`ILmr`, `DLmr`, `DLmw`, `Bc`, `Bcm`, `Bi`, `Bim`. Layout depends
on the iai-callgrind version; the orchestrator copies the events
sub-object verbatim so future versions can extend without a
schema break.

```json,ignore
{
  "iai_decide_n1k_p1_sel100": {
    "events": {
      "Ir":   180000,
      "Dr":   60000,
      …
    }
  }
}
```

The regression diff defaults to comparing `events.Ir`
(instructions retired). > 5% delta opens an issue.

## `heap_dhat` — heap totals

Single-bench (the dhat bench runs one fixture: 1k decisions over
n1000/p1/sel1.00). Schema:

```json,ignore
{
  "bench":               "dhat_decision",
  "iterations":          1000,
  "fixture":             "n1000/p1/sel1.00",
  "total_blocks":        12345,
  "total_bytes":         9876543,
  "peak_blocks":         234,
  "peak_bytes":          12345,
  "blocks_per_decision": 12.345,
  "bytes_per_decision":  9876.5
}
```

When `dhat_decision` runs without the `dhat-heap` cargo feature
on, the values are zero-filled and a `note` field explains why.
The orchestrator preserves zeroed placeholders so absence
doesn't break the diff.

## `macro` — operator-run vegeta load

Filled by `bench/macro/loadgen.sh`'s `run.json`, then merged here
by the operator (or by `xtask bench-all --merge-macro <path>` —
not implemented in v0.1; left as a Phase 6 polish if needed).

Expected shape when present:

```json,ignore
{
  "rate":               "5000/s",
  "duration":           "60s",
  "p50_ms":             0.8,
  "p95_ms":             4.0,
  "p99_ms":             9.5,
  "max_ms":             23.0,
  "success_rate":       1.0,
  "throughput_qps":     4998,
  "proc_samples_summary": {
    "p50_cpu_pct":          85.0,
    "p95_cpu_pct":          92.0,
    "peak_rss_mb":          412,
    "ctxt_switch_rate_per_s": 1500
  }
}
```

`peak_rss_mb` is read from `/proc/<pid>/status:VmHWM` (same field
container memory accounting reads). `ctxt_switch_rate_per_s` is
derived from `voluntary_ctxt_switches` deltas across the
sampling interval.

## Regression diff rules (orchestrator-applied)

`cargo xtask bench-all --against v<prev>` walks both files and
prints a markdown table. Default thresholds:

| signal | threshold | rationale |
|---|---|---|
| iai `events.Ir` | > 5% | deterministic; any drift is real |
| criterion `mean_ns` | > 20% | matches release-tag policy |
| dhat `total_bytes` | > 30% | high-noise; large changes still call out |

Macro deltas are flagged manually in the release-tagging PR
comment per `TESTING.md` § 8 (> 20% on p50/p99/QPS blocks the tag
without an explicit waiver).

`(new)` markers identify benches present in the current run but
absent from `--against`. Adding a bench is normal; removing one
is a deliberate change worth calling out in the commit message.
