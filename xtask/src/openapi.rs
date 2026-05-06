//! OpenAPI spec generation + drift check — real impl in Phase 2.8.
//!
//! `--check` mode: regenerate the spec from the binary, diff against
//! the committed `openapi.yaml`, fail on mismatch.
//!
//! Without `--check`: write the regenerated spec to `openapi.yaml`.
//!
//! Spec ref: `TESTING.md` § 6.3, § 12.7.

use anyhow::Result;

pub fn run(check: bool) -> Result<()> {
    if check {
        println!("xtask openapi --check: stub (Phase 2.8 will implement)");
    } else {
        println!("xtask openapi: stub (Phase 2.8 will implement)");
    }
    Ok(())
}
