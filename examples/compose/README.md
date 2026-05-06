# Knievel reference compose stack

Phase 4.1 deliverable. `docker compose up` boots Postgres + knievel
in under a minute against either the published
`ghcr.io/knievel-ads/knievel:latest` image (default) or a locally-built
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
KNIEVEL_IMAGE=ghcr.io/knievel-ads/knievel@sha256:... docker compose up
```

Phase 4.3 publishes images **on semver tags only** (no main-branch
pushes — every published image is a deliberate release). After
the first release, `ghcr.io/knievel-ads/knievel:vX.Y.Z` is the immutable
tag; `ghcr.io/knievel-ads/knievel:latest` re-points to the freshest semver
release. Pre-release work pins to a digest from a `vX.Y.Z-rc.N`
tag or builds locally via `KNIEVEL_BUILD=1`.

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

## Seeding the demo data

`knievel-seed` runs `knievel-cli seed-demo` once on first compose
up — connects to Postgres directly, idempotently provisions an
org / project / advertiser / campaign / flight / ad / creative /
site / zone, and writes a fixed dev bearer to
`./tmp/knievel-dev-token` (path is `examples/compose/../../tmp/`,
i.e. the repo root's `tmp/`). Re-running compose up after data
already exists is a no-op apart from a hash rotation on the
bootstrap token.

```bash
docker compose up                         # one-shot bootstrap
TOKEN=$(cat ../../tmp/knievel-dev-token)  # the demo bearer
curl -fsS -X POST http://localhost:8080/v1/projects/<pj>/decisions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"placements": [{"id": "header", "ad_type_id": <id>}]}'
```

The project and ad-type ids are printed on the seed-demo log line:
`docker compose logs knievel-seed`.

## Refs

- `REQUIREMENTS.md` § 8 (Deliverables, Compose manifest is item 7)
- `MIGRATION_RX.md` "Local Development for RX Engineers"
- `TESTING.md` § 11.1 (`seed-demo` as the canonical fixture)
