#!/usr/bin/env bash
# Macro load benchmark — `POST /v1/projects/{p}/decisions` against a
# real knievel binary + Postgres + a synthetic 100k-flight project.
# Operator-run; not part of CI. Documented in `bench/README.md`
# and `TESTING.md` § 8.
#
# Phase 5.7 — vegeta wrapper and target template.
# Phase 5.9 — concurrent /proc/<pid>/... sampling so we capture
# CPU% and RSS over the run alongside vegeta's p50/p99/QPS, and
# unified `run.json` matching `bench/results/SCHEMA.md`'s
# `macro` slot.
#
# Inputs (all overridable via env):
#
#   KNIEVEL_HOST       — e.g. https://knievel-bench.internal:8080
#   KNIEVEL_TOKEN      — bearer minted via `knievel-cli mint-token`
#   KNIEVEL_PROJECT    — e.g. pj_xxxxxxxxxxxx
#   KNIEVEL_SITE_ID    — site fixture id in the synthetic project
#   KNIEVEL_AD_TYPE    — ad type fixture id
#   KNIEVEL_PID        — pid of the knievel process for /proc sampling.
#                        If unset, the proc-sampler is skipped and
#                        run.json's proc_samples_summary is left null.
#   DURATION           — vegeta -duration (default 60s)
#   RATE               — vegeta -rate (default 5000/s)
#   OUT                — directory for raw + summary outputs
#                        (default ./bench-out/<timestamp>)
#
# Output (in $OUT/):
#
#   target.txt, body.json         vegeta inputs
#   raw.bin                       vegeta binary results
#   summary.txt, histogram.txt    vegeta reports
#   proc_samples.csv              1 row/sec /proc/<pid>/... sample
#   env.json                      host fingerprint via xtask bench-env
#   run.json                      unified summary for the
#                                 `macro` slot in bench/results/v<X>.json
#
# Pre-req:
#
#   1. `cargo install vegeta` (or `apt install` / `brew install`).
#   2. `bash bench/macro/seed.sh` against the target DB to seed
#      the 100k-flight synthetic project.
#   3. Knievel running on $KNIEVEL_HOST (release build).

set -euo pipefail

: "${KNIEVEL_HOST:?required}"
: "${KNIEVEL_TOKEN:?required}"
: "${KNIEVEL_PROJECT:?required}"
KNIEVEL_SITE_ID="${KNIEVEL_SITE_ID:-1}"
KNIEVEL_AD_TYPE="${KNIEVEL_AD_TYPE:-1}"
KNIEVEL_PID="${KNIEVEL_PID:-}"
DURATION="${DURATION:-60s}"
RATE="${RATE:-5000/s}"
OUT="${OUT:-./bench-out/$(date -u +%Y%m%dT%H%M%SZ)}"

mkdir -p "$OUT"

# ----- env fingerprint -----
if command -v cargo >/dev/null 2>&1 && [ -f Cargo.toml ]; then
  cargo run -p xtask --quiet -- bench-env > "$OUT/env.json" || \
    echo '{"note":"xtask bench-env failed; skipping fingerprint"}' > "$OUT/env.json"
else
  echo '{"note":"cargo/xtask not on PATH; skipping fingerprint"}' > "$OUT/env.json"
fi

# ----- vegeta target -----
target_file="$OUT/target.txt"
body_file="$OUT/body.json"
cat > "$body_file" <<EOF
{
  "placements": [
    {"id": "header", "site_id": ${KNIEVEL_SITE_ID}, "ad_types": [${KNIEVEL_AD_TYPE}]}
  ]
}
EOF
cat > "$target_file" <<EOF
POST ${KNIEVEL_HOST}/v1/projects/${KNIEVEL_PROJECT}/decisions
Authorization: Bearer ${KNIEVEL_TOKEN}
Content-Type: application/json
@${body_file}
EOF

# ----- /proc sampler (optional, requires KNIEVEL_PID) -----
sampler_pid=""
proc_csv="$OUT/proc_samples.csv"
if [[ -n "$KNIEVEL_PID" ]] && [[ -d "/proc/$KNIEVEL_PID" ]]; then
  echo "ts_unix,cpu_pct,rss_kb,vmpeak_kb,threads,vol_ctxt,nonvol_ctxt" > "$proc_csv"
  (
    # cpu_pct uses delta of utime+stime over wall-clock seconds.
    prev_jiffies=$(awk '{print $14+$15}' "/proc/$KNIEVEL_PID/stat" 2>/dev/null || echo 0)
    prev_ts=$(date +%s)
    while [[ -d "/proc/$KNIEVEL_PID" ]]; do
      sleep 1
      now_ts=$(date +%s)
      now_jiffies=$(awk '{print $14+$15}' "/proc/$KNIEVEL_PID/stat" 2>/dev/null || echo 0)
      dt=$((now_ts - prev_ts))
      [[ "$dt" -lt 1 ]] && dt=1
      djiff=$((now_jiffies - prev_jiffies))
      hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
      cpu_pct=$(awk -v dj="$djiff" -v dt="$dt" -v hz="$hz" 'BEGIN { if (dt>0 && hz>0) printf "%.2f", (dj/hz)/dt*100; else print "0.00" }')
      rss_kb=$(awk '/^VmRSS:/{print $2}' "/proc/$KNIEVEL_PID/status" 2>/dev/null || echo 0)
      vmpeak_kb=$(awk '/^VmPeak:/{print $2}' "/proc/$KNIEVEL_PID/status" 2>/dev/null || echo 0)
      threads=$(awk '/^Threads:/{print $2}' "/proc/$KNIEVEL_PID/status" 2>/dev/null || echo 0)
      vol=$(awk '/^voluntary_ctxt_switches:/{print $2}' "/proc/$KNIEVEL_PID/status" 2>/dev/null || echo 0)
      nonvol=$(awk '/^nonvoluntary_ctxt_switches:/{print $2}' "/proc/$KNIEVEL_PID/status" 2>/dev/null || echo 0)
      echo "${now_ts},${cpu_pct},${rss_kb},${vmpeak_kb},${threads},${vol},${nonvol}" >> "$proc_csv"
      prev_jiffies=$now_jiffies
      prev_ts=$now_ts
    done
  ) &
  sampler_pid=$!
  trap '[[ -n "$sampler_pid" ]] && kill "$sampler_pid" 2>/dev/null || true' EXIT
else
  echo "WARN: KNIEVEL_PID unset or /proc/\$KNIEVEL_PID missing; skipping proc sampler" >&2
fi

# ----- vegeta -----
vegeta attack \
  -targets="$target_file" \
  -duration="$DURATION" \
  -rate="$RATE" \
  > "$OUT/raw.bin"

vegeta report -type=text < "$OUT/raw.bin" > "$OUT/summary.txt"
vegeta report -type='hist[0,2ms,5ms,10ms,25ms,50ms,100ms]' < "$OUT/raw.bin" \
  > "$OUT/histogram.txt"

# Stop the sampler now that vegeta is done.
if [[ -n "$sampler_pid" ]]; then
  kill "$sampler_pid" 2>/dev/null || true
  wait "$sampler_pid" 2>/dev/null || true
fi

# ----- summarize proc samples -----
proc_summary="null"
if [[ -f "$proc_csv" ]] && [[ "$(wc -l < "$proc_csv")" -gt 1 ]]; then
  proc_summary=$(awk -F, 'NR > 1 {
    cpu[NR] = $2
    rss[NR] = $3
    n++
    if ($3+0 > peak_rss) peak_rss = $3+0
    if ($6+0 > vmax) vmax = $6+0
    if (NR == 2) { vmin = $6+0; cmin = $7+0; }
    if ($7+0 > cmax) cmax = $7+0
  }
  END {
    if (n == 0) { print "null"; exit }
    asort(cpu)
    p50_idx = int(0.5 * n + 0.5); if (p50_idx < 1) p50_idx = 1
    p95_idx = int(0.95 * n + 0.5); if (p95_idx < 1) p95_idx = 1
    if (p50_idx > n) p50_idx = n
    if (p95_idx > n) p95_idx = n
    p50 = cpu[p50_idx]; p95 = cpu[p95_idx]
    ctxt_rate = (vmax - vmin + cmax - cmin) / n
    peak_rss_mb = peak_rss / 1024
    printf "{\"p50_cpu_pct\":%s,\"p95_cpu_pct\":%s,\"peak_rss_mb\":%.1f,\"ctxt_switch_rate_per_s\":%.1f}", p50, p95, peak_rss_mb, ctxt_rate
  }' "$proc_csv")
fi

# ----- vegeta numeric extracts -----
# vegeta's text summary has lines like:
#   Latencies     [min, mean, 50, 90, 95, 99, max]  ...
#   Success       [ratio]  100.00%
#   Throughput    [target, achieved]                ...
# Pull the headline metrics with awk.
p50_ms=$(awk '/^Latencies/ {gsub(/[a-z\[\],]/, "", $0); print $4}' "$OUT/summary.txt" | tr -d ' ')
p95_ms=$(awk '/^Latencies/ {gsub(/[a-z\[\],]/, "", $0); print $6}' "$OUT/summary.txt" | tr -d ' ')
p99_ms=$(awk '/^Latencies/ {gsub(/[a-z\[\],]/, "", $0); print $7}' "$OUT/summary.txt" | tr -d ' ')
max_ms=$(awk '/^Latencies/ {gsub(/[a-z\[\],]/, "", $0); print $8}' "$OUT/summary.txt" | tr -d ' ')
success_pct=$(awk '/^Success/ {print $NF}' "$OUT/summary.txt" | tr -d '%')
qps=$(awk '/^Throughput/ {print $NF}' "$OUT/summary.txt")

# Capture knievel SHA from /version (best-effort).
knievel_sha="$(curl -fsS "${KNIEVEL_HOST}/version" 2>/dev/null | jq -r .git_sha 2>/dev/null || echo unknown)"

cat > "$OUT/run.json" <<EOF
{
  "knievel_sha":     "${knievel_sha}",
  "host":            "${KNIEVEL_HOST}",
  "project_id":      "${KNIEVEL_PROJECT}",
  "duration":        "${DURATION}",
  "rate":            "${RATE}",
  "site_id":         ${KNIEVEL_SITE_ID},
  "ad_type":         ${KNIEVEL_AD_TYPE},
  "p50_ms":          ${p50_ms:-null},
  "p95_ms":          ${p95_ms:-null},
  "p99_ms":          ${p99_ms:-null},
  "max_ms":          ${max_ms:-null},
  "success_rate":    ${success_pct:-null},
  "throughput_qps":  ${qps:-null},
  "proc_samples_summary": ${proc_summary}
}
EOF

echo
echo "Run written to: ${OUT}"
echo
cat "$OUT/summary.txt"
echo
echo "Paste \`${OUT}/run.json\` into the \`macro\` slot of \`bench/results/v<X>.json\`."
