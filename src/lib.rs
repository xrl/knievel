//! `knievel` library crate.
//!
//! Exposes the modules the binary uses so out-of-tree tooling
//! (`xtask`) can build the OpenAPI spec without spawning a server.
//! Carries the same `#![allow(dead_code)]` budget as the binary
//! while Phase 2 is still wiring consumers.

#![allow(dead_code)]

pub mod advertisers;
pub mod auth;
pub mod campaigns;
pub mod config;
pub mod creative_templates;
pub mod creatives;
pub mod db;
pub mod flights;
pub mod handlers;
pub mod idempotency;
pub mod observability;
pub mod orgs;
pub mod server;
pub mod state;
pub mod system;
pub mod tokens;

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
        ),
        "knievel",
        env!("CARGO_PKG_VERSION"),
    );
    svc.spec_yaml()
}
