# Knievel reference compose stack

Phase 4.1 deliverable. `docker compose up` boots Postgres + knievel
in under a minute against either the published
`ghcr.io/xrl/knievel:latest` image (default) or a locally-built
image (set `KNIEVEL_BUILD=1`).

## Quick start

```bash
cd examples/compose
docker compose up
# in another terminal:
curl -fsS http://localhost:8080/healthz
curl -fsS http://localhost:8080/readyz
curl -fsS http://localhost:8080/version
```

`/healthz` returns `200 ok` once the binary is listening; `/readyz`
goes green after the DB pool is up and migrations have applied.

## Building from source

The published image lands with Phase 4.3. Until then, build
locally:

```bash
KNIEVEL_BUILD=1 docker compose build
KNIEVEL_BUILD=1 docker compose up
```

The build context is the repo root; the `Dockerfile` produces a
distroless `cc:nonroot` image with the `knievel` binary at
`/usr/local/bin/knievel`. First build takes a few minutes
(downloads + compiles every workspace dep); rebuilds reuse the
dependency layer.

## Switching to a pinned digest

Production deployments should pin to a specific image digest, not
`:latest`:

```bash
KNIEVEL_IMAGE=ghcr.io/xrl/knievel@sha256:... docker compose up
```

Phase 4.3 publishes both `ghcr.io/xrl/knievel:vX.Y.Z` (semver) and
`ghcr.io/xrl/knievel:sha-<short>` (per-commit) so you have a stable
tag and an immutable digest available.

## Layout

```
examples/compose/
  compose.yaml      # Postgres + knievel + (stubbed) seed sidecar
  config.yaml       # mounted at /etc/knievel/config.yaml
  init.sql          # one-time DB bootstrap (schema + pgcrypto)
  README.md         # this file
```

This mirrors the canonical RX example in
`MIGRATION_RX.md` "Local Development for RX Engineers" — same
service names, same volume names, same healthcheck shape — so
you can read either file and recognize the structure.

## Common workflows

- **Wipe and start over:** `docker compose down -v` drops the
  Postgres volume; the next `up` re-runs `init.sql` and
  re-applies migrations from a clean slate.
- **Inspect Postgres:** `docker compose exec knievel-postgres
  psql -U knievel_app -d knievel`.
- **Tail logs:** `docker compose logs -f knievel`.

## What's stubbed

`knievel-seed` is a placeholder until Phase 4.2 wires
`knievel-cli seed-demo`. Today it polls `/readyz` and exits 0.
After 4.2 lands it'll mint a fixed dev token and seed an org +
project + advertiser + flight + ad + creative so you can issue
meaningful decisions immediately.

## Refs

- `REQUIREMENTS.md` § 8 (Deliverables, Compose manifest is item 7)
- `MIGRATION_RX.md` "Local Development for RX Engineers"
- `TESTING.md` § 11.1 (`seed-demo` as the canonical fixture)
