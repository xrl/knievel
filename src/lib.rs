//! `knievel` library crate.
//!
//! Exposes the modules the binary uses so out-of-tree tooling
//! (`xtask`) can build the OpenAPI spec without spawning a server.
//! Carries the same `#![allow(dead_code)]` budget as the binary
//! while Phase 2 is still wiring consumers.

#![allow(dead_code)]

pub mod ad_library;
pub mod admin_ui;
pub mod ads;
pub mod advertisers;
pub mod api_tags;
pub mod audit;
pub mod auth;
pub mod batch;
pub mod campaigns;
pub mod cli;
pub mod config;
pub mod creative_templates;
pub mod creatives;
pub mod db;
pub mod decisions;
pub mod etag;
pub mod event_endpoints;
pub mod events;
pub mod flights;
pub mod handlers;
pub mod hmac;
pub mod idempotency;
pub mod image_upload;
pub mod leader;
pub mod migrate;
pub mod observability;
pub mod orgs;
pub mod pagination;
pub mod partitions;
pub mod request_log;
pub mod rollup;
pub mod selection;
pub mod server;
pub mod sites;
pub mod snapshot;
pub mod sql;
pub mod state;
pub mod system;
pub mod taxonomy;
pub mod tokens;
pub mod whoami;
pub mod zones;

/// Default `servers:` entry stamped into the static
/// `openapi.yaml` (and the bootstrap value of the live
/// `/openapi.json` until the runtime override lands). Generated
/// clients use this as their default base URL — production
/// callers are expected to override
/// (`Knievel::Configuration.host` in the Ruby gem and
/// equivalents elsewhere).
pub const DEFAULT_OPENAPI_SERVER_URL: &str = "http://localhost:8080";
pub const DEFAULT_OPENAPI_SERVER_DESCRIPTION: &str =
    "Local development default; override via your client's host configuration for production.";

/// Generate the OpenAPI spec as YAML. Used by
/// `cargo xtask openapi` to write `openapi.yaml` and by
/// `cargo xtask openapi --check` to fail on drift
/// (`TESTING.md` § 6.3, § 12.7).
pub fn openapi_spec_yaml() -> String {
    use poem_openapi::{OpenApiService, ServerObject};
    let svc = OpenApiService::new(
        (
            system::SystemApi,
            whoami::WhoamiApi,
            orgs::OrgApi,
            tokens::TokensApi,
            ad_library::AdLibraryApi,
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
            decisions::ExplainApi,
        ),
        "knievel",
        env!("CARGO_PKG_VERSION"),
    )
    .server(
        ServerObject::new(DEFAULT_OPENAPI_SERVER_URL)
            .description(DEFAULT_OPENAPI_SERVER_DESCRIPTION),
    );
    svc.spec_yaml()
}
