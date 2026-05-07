# Kind-helm E2E plan

Black-box validation that `helm install knievel` against a real
Kubernetes cluster produces a working deployment. Sits one
level above `TESTING.md` § 7 (compose-stack acceptance) — that
exercises the binary against a real Postgres; this exercises
the **chart** against a real cluster with a real Postgres
pod, plus the SPA bundle the chart serves.

The kind-helm rig is the smallest deployment shape that
matches what an operator actually runs. If this passes, the
chart isn't lying.

## 1. Why kind, why this layer

`TESTING.md` § 7 already proves the binary works. What it
doesn't prove:

- The Helm chart actually templates valid manifests against
  the values shape the chart README documents.
- Service / Deployment / ConfigMap wiring resolves end-to-end
  inside a cluster (not just in compose's flat network).
- The init / migration story works under k8s lifecycle —
  liveness/readiness probes don't bounce the pod before
  migrations finish; the SPA bundle is reachable on the same
  Service the API is served from.
- An operator's "smoke test" first hour matches what we
  document.

[`kind`](https://kind.sigs.k8s.io/) gives us a real
control-plane + kubelet on the runner, no cloud dependency.

## 2. Prerequisites

| Component | Version | Notes |
|---|---|---|
| `kind` | 0.24+ | Single-node cluster, ~30 s boot. |
| `kubectl` | 1.30+ | Whatever ships with the runner. |
| `helm` | 3.16+ | Same version pinned in `ci.yml` for `helm-lint`. |
| Container image | `ghcr.io/knievel-ads/knievel:<tag>` | Loaded into the kind node via `kind load docker-image`. |
| Postgres | `postgres:16` | Same image as compose; no operator/CRD. |

No cloud creds, no DNS, no TLS — kind runs locally on the
runner. The operator path that involves Aurora / GCP Cloud
SQL / etc. is **out of scope** for this layer; those land in
the post-v0 chaos suite (`REQUIREMENTS.md` § 10.9).

## 3. Cluster + namespace setup

```sh
kind create cluster --name knievel-e2e --wait 60s
kind load docker-image ghcr.io/knievel-ads/knievel:<tag> \
  --name knievel-e2e
kubectl create namespace knievel
kubectl config set-context --current --namespace knievel
```

Single-node cluster, single namespace. No multi-tenancy
testing here — the platform's RLS gates that and is covered
by `tests/cross_tenant_manifest.toml`.

## 4. Postgres pod

A bare `Deployment` + `Service` is enough. No
`postgres-operator`, no `Helm` for Postgres — keep it minimal
so the failure surface is just "knievel against a known-good
Postgres."

```yaml,ignore
# manifests/postgres.yaml (committed to examples/kind-e2e/)
# Two documents (Deployment + Service) — multi-doc YAML; the
# fence is `,ignore` because the doc-fence gate parses single
# documents only.
apiVersion: apps/v1
kind: Deployment
metadata: { name: knievel-postgres }
spec:
  replicas: 1
  selector: { matchLabels: { app: knievel-postgres } }
  template:
    metadata: { labels: { app: knievel-postgres } }
    spec:
      containers:
        - name: postgres
          image: postgres:16
          env:
            - { name: POSTGRES_USER,     value: postgres }
            - { name: POSTGRES_PASSWORD, value: dev }
            - { name: POSTGRES_DB,       value: knievel }
          ports: [{ containerPort: 5432 }]
          readinessProbe:
            exec:
              command: [pg_isready, -U, postgres, -d, knievel]
            periodSeconds: 2
---
apiVersion: v1
kind: Service
metadata: { name: knievel-postgres }
spec:
  selector: { app: knievel-postgres }
  ports: [{ port: 5432, targetPort: 5432 }]
```

A `Job` (or kubectl exec) provisions `knievel_app` as
NOSUPERUSER once the readiness probe passes — same shape as
`ci.yml`'s integration / acceptance jobs (rationale: see the
v0.1.7→0.1.11 release-pipeline saga; Postgres 16.13 rejects
self-alter even from a verified superuser).

## 5. Helm install

```sh
helm install knievel charts/knievel \
  --namespace knievel \
  --set image.repository=ghcr.io/knievel-ads/knievel \
  --set image.tag=<tag> \
  --set image.pullPolicy=Never \
  --set database.url='postgres://knievel_app:dev@knievel-postgres:5432/knievel' \
  --set hmac.defaultSecret=test-hmac-secret-32-bytes-of-data!! \
  --set adminUi.enabled=true \
  --wait --timeout=120s
```

`pullPolicy=Never` because the image came in via
`kind load docker-image` and isn't on a registry the cluster
can pull from. `--wait` blocks until the Deployment hits
`ready`; if the readiness probe (`/readyz`) doesn't pass
within 120 s, the job fails loudly.

## 6. Validation surface

The assertion set the user asked for, plus what the chart
actually exposes. Each step is a single curl / kubectl call;
the whole script runs in under a minute against a warm kind
cluster.

```sh
# Port-forward once for all checks.
kubectl port-forward svc/knievel 8080:8080 &
PORT_FORWARD_PID=$!
trap "kill $PORT_FORWARD_PID" EXIT
sleep 2  # let the forwarder bind
```

### 6.1 `/version` returns the running build

```sh
curl -fsS http://localhost:8080/version | jq -e '
  .package_version == "<tag-without-v>"
  and .git_sha != null
  and .build_timestamp != null
'
```

Catches: image-tag-doesn't-match-what-Helm-installed, or the
binary inside the image was built without `build.rs` running.

### 6.2 `/healthz` and `/readyz` are reachable

```sh
test "$(curl -fsS http://localhost:8080/healthz)" = "ok"
curl -fsS http://localhost:8080/readyz | grep -q '^ok'
```

Catches: container has booted but the binary is wedged.

### 6.3 The admin SPA serves at `/`

```sh
curl -fsS http://localhost:8080/admin/ | grep -q '<div id="root"'
curl -fsS http://localhost:8080/admin/config.json | jq -e '.api_base != null'
```

Catches: chart didn't bundle the SPA into the image, or
`StaticFilesEndpoint` mount was lost in the route table.

(Phase 7.11 ships the SPA bundle into the image; once that
lands, this assertion goes from "should serve a page" to
"should serve the React-mount root".)

### 6.4 OpenAPI spec is current

```sh
curl -fsS http://localhost:8080/openapi.json \
  | jq -e '.info.version == "<tag-without-v>"'
```

Catches: the spec served by the running binary doesn't
match the tag (image-rebuild was skipped, or the tag was
re-pushed onto a different commit).

### 6.5 Migrations took over Postgres

```sh
kubectl exec deploy/knievel-postgres -- \
  psql -U postgres -d knievel -tAc "
    SELECT count(*) FROM information_schema.tables
    WHERE table_schema = 'knievel'
      AND table_name IN (
        'organizations','projects','advertisers','campaigns',
        'flights','ads','creatives','sites','zones',
        'idempotency_keys','events_raw','api_tokens'
      )
  " | grep -q '^12$'
```

Catches: `auto_migrate: true` didn't run, the Job that ran
migrations crashed silently, or the schema doesn't match the
migration set committed to `migrations/`.

A second query confirms `_sqlx_migrations` exists and lists
every migration in the directory:

```sh
kubectl exec deploy/knievel-postgres -- \
  psql -U postgres -d knievel -tAc \
  "SELECT count(*) FROM knievel._sqlx_migrations" \
  > expected_migration_count.txt
ls migrations/*.sql | wc -l \
  | diff - expected_migration_count.txt
```

### 6.6 Decision endpoint round-trip

A tiny seed SQL block lands one Org / Project / Advertiser /
Campaign / Flight / Ad / Creative / Site (the `seed-demo`
shape in compressed form). Then:

```sh
curl -fsS -X POST http://localhost:8080/v1/projects/$PROJECT_ID/decisions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"placements":[{"id":"hdr","site_id":1,"ad_types":[1]}]}' \
  | jq -e '.decisions.hdr | length == 1'
```

Catches: snapshot loader didn't pick up the seed; LISTEN/NOTIFY
isn't wired through the cluster's network policy.

## 7. Cleanup

```sh
helm uninstall knievel
kubectl delete namespace knievel
kind delete cluster --name knievel-e2e
```

CI hosts deletes the kind cluster automatically when the
runner exits; we still call `delete cluster` so a successful
run leaves no orphan-images on the runner's pruning policy.

## 8. CI integration

New workflow `.github/workflows/kind-e2e.yml`:

- Trigger: on `v*` tag push (after `release.yml` publishes
  the image); also `workflow_dispatch` for manual exercise.
- Job sequence:
  1. Wait for `release.yml`'s `publish-image` job (via
     `workflow_run` predicate, or repeat-until-image-resolves
     loop with a 5-min ceiling).
  2. `kind create cluster`.
  3. Apply manifests + helm install + curl checks above.
  4. Capture `kubectl describe pod -A` and the Deployment's
     events on failure as workflow artifacts (debugging into
     CI without a kubectl shell is otherwise miserable).
  5. Delete cluster on success or failure.
- Wall-time budget: 4 minutes. Cluster boot is the dominant
  cost (~30 s); image pull is ~10 s after `kind load`; helm
  install is ~20 s; the assertion script is ~30 s.

Failure of this workflow blocks the GitHub Release flag from
flipping to "ready" but doesn't yank a published image — the
release-readiness signal lives in
`RELEASE_CHECKLIST.md`.

## 9. Out of scope (and why)

- **Multi-node cluster topology.** Single-node kind is enough
  for the chart-correctness questions this layer asks. Real
  multi-node behavior (PodDisruptionBudget, leader election
  under leader-pod-restart) lives in the chaos suite.
- **TLS / Ingress.** Operators are expected to terminate TLS
  upstream; the chart doesn't ship cert-manager wiring.
- **Custom storage classes.** Kind's default `standard`
  provisioner suffices for the Postgres PVC.
- **Real cloud Postgres.** Aurora / Cloud SQL / Supabase
  variance is the chaos suite's beat. Kind-helm tests the
  chart, not the Postgres tier.
- **Helm rollback.** First-install path only. Upgrade /
  rollback testing lands when we have a previous tag to
  upgrade from cleanly (post-v0.1.0).
- **OCI chart pull.** The CI installs from the in-tree
  `charts/knievel/` directory rather than `oci://...`. The
  publish-as-OCI path is a separate gate
  (`charts/knievel/Chart.yaml` lint + `helm push` rehearsal)
  living in `release.yml`.

## 10. Where this fits in the test pyramid

```
TESTING.md sections                  this doc
─────────────────                    ────────
§ 4   Unit / property                kind-helm sits one layer
§ 5   Integration (Postgres)          above § 7's compose
§ 6   API / contract                  acceptance — same
§ 7   E2E acceptance (compose)  ◀──── black-box assertions,
§ 8   Performance / capacity          but the SUT is the
§ 9   Chaos / degraded-mode           chart, not just the
                                      binary.
```

If § 7 is the deployable's smoke test, kind-helm is the
deployment's smoke test.
