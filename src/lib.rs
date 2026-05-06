//! `knievel` library crate.
//!
//! Exposes the modules the binary uses so out-of-tree tooling
//! (`xtask`) can build the OpenAPI spec without spawning a server.
//! Carries the same `#![allow(dead_code)]` budget as the binary
//! while Phase 2 is still wiring consumers.

#![allow(dead_code)]

pub mod ads;
pub mod advertisers;
pub mod auth;
pub mod batch;
pub mod campaigns;
pub mod config;
pub mod creative_templates;
pub mod creatives;
pub mod db;
pub mod decisions;
pub mod flights;
pub mod handlers;
pub mod hmac;
pub mod idempotency;
pub mod observability;
pub mod orgs;
pub mod selection;
pub mod server;
pub mod sites;
pub mod snapshot;
pub mod state;
pub mod system;
pub mod taxonomy;
pub mod tokens;
pub mod zones;

/// Generate the OpenAPI spec as YAML. Used by
/// `cargo xtask openapi` to write `openapi.yaml` and by
/// `cargo xtask openapi --check` to fail on drift
/// (`TESTING.md` § 6.3, § 12.7).
pub fn openapi_spec_yaml() -> String {
    use poem_openapi::OpenApiService;
    let svc = OpenApiService::new(
        (
            system::SystemApi,
            orgs::OrgApi,
            tokens::TokensApi,
            advertisers::AdvertisersApi,
            campaigns::CampaignsApi,
            flights::FlightsApi,
            creatives::CreativesApi,
            creative_templates::CreativeTemplatesApi,
            ads::AdsApi,
            sites::SitesApi,
            zones::ZonesApi,
            taxonomy::TaxonomyApi,
            decisions::DecisionsApi,
        ),
        "knievel",
        env!("CARGO_PKG_VERSION"),
    );
    svc.spec_yaml()
}
