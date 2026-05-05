# Knievel Reporting

Knievel's data model is shaped to make downstream reporting easy.
The expected pattern is **dbt running against the same Postgres
cluster**, reading from `knievel.*` schemas alongside the operator's
existing analytics tables. No ETL hop, no separate warehouse, no
JSON unpacking.

This document covers the access pattern, the schema for reporters,
and worked dbt examples mapping `knievel.events_raw` into a typical
medallion layout.

For the design context — why partitioned events, what's in
`events_rollup`, the in-process partition manager — see
`REQUIREMENTS.md` §7.

## Why this is efficient

Knievel writes events as plain typed rows into a daily-partitioned
table. The expensive things — JSON unpacking, cross-database ETL,
schema drift — don't apply.

- **Same Postgres cluster.** dbt models read `knievel.*` like any
  other source. No Fivetran, no `pg_dump`, no warehouse sync. Joins
  to operator-owned tables in `public.*` (or wherever) are local.
- **Daily partitions on `ts`.** A dbt incremental model filtered to
  yesterday's data scans one partition. A 7-day backfill scans 7.
  Queries that don't filter by `ts` would scan everything — but
  reporting queries always filter by time, so partition pruning is
  always in play.
- **No secondary indexes on `events_raw`.** Indexes would slow the
  `COPY` ingest path on knievel's side, and partition pruning makes
  them unnecessary for the reporting workload. If a specific report
  is slow, the dbt model materializes it (bronze → silver → gold);
  the gold layer is where you index, not the source.
- **Append-only.** Bronze incremental models use `WHERE ts >
  max(ts)` and are wrong-by-zero — no late updates, no UPSERTs to
  reconcile.
- **Typed columns.** Every event field is a typed column. No
  `event_payload->>'foo'` casting; the column either exists or it
  doesn't.
- **Stable schema.** New event kinds = new rows in the same shape.
  New columns = additive only, behind sqlx migrations. No breaking
  schema changes without a major version bump.

## Access Pattern

Reporting consumers (dbt, BI tools, ad-hoc analysts) connect using
a dedicated **read-only Postgres role**, separate from the
`knievel_app` role knievel itself uses.

```sql
-- One-time, run by a superuser (or via IaC).
CREATE ROLE knievel_reader;

GRANT USAGE ON SCHEMA knievel TO knievel_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA knievel TO knievel_reader;

-- Cover future tables (new partitions, new dimensions in migrations).
ALTER DEFAULT PRIVILEGES FOR ROLE knievel_app IN SCHEMA knievel
  GRANT SELECT ON TABLES TO knievel_reader;
```

Then grant `knievel_reader` membership to the actual analytics user:

```sql
-- The dbt service account, or your warehouse user, or whoever.
GRANT knievel_reader TO dbt_service;
```

Knievel itself never reads through `knievel_reader`. It exists
purely so the operator can give the dbt pipeline `SELECT` access
without giving it any chance to mutate knievel state.

In a shared-cluster deployment (knievel and the host application
share a Postgres cluster, like RX in `MIGRATION_RX.md`), `dbt_service`
is the existing dbt role with one extra grant added.

### Reading from a replica

Aurora and most managed Postgres support read replicas. Point heavy
dbt jobs at the **reader endpoint** to keep the writer's I/O budget
focused on knievel's hot path. Replica lag is typically sub-second
and well within the cadence of any dbt schedule.

(Reminder: knievel itself MUST connect to the writer endpoint —
`LISTEN/NOTIFY` doesn't propagate to readers. dbt has the opposite
constraint.)

## Schema for Reporters

The interesting tables for reporting:

### `knievel.events_raw` (fact source)

Append-only, partitioned by day on `ts`. One row per event of any
kind.

| Column | Type | Notes |
|---|---|---|
| `ts` | `timestamptz` | Partition key. Event time. |
| `org_id` | `bigint` | |
| `project_id` | `bigint` | |
| `kind` | `smallint` | Enum: `1=decision`, `2=impression`, `3=click`. |
| `placement_id` | `text` | The caller-supplied placement id. |
| `site_id` | `bigint` | |
| `zone_id` | `bigint` | Nullable. |
| `ad_id` | `bigint` | Nullable on `decision` rows that selected nothing. |
| `creative_id` | `bigint` | |
| `flight_id` | `bigint` | |
| `campaign_id` | `bigint` | |
| `advertiser_id` | `bigint` | |
| `url` | `text` | From decision-time `context.url`. |
| `referrer_host` | `text` | Hostname only. |
| `user_agent_hash` | `bytea` | Hash, not raw UA. |
| `signature_nonce` | `bytea` | Per-event nonce; useful as a unique key. |
| `dedup_key` | `bytea` | Stable per (event, signature) for impression-replay dedup. |

**Default retention:** 30 days. Rolling reports older than that
must materialize their own retention in dbt.

### `knievel.events_rollup` (pre-aggregated facts)

Hourly aggregates by `(project_id, site_id, zone_id, flight_id,
ad_id, creative_id, kind)`, computed by knievel's leader-elected
job before raw partitions age out. Indefinite retention.

| Column | Type |
|---|---|
| `hour` | `timestamptz` (truncated to hour) |
| `project_id` | `bigint` |
| `site_id` | `bigint` |
| `zone_id` | `bigint` |
| `flight_id` | `bigint` |
| `ad_id` | `bigint` |
| `creative_id` | `bigint` |
| `kind` | `smallint` |
| `count` | `bigint` |

Useful as a cheaper bronze input for dashboards that don't need raw
event detail. Don't double-count by combining `events_rollup` with
the same time range from `events_raw`.

### Dimensional tables

These are mutable — names change, flights get extended, etc. dbt
should snapshot them if SCD2 history matters; otherwise treat them
as current-state lookups.

| Table | Purpose |
|---|---|
| `knievel.organizations` | Top-level tenant. Rare changes. |
| `knievel.projects` | Project-per-tenant directory. |
| `knievel.advertisers` | Advertiser dimension. |
| `knievel.campaigns` | Campaign dimension. Has `advertiser_id`. |
| `knievel.flights` | Flight dimension. Has `campaign_id`, dates, priority, targeting (`site_ids`, `zone_ids`, `ad_types`). |
| `knievel.ads` | Ad dimension. Has `flight_id`, `creative_id`, weight. |
| `knievel.creatives` | Creative dimension. Type, dimensions, template id. |
| `knievel.creative_templates` | Native-ad template definitions. |
| `knievel.sites` | Site dimension. Has `url`, `aliases`, `channel_id`. |
| `knievel.zones` | Zone dimension. Has `site_id`. |
| `knievel.channels` | Channel grouping. |
| `knievel.priorities` | Priority tiers. |
| `knievel.ad_types` | Ad type catalog. |

## dbt Integration

A typical medallion layout against knievel:

```
sources (knievel.*)
  └── bronze (raw, lightly cleaned)
        └── silver (joined with dimensions, one model per business concept)
              └── gold (aggregates, fact tables, reports)
```

### `_sources.yml`

```yaml
version: 2

sources:
  - name: knievel
    schema: knievel
    description: "Knievel ad-serving platform."
    freshness:
      warn_after: { count: 1, period: hour }
      error_after: { count: 6, period: hour }
    loaded_at_field: ts
    tables:
      - name: events_raw
        description: "Append-only event facts (decisions, impressions, clicks)."
        columns:
          - name: ts
            description: "Event time. Partition key."
            tests:
              - not_null
          - name: kind
            description: "1=decision, 2=impression, 3=click."
      - name: events_rollup
      - name: advertisers
        loaded_at_field: updated_at
      - name: campaigns
      - name: flights
      - name: ads
      - name: creatives
      - name: sites
      - name: zones
      - name: projects
      - name: organizations
```

### Bronze: incremental copy with light cleanup

```sql
-- models/bronze/knievel_events.sql
{{ config(
    materialized='incremental',
    unique_key='signature_nonce',
    incremental_strategy='append',
    on_schema_change='sync_all_columns'
) }}

SELECT
  ts,
  org_id,
  project_id,
  CASE kind
    WHEN 1 THEN 'decision'
    WHEN 2 THEN 'impression'
    WHEN 3 THEN 'click'
  END AS kind,
  placement_id,
  site_id,
  zone_id,
  ad_id,
  creative_id,
  flight_id,
  campaign_id,
  advertiser_id,
  url,
  referrer_host,
  signature_nonce
FROM {{ source('knievel', 'events_raw') }}
{% if is_incremental() %}
  WHERE ts > (SELECT max(ts) - interval '1 hour' FROM {{ this }})
{% endif %}
```

The `- interval '1 hour'` overlap absorbs flusher batches that may
have arrived late; deduped by `signature_nonce`.

### Silver: one model per event kind, joined with dims

```sql
-- models/silver/ad_impressions.sql
SELECT
  e.ts                       AS impression_ts,
  e.project_id,
  e.placement_id,

  -- Advertiser
  e.advertiser_id,
  a.name                     AS advertiser_name,
  a.external_id              AS advertiser_external_id,

  -- Campaign
  e.campaign_id,
  c.name                     AS campaign_name,

  -- Flight
  e.flight_id,
  f.name                     AS flight_name,
  f.start_date               AS flight_start,
  f.end_date                 AS flight_end,
  f.priority_id,

  -- Creative
  e.creative_id,
  cr.name                    AS creative_name,
  cr.type                    AS creative_type,

  -- Ad
  e.ad_id,
  ad.weight                  AS ad_weight,

  -- Inventory
  e.site_id,
  s.url                      AS site_url,
  s.name                     AS site_name,
  e.zone_id,
  z.name                     AS zone_name,

  -- Context
  e.url                      AS page_url,
  e.referrer_host

FROM {{ ref('knievel_events') }}            e
JOIN {{ source('knievel', 'advertisers') }} a  ON a.id  = e.advertiser_id
JOIN {{ source('knievel', 'campaigns') }}   c  ON c.id  = e.campaign_id
JOIN {{ source('knievel', 'flights') }}     f  ON f.id  = e.flight_id
JOIN {{ source('knievel', 'ads') }}         ad ON ad.id = e.ad_id
JOIN {{ source('knievel', 'creatives') }}   cr ON cr.id = e.creative_id
JOIN {{ source('knievel', 'sites') }}       s  ON s.id  = e.site_id
LEFT JOIN {{ source('knievel', 'zones') }}  z  ON z.id  = e.zone_id
WHERE e.kind = 'impression'
```

`ad_clicks` and `ad_decisions` are siblings with `kind = 'click'` /
`'decision'`. Click rows additionally need `redirect_target` if you
add it (knievel can include it on click events).

### Gold: aggregate fact tables

```sql
-- models/gold/fact_daily_ad_performance.sql
{{ config(materialized='table') }}

WITH events AS (
  SELECT 'impression' AS kind, ad_id, creative_id, flight_id,
         campaign_id, advertiser_id, project_id, site_id,
         impression_ts AS ts
  FROM {{ ref('ad_impressions') }}
  UNION ALL
  SELECT 'click' AS kind, ad_id, creative_id, flight_id,
         campaign_id, advertiser_id, project_id, site_id,
         click_ts AS ts
  FROM {{ ref('ad_clicks') }}
)
SELECT
  date_trunc('day', ts)::date AS day,
  project_id,
  advertiser_id,
  campaign_id,
  flight_id,
  ad_id,
  creative_id,
  site_id,
  count(*) FILTER (WHERE kind = 'impression') AS impressions,
  count(*) FILTER (WHERE kind = 'click')      AS clicks,
  count(*) FILTER (WHERE kind = 'click')::numeric
    / NULLIF(count(*) FILTER (WHERE kind = 'impression'), 0) AS ctr
FROM events
GROUP BY 1, 2, 3, 4, 5, 6, 7, 8
```

### Snapshots for SCD2 history

If your reports care about "what was the campaign called when this
impression happened" (vs. its current name), snapshot the dimension
tables:

```sql
-- snapshots/campaigns_snapshot.sql
{% snapshot campaigns_snapshot %}
{{ config(
    target_schema='analytics_snapshots',
    unique_key='id',
    strategy='timestamp',
    updated_at='updated_at'
) }}
SELECT * FROM {{ source('knievel', 'campaigns') }}
{% endsnapshot %}
```

Then silver models join against the snapshot at event time rather
than the live dimension table.

## Performance Notes

### Sizing expectations

At 20k events/sec sustained, 30 days of `events_raw` is on the
order of:

- ~52 B rows / day — partitioned, so daily queries scan one
  partition
- Each row is ~200 B uncompressed; ~10 GB / day raw, ~3–4 GB after
  Postgres' page-level compression
- 30 days = 100–300 GB on disk

Most operators will be at 1–2 orders of magnitude lower volume.
Reporting queries against a single day's partition are fast even
without indexes.

### When to add indexes on raw

Don't reflexively. Profile a slow report first. The shape that
sometimes warrants an index:

- Frequent dashboard query filters on `(project_id, advertiser_id,
  ts)` and the partition pruning isn't enough — add an index on
  `(project_id, advertiser_id)` per leaf partition.

Adding via `pg_partman`-style template? We don't use pg_partman, so
new partitions need the index applied at creation time. The Rust
partition manager creates partitions; if you need indexes, configure
them in `partitions.partition_indexes` (operator-supplied SQL run
after `CREATE TABLE ... PARTITION OF ...`).

### Materializing for BI

For real-time dashboards, point BI tools at `events_rollup` rather
than `events_raw`. For historical analysis (>30 days), rely on dbt
gold tables that have moved beyond knievel's retention window.

## What Knievel Ships to Help

- `knievel_reader` role grants documented in `MIGRATION_RX.md` and
  the operator's quickstart.
- An `examples/dbt/` skeleton in the knievel repo with the source
  YAML and one-each bronze/silver/gold model. Operators copy/paste
  into their dbt project to get started.
- Foreign key constraints between events tables and dimension
  tables are **not** enforced (would slow ingest); dbt models
  joining without FKs is fine.

## What's Out of Scope (v0)

- A built-in reporting API on knievel itself. Operators with dbt
  pipelines don't need it; deployments without dbt can compute from
  `events_rollup` directly. Native reporting endpoints are on the
  roadmap (`REQUIREMENTS.md` §11).
- Real-time streaming (CDC) of `events_raw` to Kafka or similar.
  Postgres + dbt incremental models is sufficient for batch and
  near-real-time. Streaming is a roadmap item if a use case emerges.
- ML feature stores. Out of scope for the platform; build them
  downstream in your warehouse.

## References

- [dbt Sources](https://docs.getdbt.com/docs/build/sources)
- [dbt Incremental Models](https://docs.getdbt.com/docs/build/incremental-models)
- [dbt Snapshots](https://docs.getdbt.com/docs/build/snapshots)
- [Medallion Architecture (Databricks)](https://www.databricks.com/glossary/medallion-architecture)
