//! `xtask bench-all` — comprehensive decision-bench orchestrator
//! (Phase 5.9). Runs the criterion + iai-callgrind + dhat suite,
//! captures host fingerprint via `xtask bench-env`, assembles
//! `bench/results/v<MAJ>.<MIN>.json` matching
//! `bench/results/SCHEMA.md`, and refreshes the corresponding
//! `.md` entry from a template.
//!
//! Designed for **Claude Code cloud sessions** — there is no CI
//! integration. Operator/agent runs `cargo xtask bench-all` once
//! per release branch; the captured JSON is the canonical
//! historical record. Re-running on the same workspace version
//! overwrites the in-progress entry idempotently.
//!
//! Optional `--against <prev>` reads
//! `bench/results/v<prev>.json` and prints a regression diff
//! table (instructions, wall-clock, allocs). Output is for
//! pasting into the commit message; not a hard fail.
//!
//! The bench section's contract is documented in `TESTING.md`
//! § 8 and `bench/results/SCHEMA.md`. Don't add or rename
//! signal keys here without updating both — the JSON shape
//! is what makes v<X+1> diff cleanly against v<X>.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::bench_env;

#[derive(Debug)]
pub struct Args {
    pub against: Option<String>,
    pub skip_iai: bool,
    pub skip_dhat: bool,
}

pub fn run(args: Args) -> Result<()> {
    let version = read_workspace_minor()?;
    println!("xtask bench-all: capturing baseline for v{}", version);

    let env = bench_env::collect();

    let mut criterion: BTreeMap<String, Value> = BTreeMap::new();
    for bench in ["selection_pick", "hmac_verify", "decision_handler"] {
        println!("\n  -> cargo bench --bench {bench}");
        run_criterion(bench)?;
        let stats = collect_criterion_stats(bench)?;
        criterion.insert(bench.to_string(), stats);
    }

    let mut iai_stats: Value = json!({});
    if !args.skip_iai {
        for bench in ["iai_decision", "iai_hmac"] {
            println!("\n  -> cargo bench --bench {bench}");
            match run_iai(bench) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!(
                        "WARN: iai bench {bench} failed: {e}\n      \
                         (valgrind installed? `sudo apt-get install -y valgrind`)\n      \
                         capturing partial results."
                    );
                }
            }
        }
        iai_stats = collect_iai_stats()?;
    } else {
        println!("\n  -> skipping iai-callgrind benches (--skip-iai)");
    }

    let dhat_stats = if !args.skip_dhat {
        println!("\n  -> cargo bench --bench dhat_decision --features dhat-heap");
        run_dhat()?;
        collect_dhat_stats()?
    } else {
        println!("\n  -> skipping dhat heap-profile bench (--skip-dhat)");
        json!(null)
    };

    let captured = json!({
        "schema_version":  1,
        "knievel_version": format!("{}.x", version),
        "captured_at_utc": now_utc(),
        "knievel_sha":     git_short_sha(),
        "env":             env,
        "micro_criterion": criterion,
        "micro_iai":       iai_stats,
        "heap_dhat":       dhat_stats,
        "macro":           json!(null),
    });

    let json_path = json_results_path(&version);
    let md_path = md_results_path(&version);
    let pretty = serde_json::to_string_pretty(&captured)?;
    fs::write(&json_path, format!("{pretty}\n"))
        .with_context(|| format!("write {}", json_path.display()))?;
    println!("\nwrote: {}", json_path.display());

    refresh_md_entry(&md_path, &captured)?;
    println!("wrote: {}", md_path.display());

    if let Some(against) = args.against {
        match load_results(&against) {
            Ok(prev) => {
                println!("\n## Regression diff vs v{against}\n");
                print_regression_diff(&prev, &captured);
            }
            Err(e) => {
                eprintln!("WARN: --against v{against}: {e}");
            }
        }
    }

    println!(
        "\nxtask bench-all: done. Review the .md/.json entries before committing.\n\
         If macro numbers are needed for this release, run \
         `bash bench/macro/loadgen.sh` and paste the resulting \
         `run.json` into the `macro` slot."
    );

    Ok(())
}

// ---------- workspace version ----------

/// Reads `version` from `[workspace.package]` and returns the
/// `<major>.<minor>` slice. Bench results are tracked at minor
/// granularity (`v0.1`, `v0.2`, ...); patch versions roll up
/// into the same file.
fn read_workspace_minor() -> Result<String> {
    let raw = fs::read_to_string("Cargo.toml").context("read Cargo.toml")?;
    let parsed: toml::Value = toml::from_str(&raw).context("parse Cargo.toml")?;
    let v = parsed
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Cargo.toml: workspace.package.version not found"))?;
    let mut parts = v.splitn(3, '.');
    let major = parts.next().unwrap_or("0");
    let minor = parts.next().unwrap_or("0");
    Ok(format!("{major}.{minor}"))
}

fn json_results_path(minor: &str) -> PathBuf {
    PathBuf::from(format!("bench/results/v{minor}.json"))
}

fn md_results_path(minor: &str) -> PathBuf {
    PathBuf::from(format!("bench/results/v{minor}.md"))
}

// ---------- criterion ----------

fn run_criterion(bench: &str) -> Result<()> {
    // `--noplot` skips HTML/PDF generation, which `--quick`'s
    // single-sample runs trip on (criterion's plotters_backend
    // panics when fed a 1-element slice). The estimates.json
    // file we read for stats is independent of plot output.
    let status = Command::new("cargo")
        .args(["bench", "--bench", bench, "--", "--quick", "--noplot"])
        .status()
        .with_context(|| format!("spawn cargo bench {bench}"))?;
    if !status.success() {
        bail!("cargo bench --bench {bench} exited non-zero");
    }
    Ok(())
}

/// Walk `target/criterion/<group>/<bench>/new/estimates.json`
/// for every bench under a top-level group. Falls back to
/// looking for the bench by name at the top level if no group
/// directory exists.
fn collect_criterion_stats(bench: &str) -> Result<Value> {
    let root = PathBuf::from("target/criterion");
    let mut out = BTreeMap::<String, Value>::new();
    if !root.exists() {
        return Ok(json!({}));
    }
    walk_criterion_dir(&root, "", bench, &mut out)?;
    Ok(json!(out))
}

fn walk_criterion_dir(
    dir: &std::path::Path,
    prefix: &str,
    _bench: &str,
    out: &mut BTreeMap<String, Value>,
) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
    for e in entries {
        let e = e?;
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "report" {
            continue;
        }
        let est = path.join("new/estimates.json");
        if est.exists() {
            let raw =
                fs::read_to_string(&est).with_context(|| format!("read {}", est.display()))?;
            let parsed: Value =
                serde_json::from_str(&raw).with_context(|| format!("parse {}", est.display()))?;
            let mean_ns = parsed
                .pointer("/mean/point_estimate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let median_ns = parsed
                .pointer("/median/point_estimate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let std_dev_ns = parsed
                .pointer("/std_dev/point_estimate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let key = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            out.insert(
                key,
                json!({
                    "mean_ns":     mean_ns,
                    "median_ns":   median_ns,
                    "std_dev_ns":  std_dev_ns,
                }),
            );
        } else {
            // Recurse one level: criterion's group dirs hold per-bench dirs.
            let next_prefix = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            walk_criterion_dir(&path, &next_prefix, _bench, out)?;
        }
    }
    Ok(())
}

// ---------- iai-callgrind ----------

fn run_iai(bench: &str) -> Result<()> {
    // `--save-summary` writes per-bench summary.json files under
    // target/iai/.../<name>/summary.json. Without it, only raw
    // callgrind.out + a text log are produced and the orchestrator
    // can't extract the headline counters cleanly.
    let status = Command::new("cargo")
        .args(["bench", "--bench", bench, "--", "--save-summary"])
        .status()
        .with_context(|| format!("spawn cargo bench {bench}"))?;
    if !status.success() {
        bail!("cargo bench --bench {bench} exited non-zero");
    }
    Ok(())
}

/// iai-callgrind writes `target/iai/...callgrind.out` files plus
/// per-bench summary files. The summary lives at
/// `target/iai/<crate>/<bench>/<group>/<name>/<id>/summary.json`
/// for v0.16.x. Walk the tree and pull the headline counters.
fn collect_iai_stats() -> Result<Value> {
    let root = PathBuf::from("target/iai");
    let mut out = BTreeMap::<String, Value>::new();
    if !root.exists() {
        return Ok(json!({}));
    }
    walk_iai_dir(&root, &mut out)?;
    Ok(json!(out))
}

fn walk_iai_dir(dir: &std::path::Path, out: &mut BTreeMap<String, Value>) -> Result<()> {
    for e in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let e = e?;
        let path = e.path();
        if path.is_dir() {
            walk_iai_dir(&path, out)?;
        } else if path.file_name().and_then(|n| n.to_str()) == Some("summary.json") {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            let parsed: Value =
                serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
            // Re-key by the summary's `function_name` if present,
            // fall back to the parent dir name.
            let key = parsed
                .pointer("/function_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                });
            // iai-callgrind 0.16 nests headline counters at
            // `profiles[0].summaries.parts[0].metrics_summary.Callgrind.<event>.metrics.Both[0].Int`.
            // Pull the canonical event set (instructions, data ref
            // counts, cache misses, cycles). Future iai-callgrind
            // versions may flatten this — when that happens, update
            // SCHEMA.md and this extractor in lockstep.
            let cg = parsed.pointer("/profiles/0/summaries/parts/0/metrics_summary/Callgrind");
            let mut events = serde_json::Map::new();
            if let Some(cg) = cg {
                for ev in [
                    "Ir",
                    "Dr",
                    "Dw",
                    "I1mr",
                    "D1mr",
                    "D1mw",
                    "ILmr",
                    "DLmr",
                    "DLmw",
                    "EstimatedCycles",
                    "L1hits",
                    "LLhits",
                    "RamHits",
                    "TotalRW",
                ] {
                    // Two shapes — fresh run (no baseline) reports
                    // `metrics.Left.Int`; rerun against a baseline
                    // reports `metrics.Both[0].Int`. Try both.
                    let left = cg
                        .pointer(&format!("/{ev}/metrics/Left/Int"))
                        .and_then(|v| v.as_i64());
                    let both = cg
                        .pointer(&format!("/{ev}/metrics/Both/0/Int"))
                        .and_then(|v| v.as_i64());
                    if let Some(n) = left.or(both) {
                        events.insert(ev.to_string(), json!(n));
                    }
                }
            }
            out.insert(key, json!({ "events": events }));
        }
    }
    Ok(())
}

// ---------- dhat ----------

fn run_dhat() -> Result<Value> {
    let output = Command::new("cargo")
        .args([
            "bench",
            "--bench",
            "dhat_decision",
            "--features",
            "dhat-heap",
        ])
        .output()
        .context("spawn cargo bench dhat_decision")?;
    if !output.status.success() {
        bail!(
            "dhat bench failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = extract_dhat_json(&stdout)?;
    let json_path = PathBuf::from("target/dhat_decision.json");
    fs::write(&json_path, &json).context("write dhat_decision.json")?;
    Ok(serde_json::from_str(&json)?)
}

fn extract_dhat_json(out: &str) -> Result<String> {
    let begin = out
        .find("DHAT_JSON_BEGIN")
        .ok_or_else(|| anyhow!("dhat output missing DHAT_JSON_BEGIN sentinel"))?;
    let end = out
        .find("DHAT_JSON_END")
        .ok_or_else(|| anyhow!("dhat output missing DHAT_JSON_END sentinel"))?;
    let body = &out[begin + "DHAT_JSON_BEGIN".len()..end];
    Ok(body.trim().to_string())
}

fn collect_dhat_stats() -> Result<Value> {
    let p = PathBuf::from("target/dhat_decision.json");
    let raw = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

// ---------- regression diff ----------

fn load_results(minor: &str) -> Result<Value> {
    let p = json_results_path(minor);
    let raw = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

fn print_regression_diff(prev: &Value, cur: &Value) {
    println!("| signal | prev | current | delta |");
    println!("|---|---|---|---|");
    diff_section(
        "criterion mean_ns",
        prev.pointer("/micro_criterion").unwrap_or(&Value::Null),
        cur.pointer("/micro_criterion").unwrap_or(&Value::Null),
        |v| v.pointer("/mean_ns").and_then(|n| n.as_f64()),
        20.0,
    );
    diff_section(
        "iai instructions",
        prev.pointer("/micro_iai").unwrap_or(&Value::Null),
        cur.pointer("/micro_iai").unwrap_or(&Value::Null),
        |v| {
            v.pointer("/events/Ir")
                .and_then(|n| n.as_f64())
                .or_else(|| v.pointer("/events/Instructions").and_then(|n| n.as_f64()))
        },
        5.0,
    );
    diff_section(
        "dhat total_bytes",
        prev.pointer("/heap_dhat").unwrap_or(&Value::Null),
        cur.pointer("/heap_dhat").unwrap_or(&Value::Null),
        |v| v.pointer("/total_bytes").and_then(|n| n.as_f64()),
        30.0,
    );
}

fn diff_section(
    label: &str,
    prev: &Value,
    cur: &Value,
    extract: impl Fn(&Value) -> Option<f64>,
    threshold_pct: f64,
) {
    let walk = |v: &Value, into: &mut BTreeMap<String, f64>, prefix: &str| {
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}/{k}")
                };
                if let Some(n) = extract(val) {
                    into.insert(path.clone(), n);
                }
                if val.is_object() {
                    let inner_prefix = path.clone();
                    if let Some(inner) = val.as_object() {
                        for (ik, iv) in inner {
                            if let Some(n) = extract(iv) {
                                into.insert(format!("{inner_prefix}/{ik}"), n);
                            }
                        }
                    }
                }
            }
        }
    };
    let mut p = BTreeMap::<String, f64>::new();
    let mut c = BTreeMap::<String, f64>::new();
    walk(prev, &mut p, "");
    walk(cur, &mut c, "");
    let mut keys: Vec<&String> = c.keys().collect();
    keys.sort();
    for k in keys {
        let cur_v = c[k];
        let prev_v = match p.get(k) {
            Some(v) => *v,
            None => {
                println!("| {label} | {k} | (new) | {cur_v:.0} |");
                continue;
            }
        };
        if prev_v == 0.0 {
            continue;
        }
        let delta_pct = ((cur_v - prev_v) / prev_v) * 100.0;
        let flag = if delta_pct.abs() > threshold_pct {
            " ⚠"
        } else {
            ""
        };
        println!("| {label} | {k} | {prev_v:.2} | {cur_v:.2} | {delta_pct:+.1}%{flag} |");
    }
}

// ---------- markdown entry ----------

fn refresh_md_entry(path: &PathBuf, captured: &Value) -> Result<()> {
    // We don't overwrite the existing v<X>.md verbatim; v0.1.md
    // was hand-written and carries the methodology section.
    // Instead, we replace the §§ 2.3 (iai), 2.4 (dhat) sections
    // with auto-generated tables and append the schema/JSON
    // pointer if missing. If the file doesn't exist (future
    // versions), we synthesize one from the template below.
    let template = format!(
        "# Bench results — knievel v{}\n\n\
         Auto-generated by `cargo xtask bench-all` (Phase 5.9).\n\
         Methodology, fixture matrix, and regression policy live in\n\
         `bench/results/SCHEMA.md` and `TESTING.md` § 8. The JSON\n\
         companion file is the source of truth for diffing.\n\n\
         **JSON companion:** see `{}.json`\n\n\
         **Knievel SHA:** `{}`\n\n\
         **Captured (UTC):** {}\n\n\
         ## Hardware fingerprint\n\n\
         See `env` block in the JSON companion. Summary:\n\n\
         - CPU: {}\n\
         - Cores (logical): {}\n\
         - Memory: {} kB\n\
         - Kernel: {}\n\
         - Rustc: {}\n\
         - CPU governor: {}\n\n\
         ## Microbenchmark results\n\n\
         Numbers are summary headlines; full per-bench detail in JSON.\n\n\
         See JSON companion for the matrix.\n\n\
         ## Regression policy\n\n\
         Per `TESTING.md` § 8:\n\n\
         - **iai-callgrind instructions:** > 5% regression opens an issue\n  \
           (deterministic counters; any drift is real).\n\
         - **criterion wall-clock:** > 30% regression on micro;\n  \
           > 20% on macro p50/p99/QPS blocks the release tag.\n\
         - **dhat allocations:** > 30% regression opens an issue.\n\n\
         Diff against the previous release with\n\
         `cargo xtask bench-all --against vX.Y`.\n",
        captured
            .pointer("/knievel_version")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        path.with_extension("").display(),
        captured
            .pointer("/knievel_sha")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/captured_at_utc")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/cpu_model")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/cpu_cores_logical")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        captured
            .pointer("/env/mem_total_kb")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        captured
            .pointer("/env/kernel")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/rustc")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/cpu_governor")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );

    if !path.exists() {
        fs::write(path, template).with_context(|| format!("write {}", path.display()))?;
        return Ok(());
    }

    // For an existing v0.1.md (hand-authored), append a sentinel-
    // delimited "auto-generated tail" if not already present.
    // The hand-written sections stay untouched; the auto block
    // is overwritten on each run. This keeps v0.1.md's prose
    // intact while letting the JSON companion stay canonical.
    let body = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let begin = "<!-- BENCH_AUTO_BEGIN -->";
    let end = "<!-- BENCH_AUTO_END -->";
    let auto_block = format!(
        "\n\n{begin}\n\n## Auto-captured signals\n\n\
         JSON companion: `{}.json` (canonical).\n\n\
         **Captured (UTC):** {} | **SHA:** `{}`\n\n\
         - CPU: {} ({} logical cores)\n\
         - Mem: {} kB · Governor: {} · Kernel: {}\n\
         - Rustc: {}\n\n\
         Headline iai instruction counts and dhat heap totals\n\
         are in the JSON companion under `micro_iai` and\n\
         `heap_dhat`. Diff with `cargo xtask bench-all --against vX.Y`.\n\n\
         {end}\n",
        path.with_extension("").display(),
        captured
            .pointer("/captured_at_utc")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/knievel_sha")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/cpu_model")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/cpu_cores_logical")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        captured
            .pointer("/env/mem_total_kb")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        captured
            .pointer("/env/cpu_governor")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/kernel")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        captured
            .pointer("/env/rustc")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    );

    let new_body = if let (Some(b), Some(e)) = (body.find(begin), body.find(end)) {
        // Replace the existing auto block in place.
        let mut s = body[..b].to_string();
        s.push_str(auto_block.trim_start_matches('\n'));
        s.push_str(&body[e + end.len()..]);
        s
    } else {
        format!("{}{}", body.trim_end(), auto_block)
    };
    fs::write(path, new_body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn now_utc() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn git_short_sha() -> String {
    Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
