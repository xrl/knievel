//! Cross-tenant endpoint coverage gate — real impl in Phase 1.8.
//!
//! Walks the OpenAPI spec, lists every `/v1/projects/{p}/...`
//! operation, and fails if any is missing a paired entry in
//! `tests/cross_tenant_manifest.toml`.
//!
//! Spec ref: `TESTING.md` § 6.5, `REQUIREMENTS.md` § 7.1.1 gate (1).

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("xtask check-cross-tenant: stub (Phase 1.8 will implement)");
    Ok(())
}
