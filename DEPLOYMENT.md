# Deployment

Operator's guide to running knievel in production. Prescriptive where
the spec allows; opinionated where it doesn't. The happy path is
**self-contained** — a reader following this doc shouldn't need
`REQUIREMENTS.md` open to get a working install. Deep dives are
linked, not inlined.

## 1. Prerequisites

| Component | Required version | Notes |
|---|---|---|
| Postgres | 14 or later | Cluster writer endpoint reachable from knievel pods. `pgcrypto` extension available (every cloud provider ships it). Partitions are managed in-process by knievel; no `pg_partman` install needed. |
| S3-compatible object store | any | AWS S3, MinIO, R2, Cloudflare R2, GCS via the S3 interop layer, etc. Used for creative images. |
| Container runtime | OCI | knievel ships as a multi-arch container image (`linux/amd64` + `linux/arm64`). Helm chart for Kubernetes; compose manifest for single-node. |
| OpenTelemetry collector | any | Optional but recommended. OTLP/gRPC endpoint. |
| Sentry | any tier | Optional. DSN supplied via env. |
| Argon2id support | n/a | Token verification uses argon2; no platform dep — bundled into the binary. |

A single-node compose stack (Postgres + knievel + a seed sidecar) is
documented in § 5.

## 2. Sizing guidance

Per `REQUIREMENTS.md` § 9. **TARGET (unverified)** until the v0.1.0
benchmark in `bench/results/v0.1.md` lands; numbers below are upper
bounds the design supports, not measurements.

**Per pod:**

- CPU: 1 vCPU baseline; 2 vCPU under load.
- Memory: 512 MiB baseline; 1 GiB with snapshots over 10k ads.
- DB connections: 12 (configurable; see `REQUIREMENTS.md` § 7.8).
- Decision throughput: ~5k req/s at SLO.
- Event ingest: ~50k events/s (in-memory channel + `COPY` batches).

**Postgres:**

- Writer instance scales with event-write throughput, not decision
  rate.
- Connection budget per knievel pod × pod count must fit inside the
  Postgres connection limit. Aurora's `max_connections` is the
  practical ceiling for cloud deploys; raise the parameter group, not
  the pod count, when in doubt.
- Read replicas don't speed up decisions (RAM snapshot does). They
  do help downstream warehouse copies.

**Object store:** independent. Pick whatever is closest to your
serving region.

## 3. Database setup

### Schema + role

```sql
-- Run once, before knievel boots.
CREATE DATABASE knievel;
\c knievel

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE ROLE knievel_app LOGIN PASSWORD '<vault-it>';
ALTER ROLE knievel_app SET search_path TO knievel, public;

CREATE SCHEMA knievel AUTHORIZATION knievel_app;

-- Optional: a read-only role for warehouse extracts (see REPORTING.md).
CREATE ROLE knievel_reader LOGIN PASSWORD '<vault-it>';
ALTER ROLE knievel_reader SET search_path TO knievel, public;
GRANT USAGE ON SCHEMA knievel TO knievel_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA knievel TO knievel_reader;
ALTER DEFAULT PRIVILEGES FOR ROLE knievel_app IN SCHEMA knievel
  GRANT SELECT ON TABLES TO knievel_reader;
```

### Important: `knievel_app` is **NOT a superuser**

Postgres `FORCE ROW LEVEL SECURITY` is bypassed by superusers. The
reference compose stack (`examples/compose/init.sql`) downgrades
`knievel_app` to `NOSUPERUSER CREATEDB` immediately after creation.
The CI test harness does the same. **If you provision the role with
`SUPERUSER`, RLS will appear to work in dev and silently fail in
production.** This is gotcha 17 in `CLAUDE.md`.

### Backups

Backup is **operator-owned**. Knievel doesn't ship a backup helper.
Recommended baselines:

- **Daily logical dumps** of the `knievel` schema (excluding
  `events_raw` partitions if they're large; see `REPORTING.md` for
  the warehouse-copy alternative).
- **PITR (point-in-time recovery)** via the cloud provider's tier
  if you're on Aurora or Cloud SQL.
- **Test restores** quarterly. Untested backups don't exist.

## 4. Helm install

The chart at `charts/knievel/` is the recommended path. Published as
an OCI artifact; install:

```sh
helm install knievel oci://ghcr.io/knievel-ads/charts/knievel \
  --version 0.1.0 \
  -f values.yaml
```

A starter `values.yaml` for a single-replica cluster:

```yaml
image:
  repository: ghcr.io/knievel-ads/knievel
  tag: v0.1.0          # or a digest: 'sha256:<digest>' for immutability
  pullPolicy: IfNotPresent

replicaCount: 1

resources:
  requests:
    cpu: 500m
    memory: 512Mi
  limits:
    cpu: 2000m
    memory: 1Gi

database:
  url: ${KNIEVEL_DATABASE_URL}    # `${VAR}` interpolation in config.yaml
  maxConnections: 12

events:
  retentionDays: 30
  channelCapacity: 8192

hmac:
  defaultSecretRef:
    name: knievel-hmac
    key: default

sentry:
  dsn: ${KNIEVEL_SENTRY_DSN}

otel:
  endpoint: http://otel-collector:4317

ingress:
  enabled: true
  className: nginx
  hosts:
    - host: api.knievel.example
      paths: [{ path: /, pathType: Prefix }]

serviceAccount:
  create: true
  annotations: {}

securityContext:
  runAsNonRoot: true
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
```

The chart wires Postgres password and Sentry DSN through env vars
referenced via `${VAR}` interpolation; what gets injected and how is
covered in § 7.

## 5. Compose install

Single-binary + bring-your-own-Postgres. Same compose stack the
acceptance tests in `TESTING.md` § 7 run against:

```sh
docker compose -f examples/compose/compose.yaml up -d
# Postgres comes up, knievel comes up, knievel-seed runs once and exits
# bearing a fixture project + token at ./tmp/knievel-dev-token.

curl -fsS http://localhost:8080/healthz
```

The compose default pulls the published image; set `KNIEVEL_BUILD=1`
to build from the local checkout instead. See
`examples/compose/README.md` for the full set of overrides.

For long-lived single-node deployments, run compose under a process
supervisor (systemd, etc.) and back the Postgres data volume up
yourself. The compose layout is intentionally close to the chart
(same env var names, same healthcheck shape) so the operational
muscle memory carries between them.

## 6. Bare metal / systemd

The container image is the blessed path. Bare-metal users get the
static `knievel` and `knievel-cli` binaries from the GitHub Release
page (one per `linux/{amd64,arm64}-musl`, plus
`{x86_64,aarch64}-apple-darwin` for laptop installs) plus a sample
unit file:

```ini
# /etc/systemd/system/knievel.service
[Unit]
Description=knievel
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
User=knievel
Group=knievel
Environment=KNIEVEL_CONFIG=/etc/knievel/config.yaml
ExecStart=/usr/local/bin/knievel
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/etc/knievel

[Install]
WantedBy=multi-user.target
```

You own backups, monitoring, and process supervision. The chart
covers all three when you go that route.

## 7. Secrets management

Three secrets the operator owns:

| Secret | What it does | Sourced from |
|---|---|---|
| `KNIEVEL_DATABASE_URL` | Connection string with `knievel_app` password | Vault / Secrets Manager / chart-templated `Secret` |
| `KNIEVEL_HMAC_DEFAULT_SECRET` | Mints/verifies impression+click URL signatures | Same; rotate via the procedure in `AUTH.md` |
| `KNIEVEL_SENTRY_DSN` | Sentry project endpoint (optional) | Same |

All three are projected into the container as env vars. `config.yaml`
references them with `${VAR}` interpolation:

```yaml
database:
  url: ${KNIEVEL_DATABASE_URL}
hmac:
  default_secret: ${KNIEVEL_HMAC_DEFAULT_SECRET}
observability:
  sentry:
    dsn: ${KNIEVEL_SENTRY_DSN}
```

The Helm chart's `secrets:` block templates a `Secret` and projects
it into the deployment's env. Operators with a centralized secret
store (Vault / external-secrets-operator) project the same env vars
and the chart picks them up unchanged.

API tokens (`kvl_<env>_<scope>_<short-id>_<secret>`) are NOT operator
secrets. They're minted via the management API after the first
admin token is bootstrapped — see `AUTH.md` § "First-token
bootstrap."

## 8. Migrations

Two paths:

1. **`auto_migrate: true`** (default). Knievel runs `sqlx::migrate!`
   on startup before listening for traffic. Pod won't accept requests
   until migrations are applied. Suitable for single-instance
   deploys and rolling restarts where the first pod's migration is
   the gate.

2. **`auto_migrate: false`** + `knievel-cli migrate`. Run migrations
   out-of-band from a Job (Helm hook) or a manual one-off. Suitable
   for operators who want migration auditing separate from pod boot,
   or who have multi-region deploys where one region runs migrations
   and others wait for replication.

Migrations are **additive only** (`REQUIREMENTS.md` § 6.4). The
schema gets a new column, a new table, a new index — never a column
removal in v0. The migration linter rejects RLS-incomplete migrations
in CI (`xtask lint-migrations`).

## 9. Upgrades

Knievel pods are stateless by design. Rolling restart story:

1. Apply migrations (auto on first boot, or out-of-band per § 8).
2. Roll pods one at a time. The advisory-lock leader re-elects on
   each pod loss; the snapshot loader catches up via poll within 5 s.
3. Old pods drain via `/readyz` flipping 503 (the Helm chart's
   default `terminationGracePeriodSeconds: 30` is enough).

An Aurora failover during an upgrade looks like a 1–10 s blip in
`/readyz` while the writer endpoint moves; the LB drains the affected
pod, the snapshot loader's reconnect handler re-establishes
LISTEN/NOTIFY, the poll backstop catches up. No state loss.

## 10. Multi-region

**Single-region in v0.** Cross-region active-active is out of scope
— the snapshot's 5 s staleness window doesn't survive a cross-region
hop, and the partition manager's leader election can't span regions
either.

The Helm chart exposes `affinity` and `topologySpreadConstraints` for
multi-AZ within a region. An active-passive operator pattern
(primary region serves; secondary region is warm but not taking
traffic; manual failover via DNS) is the recommended cross-region
shape.

## 11. Observability setup

Three streams, all carrying the same `request_id` / `trace_id`:

- **Metrics**: scrape `/metrics` (Prometheus exposition format).
  Default-low cardinality — no per-tenant metrics unless explicitly
  enabled via `metrics.per_tenant_enabled: true`. The chart wires a
  `ServiceMonitor` for the `prometheus-operator` flavor.
- **Traces**: OTLP/gRPC to whatever endpoint `otel.endpoint` points
  at. Default sampler is parent-based with a 10% root-sample rate;
  override via `otel.sampler` if you want every-request tracing
  during an investigation.
- **Errors**: Sentry via `sentry-tower`. Per-request hub keeps
  fingerprinting per-tenant. 5xx files an issue; 4xx and timeouts
  don't.

`observability.logging.format: json` is the default and matches
what container log aggregators expect. `format: compact` is the
human-readable variant for `docker compose logs`.

## 12. Alerts and dashboards

Six operator-actionable thresholds (per `REQUIREMENTS.md` § 9.3).
Sample PromQL alerts checked into `examples/observability/`:

| Alert | Threshold | Likely cause |
|---|---|---|
| `knievel_decision_p99_high` | p99 > 50 ms for 5 min | Snapshot scaled past pod RAM; bump pod size or shard projects. |
| `knievel_event_channel_drops` | `events.dropped` rate > 0 for 1 min | Channel capacity too small for traffic — bump `events.channelCapacity`. |
| `knievel_db_connections_saturated` | pool wait time > 1 s | Connection budget too low; bump per-pod cap or scale pods. |
| `knievel_snapshot_stale` | `snapshot.staleness_seconds > 30` | LISTEN dropped + poll backstop also failing. Investigate DB connectivity. |
| `knievel_jwks_unreachable` | `auth.jwks_fetch_failures` > 0 for 5 min | JWKS endpoint outage — auth still works for cached keys, will degrade. |
| `knievel_partition_creation_late` | newest partition < 1 day ahead of now | Leader stuck; check `partition.leader_changes` metric. |

Dashboards-as-code (Grafana JSON) live next to the alerts in
`examples/observability/`.

## 13. Runbooks

Stub each — flesh out once an incident actually happens. Living
versions live in `examples/observability/runbooks/`.

- **DB writer unreachable**: `/readyz` 503 → LB drains. Fix Postgres,
  pods come back. No data loss for already-buffered events.
- **Snapshot stale**: `snapshot.staleness_seconds > 30`. Restart the
  affected pod; the new boot rebuilds the snapshot from scratch.
- **Event channel saturated**: `events.dropped` ticking. Bump
  `events.channelCapacity`; consider sharding by project.
- **Leader maintenance failure**: advisory lock churn means the
  leader is dropping ownership repeatedly. Symptom:
  `partition.leader_changes` > 1 per hour. Investigate Postgres
  connection stability.
- **JWKS endpoint unreachable**: tokens minted before outage keep
  working until cache TTL; new tokens 401. Fix upstream IdP,
  knievel auto-recovers on next refresh.
- **Connection-pool exhaustion**: handlers wait on the pool; p99
  spikes. Either scale pods (add capacity), scale Postgres
  connection limit, or shed load via the LB.

## 14. Troubleshooting

Short FAQ keyed off `error.code` values callers see:

| Code | Means | Operator action |
|---|---|---|
| `no_db` | No `database.url` configured | Fix config; restart pod. |
| `db_error` | Postgres returned an unexpected error | Check Postgres logs + connection budget. |
| `forbidden` | RLS rejected the query | Caller's principal doesn't bind the right org/project. Verify the token. |
| `invalid_cursor` | Pagination cursor is corrupt or for a different resource | Caller's bug. Confirm the cursor came from the right `list*` endpoint. |
| `invalid_limit` | `?limit=N` outside `[1, 500]` | Caller's bug. |
| `bad_signature` (event endpoints) | HMAC rotation window passed | If frequent, check that signing-secret rotation completed cleanly. |
| `external_id_conflict` | POST hit a row with the same `external_id` | Caller should use `:batchUpsert` for idempotent writes. (POST-side parity moves to Phase 6.1.) |

`API.md` § "Errors" has the full taxonomy.

## 15. Decommissioning

Drop in this order:

1. **Stop traffic**: scale knievel deployment to 0 replicas (chart:
   `replicaCount: 0`).
2. **Final warehouse extract** (if needed): `COPY (SELECT … FROM
   knievel.events_raw) TO …` per `REPORTING.md`.
3. **Drop the schema**: `DROP SCHEMA knievel CASCADE`.
4. **Drop the role**: `DROP ROLE knievel_app`. Same for
   `knievel_reader` if you provisioned it.
5. **Delete the secrets**: HMAC default secret, Sentry DSN, DB
   password.
6. **Helm uninstall**: `helm uninstall knievel`. Delete the namespace
   if it was knievel-only.

Object-store creative images persist independently — the operator
owns the bucket lifecycle.
