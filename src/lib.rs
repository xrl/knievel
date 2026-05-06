//! `knievel` library crate.
//!
//! Exposes the modules the binary uses so out-of-tree tooling
//! (`xtask`) can build the OpenAPI spec without spawning a server.
//! Carries the same `#![allow(dead_code)]` budget as the binary
//! while Phase 2 is still wiring consumers.

#![allow(dead_code)]

pub mod config;
pub mod observability;
pub mod server;
pub mod state;
pub mod system;

/// Generate the OpenAPI spec as YAML. Used by
/// `cargo xtask openapi` to write `openapi.yaml` and by
/// `cargo xtask openapi --check` to fail on drift
/// (`TESTING.md` § 6.3, § 12.7).
pub fn openapi_spec_yaml() -> String {
    use poem_openapi::OpenApiService;
    let svc = OpenApiService::new(system::SystemApi, "knievel", env!("CARGO_PKG_VERSION"));
    svc.spec_yaml()
}
