//! Test-file naming gate — implemented later in Phase 3.
//!
//! Verifies every `tests/*.rs` file maps cleanly to one of the
//! nextest filter slices in `TESTING.md` § 12.5
//! (`unit`, `integration`, `api`, `acceptance`).

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("xtask test-shape: stub (refines as test slices grow)");
    Ok(())
}
