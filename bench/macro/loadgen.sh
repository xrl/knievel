#!/usr/bin/env bash
# Macro load benchmark — `POST /v1/projects/{p}/decisions` against a
# real knievel binary + Postgres + a synthetic 100k-flight project.
# Operator-run; not part of CI. Documented in `TESTING.md` § 8.
#
# Inputs (all overridable via env):
#
#   KNIEVEL_HOST       — e.g. https://knievel-bench.internal:8080
#   KNIEVEL_TOKEN      — bearer minted via `knievel-cli mint-token`
#   KNIEVEL_PROJECT    — e.g. pj_xxxxxxxxxxxx
#   KNIEVEL_SITE_ID    — site fixture id in the synthetic project
#   KNIEVEL_AD_TYPE    — ad type fixture id
#   DURATION           — vegeta -duration (default 60s)
#   RATE               — vegeta -rate (default 5000/s)
#   OUT                — directory for raw + summary outputs
#                        (default ./bench-out/<timestamp>)
#
# Output:
#
#   $OUT/raw.bin               vegeta binary results
#   $OUT/summary.txt           vegeta report --type=text
#   $OUT/histogram.txt         vegeta report --type=hist[…]
#   $OUT/run.json              metadata: knievel SHA, db tier,
#                              env vars, achieved QPS / p50/p99
#
# Pre-req:
#
#   1. `cargo install vegeta` (or `apt install` / `brew install`).
#   2. Knievel running on $KNIEVEL_HOST (release build).
#   3. Synthetic project loaded — see `bench/macro/seed.sql` (TBD).
#      For v0.1, the project is seeded by hand-running
#      `bench/macro/seed.sh` against your DB; that script lands
#      with the first real bench-results entry.

set -euo pipefail

: "${KNIEVEL_HOST:?required}"
: "${KNIEVEL_TOKEN:?required}"
: "${KNIEVEL_PROJECT:?required}"
KNIEVEL_SITE_ID="${KNIEVEL_SITE_ID:-1}"
KNIEVEL_AD_TYPE="${KNIEVEL_AD_TYPE:-1}"
DURATION="${DURATION:-60s}"
RATE="${RATE:-5000/s}"
OUT="${OUT:-./bench-out/$(date -u +%Y%m%dT%H%M%SZ)}"

mkdir -p "$OUT"

# Vegeta target: one POST /v1/projects/{p}/decisions per request,
# fixed body. Real workloads vary placement.id and zone_ids;
# extend the targets file when that variance matters.
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

vegeta attack \
  -targets="$target_file" \
  -duration="$DURATION" \
  -rate="$RATE" \
  > "$OUT/raw.bin"

vegeta report -type=text < "$OUT/raw.bin" > "$OUT/summary.txt"
vegeta report -type='hist[0,2ms,5ms,10ms,25ms,50ms,100ms]' < "$OUT/raw.bin" \
  > "$OUT/histogram.txt"

# Capture run metadata so the entry in bench/results/<v>.md is
# self-describing.
knievel_sha="$(curl -fsS "${KNIEVEL_HOST}/version" | jq -r .git_sha 2>/dev/null || echo unknown)"
cat > "$OUT/run.json" <<EOF
{
  "knievel_sha": "${knievel_sha}",
  "host":         "${KNIEVEL_HOST}",
  "project_id":   "${KNIEVEL_PROJECT}",
  "duration":     "${DURATION}",
  "rate":         "${RATE}",
  "site_id":      ${KNIEVEL_SITE_ID},
  "ad_type":      ${KNIEVEL_AD_TYPE},
  "summary":      "$(sed 's/"/\\"/g' "$OUT/summary.txt" | tr '\n' ' ')"
}
EOF

echo
echo "Run written to: ${OUT}"
echo
cat "$OUT/summary.txt"
