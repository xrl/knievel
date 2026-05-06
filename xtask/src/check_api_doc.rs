//! API.md ↔ OpenAPI coverage gate — real impl in Phase 5.6.
//!
//! For every endpoint in the OpenAPI spec, asserts a matching row
//! exists in `API.md`'s resource tables.
//!
//! Spec ref: `DOCUMENTATION_PLAN.md` § 11.2.

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("xtask check-api-doc: stub (Phase 5.6 will implement)");
    Ok(())
}
