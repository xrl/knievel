//! Doc fenced-code-block syntax check — real impl in Phase 5.6.
//!
//! Walks every `.md` file, extracts each fenced code block by
//! language tag, parses with the matching parser
//! (`syn` for rust, `serde_yaml` for yaml, `serde_json` for json,
//! `pg_query` for sql). Blocks tagged `lang,ignore` skipped.
//!
//! Spec ref: `DOCUMENTATION_PLAN.md` § 11.2.

use anyhow::Result;

pub fn run() -> Result<()> {
    println!("xtask check-doc-fences: stub (Phase 5.6 will implement)");
    Ok(())
}
