//! Migration linter — real implementation lands in Phase 1.7.
//!
//! Rules per `REQUIREMENTS.md` § 7.1.1 gate (2):
//! - reject `ALTER TABLE ... DISABLE ROW LEVEL SECURITY`
//! - reject `ALTER TABLE ... NO FORCE ROW LEVEL SECURITY`
//! - reject `CREATE TABLE` in `knievel` without a paired
//!   `ALTER TABLE ... ENABLE ROW LEVEL SECURITY`
//! - reject `CREATE POLICY` whose `USING` clause doesn't reference
//!   `current_setting('knievel.project_id')`

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("xtask lint-migrations: stub (Phase 1.7 will implement)");
    Ok(())
}
