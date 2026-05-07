//! OpenAPI tag enum used by every `#[OpenApi]` impl across the
//! resource modules. Each variant becomes a tag in the emitted
//! spec; the openapi-generator (and language clients downstream)
//! shard operations into one API class per tag — without this,
//! every operation collapses into a single `DefaultApi` in the
//! generated Ruby gem (and equivalents elsewhere).
//!
//! Variant names are chosen so the generated Ruby classes read
//! idiomatically (`Knievel::AdvertisersApi`,
//! `Knievel::CreativeTemplatesApi`, etc.). Doc comments on each
//! variant flow through to the tag's `description` in the spec.
//!
//! See `PHASES.md` Phase 4.10 follow-up for context.

use poem_openapi::Tags;

#[derive(Tags)]
pub enum ApiTags {
    /// Liveness, readiness, and build/version metadata.
    System,
    /// Auth handshake — `/v1/whoami` and friends. Validates a
    /// bearer (opaque or JWT) and echoes the principal back so
    /// clients can confirm the credential before proceeding.
    Auth,
    /// Org and project lifecycle (single resource module covers
    /// both since project lookups are scoped through the org).
    Orgs,
    /// API token mint, list, and revoke.
    Tokens,
    /// Cross-project ad library (read-only inventory view).
    AdLibrary,
    /// Advertiser CRUD + `:batchUpsert`.
    Advertisers,
    /// Campaign CRUD + `:batchUpsert`.
    Campaigns,
    /// Flight CRUD + `:batchUpsert` (pacing, scheduling, demand).
    Flights,
    /// Ad CRUD + `:batchUpsert`.
    Ads,
    /// Creative CRUD + `:batchUpsert`.
    Creatives,
    /// Creative template CRUD + JSON-schema validation surface.
    CreativeTemplates,
    /// Site CRUD + `:batchUpsert`.
    Sites,
    /// Zone CRUD + `:batchUpsert`.
    Zones,
    /// Taxonomy categories + project-scoped seeding.
    Taxonomy,
    /// Hot-path decisioning endpoints.
    Decisions,
    /// Decision-explanation / preview endpoints.
    Explain,
}
