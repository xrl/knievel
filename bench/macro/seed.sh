#!/usr/bin/env bash
# Seed a synthetic 100k-flight project for the macro bench
# (`bench/macro/loadgen.sh`). Operator-run; not idempotent — drops
# and recreates the synthetic project on every invocation.
#
# Mirrors the in-process bench fixtures from
# `benches/common/mod.rs` so cross-tier comparisons are
# meaningful: same flight count, same selectivity model, same
# priority distribution.
#
# Inputs:
#
#   DATABASE_URL   — postgres://… (admin or sufficiently-privileged)
#   ORG_ID         — default: org_bench
#   PROJECT_ID     — default: pj_bench0000001 (matches benches/common)
#   N_FLIGHTS      — default: 100000
#   SELECTIVITY    — default: 1.0  (fraction of flights matching site=1)
#
# Output: writes through to the configured DB. Prints
# project_id and a single-row summary on success.

set -euo pipefail

: "${DATABASE_URL:?required}"
ORG_ID="${ORG_ID:-org_bench}"
PROJECT_ID="${PROJECT_ID:-pj_bench0000001}"
N_FLIGHTS="${N_FLIGHTS:-100000}"
SELECTIVITY="${SELECTIVITY:-1.0}"

n_match=$(awk -v n="$N_FLIGHTS" -v s="$SELECTIVITY" 'BEGIN { printf "%d", n*s + 0.5 }')

# All SQL runs in one transaction. The synthetic project sets up
# org → project → advertiser → campaigns → flights → ads → site
# → ad_type. Ad-type ID = 1 matches `benches/common::AD_TYPE`.
psql "$DATABASE_URL" <<SQL
SET search_path TO knievel, public;

BEGIN;

-- Org + project. The org must exist in the tenants table; create
-- it if not. Project carries the bench-tier defaults.
INSERT INTO orgs (id, name, slug)
  VALUES ('${ORG_ID}', 'Bench Org', 'bench')
  ON CONFLICT (id) DO NOTHING;

DELETE FROM projects WHERE id = '${PROJECT_ID}';

INSERT INTO projects (id, org_id, name, allow_force_decision)
  VALUES ('${PROJECT_ID}', '${ORG_ID}', 'Bench Project', true);

PERFORM set_config('knievel.org_id', '${ORG_ID}', true);
PERFORM set_config('knievel.project_id', '${PROJECT_ID}', true);

-- Site (id=1) — placements target this id.
INSERT INTO sites (id, project_id, url, external_id)
  VALUES (1, '${PROJECT_ID}', 'bench.example', 'site-bench')
  ON CONFLICT (id) DO NOTHING;

-- Advertiser, campaign, ad-type fixtures.
INSERT INTO advertisers (id, project_id, name, external_id)
  VALUES (1, '${PROJECT_ID}', 'Bench Advertiser', 'adv-bench')
  ON CONFLICT (id) DO NOTHING;
INSERT INTO campaigns (id, project_id, advertiser_id, name, external_id)
  VALUES (1, '${PROJECT_ID}', 1, 'Bench Campaign', 'camp-bench')
  ON CONFLICT (id) DO NOTHING;

-- Flights: insert ${N_FLIGHTS} rows with the same selectivity
-- model as benches/common::synthesize_snapshot. First n_match
-- target site=1 (eligible); the rest target site=1000 (filtered
-- out at decision time).
INSERT INTO flights (id, project_id, campaign_id, advertiser_id,
                     priority_tier, is_active, name, external_id,
                     site_ids, ad_types)
SELECT
  i,
  '${PROJECT_ID}',
  1,
  1,
  ((i - 1) % 5 + 1),
  true,
  'Bench Flight ' || i,
  'bench-fl-' || i,
  CASE WHEN i <= ${n_match} THEN ARRAY[1] ELSE ARRAY[1000] END,
  ARRAY[1]
FROM generate_series(1, ${N_FLIGHTS}) AS gs(i);

-- Ads: one ad per flight, mixed weights matching benches/common.
INSERT INTO ads (id, project_id, flight_id, weight, is_active,
                 name, external_id, kind)
SELECT
  i * 100,
  '${PROJECT_ID}',
  i,
  ((i - 1) % 50 + 1),
  true,
  'Bench Ad ' || i,
  'bench-ad-' || i,
  'inline'
FROM generate_series(1, ${N_FLIGHTS}) AS gs(i);

COMMIT;

-- Summary.
SELECT
  '${PROJECT_ID}' AS project_id,
  count(*)         AS flights_total,
  sum(CASE WHEN site_ids @> ARRAY[1] THEN 1 ELSE 0 END) AS flights_eligible,
  ${SELECTIVITY}::float AS selectivity_target
FROM flights
WHERE project_id = '${PROJECT_ID}';
SQL

echo
echo "Seeded ${N_FLIGHTS} flights into project ${PROJECT_ID}."
echo "To run macro load: KNIEVEL_PROJECT=${PROJECT_ID} KNIEVEL_SITE_ID=1 \\"
echo "                   KNIEVEL_AD_TYPE=1 KNIEVEL_PID=\$(pgrep knievel) \\"
echo "                   bash bench/macro/loadgen.sh"
