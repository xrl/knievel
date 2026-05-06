# knievel

Knievel — fearlessly fast ad delivery that steals the show.

This is the official Helm chart for [knievel](https://github.com/xrl/knievel),
a Rust ad-serving platform inspired by Kevel's domain model.

## TL;DR

```bash
helm install knievel oci://ghcr.io/xrl/charts/knievel \
  --set database.host=aurora-cluster.example.com \
  --set database.name=knievel \
  --set database.existingSecret=knievel-db
```

## Prerequisites

- Kubernetes ≥ 1.27 (the chart uses standard apps/v1 + networking.k8s.io/v1).
- A reachable Postgres 14+ cluster (Aurora-PostgreSQL recommended per
  `REQUIREMENTS.md` § 8 ("native Aurora-PostgreSQL")).
- A `Secret` carrying the DB username and password (referenced via
  `database.existingSecret`).
- Optional: kube-prometheus-stack for `ServiceMonitor` scraping.

## Values

The full surface is documented in `values.yaml`. Highlights:

| Path                       | Purpose                                                         |
|----------------------------|-----------------------------------------------------------------|
| `image.repository`         | Default `ghcr.io/xrl/knievel`. Pin a digest in `image.tag`.     |
| `replicaCount`             | Default 2. The platform is stateless, scale horizontally.       |
| `database.host`            | Aurora cluster writer endpoint. Required.                       |
| `database.existingSecret`  | Secret with `username` + `password` keys.                       |
| `database.autoMigrate`     | Run migrations at boot. `true` for v0; flip off in CD pipelines that run migrations as a separate job. |
| `events.retentionDays`     | Per `REQUIREMENTS.md` § 7.3 (default 30).                       |
| `decisions.forceOverridesEnabled` | Project-level kill-switch on `force.*` overrides.        |
| `sentry.*` / `otel.*`      | Both off by default; enable per environment.                    |
| `serviceMonitor.enabled`   | Emits a `monitoring.coreos.com/v1` ServiceMonitor.              |
| `ingress.*`                | Off by default; configure for north-south traffic.              |

## Multi-AZ / topology spread

For HA across availability zones (a soft expectation in v0,
graduating to a hard one in Phase 5):

```yaml
topologySpreadConstraints:
  - maxSkew: 1
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: DoNotSchedule
    labelSelector:
      matchLabels:
        app.kubernetes.io/name: knievel

affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          topologyKey: topology.kubernetes.io/zone
          labelSelector:
            matchLabels:
              app.kubernetes.io/name: knievel
```

`whenUnsatisfiable: DoNotSchedule` makes the constraint hard — pods
won't schedule if a zone is missing capacity, surfacing the problem
loudly. The soft `podAntiAffinity` adds a tie-breaker for the
scheduler when capacity is plentiful.

## Pinning a digest

Pin a per-commit `sha-<short>` digest in `image.tag` for reproducible
deploys (the per-commit images are published by
`.github/workflows/main-image.yml`):

```bash
helm upgrade knievel oci://ghcr.io/xrl/charts/knievel \
  --set image.tag=sha256:<digest>
```

The chart honors any `tag` starting with `sha256:` and renders the
image reference as `repository@sha256:...` instead of `repository:tag`.

## Verifying the image

The image is cosign-signed keyless via GitHub OIDC. Verify before
pulling into a cluster:

```bash
cosign verify ghcr.io/xrl/knievel@sha256:<digest> \
  --certificate-identity-regexp 'https://github.com/xrl/knievel/.github/workflows/release.yml.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## Refs

- `REQUIREMENTS.md` § 8 (Deliverables, Helm chart is item 6) and § 8.1.
- `TESTING.md` § 12.4 / § 12.6 (Helm-related CI gates).
- The chart is `helm lint` clean and `kubeconform`-validated against
  Kubernetes 1.30 + the kube-prometheus-stack CRDs (for `ServiceMonitor`).
