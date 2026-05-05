# pg_partman Compatibility Notes

Knievel depends on the [`pg_partman`](https://github.com/pgpartman/pg_partman)
extension to manage daily partitions of the `events_raw` table. This
document captures what that means in practice across managed Postgres
vendors and how to set up a local development environment.

For the design context — why pg_partman, what tables it manages, how
maintenance runs — see `REQUIREMENTS.md` §7.

## Design choice: in-process maintenance, no `pg_partman_bgw`

`pg_partman` ships with two maintenance modes:

1. **Background worker (`pg_partman_bgw`).** Loaded via Postgres'
   `shared_preload_libraries`. Calls `partman.run_maintenance()` on a
   timer. Requires a Postgres restart to enable, plus
   `pg_partman_bgw.dbname` and `pg_partman_bgw.role` config values.
2. **External scheduler.** Anything that calls
   `SELECT partman.run_maintenance()` periodically — `pg_cron`, an
   OS cron job, a Cloud Scheduler hook, or — what knievel does — a
   tokio task inside the application process.

Knievel uses (2). Reasons:

- **Compatibility breadth.** The bgw needs `shared_preload_libraries`
  edited and a server restart, which not every managed Postgres
  surfaces. Cloud SQL doesn't ship the bgw at all. Doing maintenance
  in-process means knievel works anywhere the extension is installed.
- **One less moving piece.** No separate cron, no DB-side bgw config
  to drift from app-side expectations. Maintenance failure surfaces
  as a regular Sentry error from the knievel process.
- **Operator clarity.** Partition lifecycle is a knievel concern, not
  a database concern. The DBA doesn't need to know what we partition
  or how often.

We still require the **extension** to be installed (`CREATE EXTENSION
pg_partman`) — knievel calls `partman.create_parent()` and
`partman.run_maintenance()`. We just don't require the bgw component.

## Vendor Compatibility Matrix

| Vendor | Extension | bgw available | Notes |
|---|---|---|---|
| **AWS RDS Postgres** | ✅ | ✅ | Postgres 12.5+. Current versions ship pg_partman 5.2.4. |
| **AWS Aurora Postgres** | ✅ | ✅ | Postgres 12.6+. Aurora Serverless v2 supported (PG 15.4+); **Aurora Serverless v1 (PG 11.x) is NOT supported**. |
| **Azure Database for PostgreSQL Flexible Server** | ✅ | ✅ | Add `pg_partman` to `azure.extensions` server parameter; if using bgw, also list `pg_partman_bgw` in `shared_preload_libraries` and set `pg_partman_bgw.dbname`. |
| **Google Cloud SQL for PostgreSQL** | ✅ | ❌ | Extension installs cleanly. No bgw. Requires external scheduling. **Knievel's in-process maintenance design works here unchanged.** |
| **Neon** | ✅ | ✅ | `pg_partman_bgw` is preloaded by default in Neon. `CREATE EXTENSION pg_partman` is all that's needed. |
| **DigitalOcean Managed Postgres** | ✅ | ✅ | Configurable via `pg_partman_bgw_role` and `pg_partman_bgw_interval` cluster parameters. |
| **Heroku Postgres** | ✅ (Standard+) | ✅ | **Not available on the Essential tier.** Verify with `SHOW extwlist.extensions;`. |
| **Crunchy Bridge / Crunchy Postgres** | ✅ | ✅ | Crunchy Data is a primary contributor to pg_partman; first-class support. |
| **Supabase** | ❌ | — | Despite documentation referencing pg_partman, the extension is **not actually installed** on Supabase's managed Postgres. Knievel cannot run on Supabase as of 2026-05. |
| **Self-hosted Postgres** | ✅ | ✅ | `apt-get install postgresql-{N}-partman` on Debian/Ubuntu, or build from source. |

When a vendor / version isn't on this list: check their extension
catalog. If `pg_partman` isn't there, knievel can't run on it without
either landing the extension yourself (self-hosted) or moving
operationally to a different cluster.

### The Aurora Serverless v1 trap

If anyone proposes Aurora Serverless v1, push back. It's stuck on
Postgres 11.x and has never received the `pg_partman` extension.
Migration target should be Aurora Serverless v2 (which uses the same
extension catalog as standard Aurora) or standard Aurora.

### The Supabase trap

Supabase's own documentation has guides referencing `pg_partman`
configuration, but the extension is not whitelisted in their managed
service. There's a multi-year-open issue about this discrepancy. Don't
plan around being able to enable it later.

## Local Development with Docker

### Recommended image

```
ghcr.io/dbsystel/postgresql-partman:{pg_version}-{partman_version}
```

- Maintained by [dbsystel/postgresql-partman-container](https://github.com/dbsystel/postgresql-partman-container).
- Apache-2.0, nightly automated builds, signed images.
- Based on the official `postgres` image (Alpine variant) since
  August 2025; previously based on Bitnami Postgres.
- Currently builds for Postgres 14, 15, 16, 17, 18. (Postgres 13
  support was dropped in August 2025.)
- Tag patterns:
  - `16` — latest known-good pg_partman for Postgres 16.
  - `16-5.2.4` — pin both Postgres and pg_partman versions
    explicitly.

For knievel local dev, **pin both versions explicitly** so CI and
laptops match.

### Compose snippet

A minimal `compose.yaml` for `cargo run` against a local Postgres with
pg_partman installed:

```yaml
services:
  postgres:
    image: ghcr.io/dbsystel/postgresql-partman:16-5.2.4
    environment:
      POSTGRES_USER: knievel_app
      POSTGRES_PASSWORD: dev
      POSTGRES_DB: knievel
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U knievel_app -d knievel"]
      interval: 2s
      timeout: 2s
      retries: 30
    volumes:
      - knievel-pgdata:/var/lib/postgresql/data
      - ./dev/init.sql:/docker-entrypoint-initdb.d/00-init.sql:ro

volumes:
  knievel-pgdata:
```

`dev/init.sql` runs on first boot only:

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_partman;
CREATE SCHEMA IF NOT EXISTS knievel AUTHORIZATION knievel_app;
ALTER ROLE knievel_app SET search_path = knievel, public;
```

Knievel then connects with:

```
KNIEVEL_DATABASE_URL=postgres://knievel_app:dev@localhost:5432/knievel?sslmode=disable
KNIEVEL_AUTO_MIGRATE=true
```

### Building your own image

If the dbsystel image doesn't fit (e.g. you need a non-supported
Postgres version, or a non-Alpine glibc base), the layered Dockerfile
on top of the official image is short:

```dockerfile
FROM postgres:16

RUN apt-get update \
 && apt-get install -y --no-install-recommends postgresql-16-partman \
 && rm -rf /var/lib/apt/lists/*
```

The Debian variant of `postgres:16` carries the
`postgresql-16-partman` package via the PGDG repos, which the official
image already configures.

For Alpine bases or building from source: see [pg_partman's
installation
docs](https://github.com/pgpartman/pg_partman#installation) — `make`
+ `make install` against a configured Postgres dev environment, then
`CREATE EXTENSION pg_partman` in the target DB.

### Why not the official `postgres` image directly?

The official image doesn't ship pg_partman. You'd be running
`apt-get install` at first boot via an entrypoint script, which slows
container startup and is fragile across Postgres minor-version bumps.
A purpose-built image with the extension already present is the
right answer.

## Required Setup (any environment)

Once the extension is available:

```sql
-- Cluster-level (superuser).
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_partman;

-- Schema and role for knievel.
CREATE SCHEMA knievel;
CREATE ROLE knievel_app LOGIN PASSWORD :'pw';
GRANT USAGE, CREATE ON SCHEMA knievel TO knievel_app;

-- Knievel calls partman functions; grant access.
GRANT USAGE ON SCHEMA partman TO knievel_app;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA partman TO knievel_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA partman TO knievel_app;

ALTER ROLE knievel_app SET search_path = knievel, public;
```

Knievel's migrations create the partitioned table and register it
with partman:

```sql
CREATE TABLE knievel.events_raw (
  ts TIMESTAMPTZ NOT NULL,
  -- ... other columns ...
) PARTITION BY RANGE (ts);

SELECT partman.create_parent(
  p_parent_table => 'knievel.events_raw',
  p_control      => 'ts',
  p_type         => 'range',
  p_interval     => '1 day',
  p_premake      => 4
);

UPDATE partman.part_config
SET retention            = '30 days',
    retention_keep_table = false,
    retention_keep_index = false
WHERE parent_table = 'knievel.events_raw';
```

Knievel's in-process maintenance task then runs hourly:

```sql
SELECT partman.run_maintenance('knievel.events_raw');
```

## Vendor-Specific Notes

### AWS RDS / Aurora

- Use the **cluster writer endpoint** for `KNIEVEL_DATABASE_URL`
  (LISTEN/NOTIFY does not propagate to readers).
- The `rds_superuser` role typically owns the extension; the knievel
  app role only needs the grants listed above.
- Aurora Serverless v2 works; v1 does not.

### Azure Flexible Server

- Allow-list `pg_partman` in the server parameter `azure.extensions`
  before `CREATE EXTENSION` will succeed.
- If you opt into the bgw (knievel doesn't need it), also append
  `pg_partman_bgw` to `shared_preload_libraries` and set
  `pg_partman_bgw.dbname` to your knievel database name. Restart
  required.

### Google Cloud SQL

- The bgw is unavailable. Knievel's in-process maintenance is the
  intended path.
- Cloud SQL caps the number of databases per instance lower than
  Aurora; check limits if you're squeezing knievel onto an existing
  instance.

### Neon

- `pg_partman_bgw` is preloaded by default. Knievel doesn't use it
  but it being there is harmless.
- Neon's branching feature is a nice perk for testing migrations
  against a copy of prod data.

### Heroku Postgres

- Essential-tier plans don't expose pg_partman. You need Standard or
  higher.
- Verify before provisioning: `heroku pg:psql -- -c "SHOW
  extwlist.extensions;"` and grep for `pg_partman`.

### Supabase

- Don't.

## References

- [pg_partman — pgpartman/pg_partman](https://github.com/pgpartman/pg_partman)
- [AWS Aurora Postgres — Managing partitions with pg_partman](https://docs.aws.amazon.com/AmazonRDS/latest/AuroraUserGuide/PostgreSQL_Partitions.html)
- [AWS Aurora Postgres — Extension versions](https://docs.aws.amazon.com/AmazonRDS/latest/AuroraPostgreSQLReleaseNotes/AuroraPostgreSQL.Extensions.html)
- [Azure Flexible Server — Enable and use pg_partman](https://learn.microsoft.com/en-us/azure/postgresql/configure-maintain/how-to-use-pg-partman)
- [Cloud SQL for PostgreSQL — Configure extensions](https://cloud.google.com/sql/docs/postgres/extensions)
- [Cloud SQL — Dynamically drop partitions](https://cloud.google.com/blog/products/databases/dynamically-drop-partitions-within-cloud-databases/)
- [Neon — pg_partman extension docs](https://neon.com/docs/extensions/pg_partman)
- [DigitalOcean — Supported Postgres extensions](https://docs.digitalocean.com/products/databases/postgresql/details/supported-extensions/)
- [Heroku Dev Center — pg_partman whitelisted](https://devcenter.heroku.com/changelog-items/942)
- [Heroku Dev Center — Postgres extensions](https://devcenter.heroku.com/articles/heroku-postgres-extensions-postgis-full-text-search)
- [Supabase — pg_partman not available, ongoing issue](https://github.com/supabase/postgres/issues/1586)
- [Crunchy Data — pg_partman documentation](https://access.crunchydata.com/documentation/pg-partman/latest/pg_partman/)
- [dbsystel/postgresql-partman-container](https://github.com/dbsystel/postgresql-partman-container)
- [pg_partman — containerized environments discussion](https://github.com/pgpartman/pg_partman/discussions/582)
