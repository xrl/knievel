//! `xtask bench-env` — emit a host fingerprint as JSON.
//!
//! Phase 5.9. The fingerprint pins down the hardware + toolchain
//! a benchmark run was captured on so future readers can decide
//! whether a delta is real (same hardware) or noise (different
//! runner). Captured into `bench/results/v<X>.json` under the
//! `env` key by `xtask bench-all`; can also be invoked
//! standalone for debugging.
//!
//! Output is intentionally a flat object — diffing two fingerprints
//! between runs is trivially `jq` -able. Schema is documented
//! in `bench/results/SCHEMA.md`.

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use anyhow::Result;
use serde_json::{json, Value};

pub fn run() -> Result<()> {
    let v = collect();
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// Public accessor used by `bench_all::run` so the orchestrator
/// can splice the fingerprint into the unified results JSON
/// without shelling out to itself.
pub fn collect() -> Value {
    let mut env = BTreeMap::<&'static str, Value>::new();
    env.insert("captured_at_utc", json!(now_utc()));
    env.insert("kernel", json!(uname_r()));
    env.insert("rustc", json!(rustc_version()));
    env.insert("os_release", json!(os_release()));
    env.insert("cpu_model", json!(cpu_model()));
    env.insert("cpu_cores_logical", json!(cpu_cores_logical()));
    env.insert("cpu_mhz_max", json!(cpu_mhz_max()));
    env.insert("cpu_governor", json!(cpu_governor()));
    env.insert("mem_total_kb", json!(mem_total_kb()));
    env.insert("hostname", json!(hostname()));
    env.insert(
        "container",
        json!({
            "cgroup_v2_cpu_max":     read_trim("/sys/fs/cgroup/cpu.max"),
            "cgroup_v2_memory_max":  read_trim("/sys/fs/cgroup/memory.max"),
        }),
    );
    json!(env)
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

fn uname_r() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn hostname() -> String {
    Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn os_release() -> String {
    let s = read_trim("/etc/os-release");
    // Pull `PRETTY_NAME=...` for a one-liner; fall back to raw
    // contents if the file shape is unexpected.
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return rest.trim_matches('"').to_string();
        }
    }
    s
}

fn cpu_model() -> String {
    let s = read_trim("/proc/cpuinfo");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("model name") {
            return rest.trim_start_matches([':', ' ', '\t']).to_string();
        }
    }
    String::new()
}

fn cpu_cores_logical() -> u64 {
    let s = read_trim("/proc/cpuinfo");
    s.lines().filter(|l| l.starts_with("processor")).count() as u64
}

fn cpu_mhz_max() -> f64 {
    // Best-effort. /proc/cpuinfo has `cpu MHz` per logical core
    // (current freq, may be governor-throttled). Take the max
    // across reported values; the bench's CPU governor section
    // discloses whether throttling was on.
    let s = read_trim("/proc/cpuinfo");
    s.lines()
        .filter_map(|l| l.strip_prefix("cpu MHz"))
        .filter_map(|rest| {
            rest.trim_start_matches([':', ' ', '\t'])
                .parse::<f64>()
                .ok()
        })
        .fold(0.0_f64, |acc, v| acc.max(v))
}

fn cpu_governor() -> String {
    read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
}

fn mem_total_kb() -> u64 {
    let s = read_trim("/proc/meminfo");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            // Format: `MemTotal:       16384000 kB`.
            return rest
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0);
        }
    }
    0
}

fn read_trim(path: &str) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
