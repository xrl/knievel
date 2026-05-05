# RX → Knievel Migration Guide

How RX moves from Kevel to knievel without behavior change. Companion
to `REQUIREMENTS.md` and `API.md`; **not part of the platform spec**.
Knievel is a general-purpose ad platform; this document is one
consumer's mapping.

## Topology

| RX environment | Knievel Org | Knievel Projects |
|---|---|---|
| `production` | `scientist-com-prod` | one per RX Organization |
| `staging` | `scientist-com-staging` | one per RX Organization |

- One **knievel Org** per RX environment.
- One **knievel Project** per RX Organization (`az`, `pfizer`, …) —
  including the long tail of small marketplaces. Project provisioning
  is a single idempotent API call, so spinning up a new RX Organization
  costs one round-trip.
- One **Org Editor token** per environment, held by the Rails app.
  Used for both sync and decision calls. Project ID is supplied per
  call (`/v1/projects/{projectId}/...`).

## Concept Map

| RX | Kevel today | Knievel |
|---|---|---|
| Environment (`prod`, `staging`) | API key + `KEVEL_NETWORK_ID` | Org |
| RX Organization (marketplace) | (implicit; resolved by URL at decision time) | Project |
| Provider | Advertiser | Advertiser |
| `KevelAdConfiguration.campaign_name` | Campaign name | Campaign |
| `KevelAd` (one per ad) | 1 Flight + 1 Ad + 1 Creative | 1 Flight + 1 Ad + 1 Creative (orchestrated by gem helper) |
| `KevelAd.site_urls` | siteIds via URL lookup | (implicit per-project; sites scoped to the Project) |
| Provider published / Org archived (post-filter) | `blockedCreatives` | `block.creativeIds` |
| `AdConfiguration.priority_id` | Priority | Priority |
| `AdConfiguration.ad_type_id` | AdType | AdType |
| `AdConfiguration.zone_id` | Zone | Zone |
| `AdConfiguration.creative_template_id` | CreativeTemplate | CreativeTemplate |
| `current_organization.host` | site lookup → `siteId` | project routing in caller; optional `siteUrl` shorthand |

## Data Model Additions on the RX Side

Add to existing tables:

- `organizations.knievel_project_id` (string, nullable until backfilled).
- `providers.knievel_advertiser_id` (string) — analogous to existing
  `kevel_advertiser_id`; live in parallel during rollout.
- `kevel_ads.knievel_ad_id`, `knievel_flight_id`, `knievel_creative_id`,
  `knievel_campaign_id` — analogous to the existing `kevel_*` columns;
  live in parallel.

These columns mirror today's `kevel_*` shape so the rollout flag can
flip per-marketplace without losing the old IDs.

(Optional: rename later, once Kevel is decommissioned.)

## Sync Job Changes

`Kevel::SyncKevelRecordsFromAdJob` becomes
`Knievel::SyncKnievelRecordsFromAdJob`. Same trigger
(`KevelAd after_save`), same orchestration shape, different client.

### 1. Project provisioning (new step)

When an RX Organization is created — or first seen by the new sync —
upsert the Project:

```ruby
project = client.org("scientist-com-#{Rails.env}").projects.upsert(
  external_id: "rx_org:#{rx_org.id}",
  name:        rx_org.name
)
rx_org.update!(knievel_project_id: project.id)
```

Idempotent on `external_id`; safe to call repeatedly.

### 2. Default Site provisioning

Each RX Organization typically needs one Site representing the
marketplace itself, plus its zones. Create on first sync into a
Project:

```ruby
project_client = client.project(rx_org.knievel_project_id)
site = project_client.sites.upsert_by_url(
  url:  "https://#{rx_org.host}",
  name: rx_org.name
)
```

If the Organization has multiple hostnames, pass them via `aliases:` on
the upsert.

### 3. Provider → Advertiser

```ruby
advertiser = project_client.advertisers.upsert(
  external_id: "provider:#{provider.id}",
  name:        provider.name
)
provider.update!(knievel_advertiser_id: advertiser.id)
```

### 4. Campaign

```ruby
campaign = project_client.campaigns.upsert(
  external_id:   "advertiser:#{advertiser.id}:campaign:#{ad_config.campaign_name}",
  advertiser_id: advertiser.id,
  name:          ad_config.campaign_name
)
```

### 5. Flight + Ad + Creative

Today: three sequential round-trips. The Ruby gem ships a hand-rolled
helper that does all three in one call (no new wire endpoint — it just
orchestrates standard upserts):

```ruby
result = project_client.ads.upsert_with_flight_and_creative(
  external_id:   "kevel_ad:#{kevel_ad.id}",
  advertiser_id: advertiser.id,
  campaign_id:   campaign.id,
  flight: {
    external_id: "kevel_ad:#{kevel_ad.id}:flight",
    site_ids:    [site.id],            # the project's default site
    zone_ids:    [ad_config.zone_id],
    ad_types:    [ad_config.ad_type_id],
    priority_id: ad_config.priority_id,
    start_date:  kevel_ad.starts_at,
    end_date:    kevel_ad.ends_at
  },
  creative: {
    external_id:  "kevel_ad:#{kevel_ad.id}:creative",
    type:         :native,
    template_id:  ad_config.creative_template_id,
    values:       kevel_ad.dynamic_values
  },
  weight: 100
)

kevel_ad.update!(
  knievel_ad_id:       result.ad.id,
  knievel_flight_id:   result.flight.id,
  knievel_creative_id: result.creative.id,
  knievel_campaign_id: campaign.id
)
```

### 6. Multi-marketplace ads

The existing `type_option` enum has `subscription` (active),
`per_marketplace` (not wired), and `all_marketplaces` (not wired).

- `subscription` stays single-marketplace: one Project per RX
  Organization, sync runs in that Project only.
- When `all_marketplaces` is wired, sync iterates the org's Projects
  and upserts the same ad into each. Until then, no change.

## Decision Call Changes

`AdDecisionRequestsController` swaps `Kevel::Decision` for the gem:

```ruby
# Before
Kevel::Decision.new(
  network_id:        ENV["KEVEL_NETWORK_ID"],
  site_id:           Kevel::Site.find_by(url: "https://#{current_organization.host}").id,
  ad_types:          [...],
  zone_ids:          [...],
  blocked_creatives: blocked_creative_ids
).call

# After
client = Knievel::Client.new(token: ENV["KNIEVEL_ORG_TOKEN"])
client.project(current_organization.knievel_project_id).decisions.create(
  context: {
    url:        request.url,
    referrer:   request.referer,
    user_agent: request.user_agent
  },
  placements: [{
    id:       "main",
    site_url: "https://#{current_organization.host}",  # resolved server-side
    zone_ids: [...],
    ad_types: [...],
    count:    1
  }],
  block: { creative_ids: blocked_creative_ids }
)
```

The `blocked_creative_ids` computation
(`AdDecisionRequestsController#blocked_creative_ids` — unpublished
providers + archived organizations) **stays exactly as today**. It's
RX state, knievel doesn't model it.

## Configuration

| Old (Kevel) | New (Knievel) |
|---|---|
| `KEVEL_API_KEY` | `KNIEVEL_ORG_TOKEN` |
| `KEVEL_NETWORK_ID` | (replaced by per-call `projectId`) |
| `https://e-{network}.adzerk.net/api/v2` | `KNIEVEL_BASE_URL` (e.g. `https://ads.scientist.com`) |
| (none) | `KNIEVEL_ORG_EXTERNAL_ID` (e.g. `scientist-com-prod`) |

## Rollout Strategy

Phased per-marketplace. Both clients (`Kevel::*` and `Knievel::*`)
coexist throughout.

1. **Stand up knievel in staging.** Provision Org
   `scientist-com-staging` and one pilot Project (a small staging
   marketplace).
2. **Sync writes go to both** during rollout. Add a feature flag
   `dual_write_knievel: true` to the sync job; it runs the existing
   Kevel sync and the new knievel sync in series. Errors on the knievel
   side log but don't fail the job.
3. **Backfill.** One-time job walks existing `KevelAd`s for the pilot
   marketplace and runs the knievel sync. Verify decisions match
   (golden-file diff against Kevel responses for a sample of placements).
4. **Per-marketplace decision flip.** Feature flag
   `use_knievel_for_decisions` on RX Organization. Flip for the pilot
   marketplace first; monitor latency, error rate, fill rate; expand.
5. **Production cutover.** Provision Org `scientist-com-prod`, run
   dual-write across all marketplaces, backfill, then flip decision
   flags marketplace-by-marketplace.
6. **Decommission Kevel.** Once all marketplaces are stable on knievel:
   stop dual-write, remove `Kevel::*` code, drop the `kevel_*` columns
   in a follow-up.

Rollback at any stage: flip the decision flag back. Sync continues to
both as long as dual-write is enabled.

## Call-Site Inventory (PR-Sized Chunks)

Approximate one PR per item:

1. Add `knievel_*` columns to `organizations`, `providers`, `kevel_ads`.
2. Vendor / install the `knievel-ruby` gem.
3. Implement `Knievel::SyncKnievelRecordsFromAdJob` (mirrors existing
   sync, no flag wiring yet).
4. Wire `dual_write_knievel` flag in the existing sync job.
5. Implement `Knievel::Decision` shim in `app/services/knievel/`.
6. Add `use_knievel_for_decisions` flag check in
   `AdDecisionRequestsController`.
7. Backfill rake task: walk existing `KevelAd`s, run sync, verify ID
   columns populated.
8. Backoffice updates: provider-facing screens read knievel IDs once
   populated, fall back to Kevel.
9. Per-environment ENV / secrets rollout.
10. Per-marketplace flag flip (one or more PRs as rollout progresses).
11. Decommission: remove `Kevel::*`, drop columns.

## What Doesn't Move to Knievel

These stay in RX because they're product/business logic, not ad-server
concerns:

- `KevelAd.type_option` enum — RX product surface.
- `Kevel::SubscriptionAdValidator` — RX business rule (per-provider
  marketplace caps).
- Provider/Organization scoping rules — RX auth model.
- `closest_advertisable_organization` fallback — RX-specific routing.
- The `blocked_creative_ids` computation — RX state (publish status,
  archival).

Knievel exposes the primitives (`block.creativeIds`, etc.) that let RX
keep all of the above on the RX side without knievel needing to know
about Providers, Organizations, subscriptions, or archival.
