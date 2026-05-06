//! Migration linter (`REQUIREMENTS.md` § 7.1.1 gate (2)).
//!
//! Walks `.sql` files in the configured directory and rejects:
//!
//! 1. `ALTER TABLE ... DISABLE ROW LEVEL SECURITY`
//! 2. `ALTER TABLE ... NO FORCE ROW LEVEL SECURITY`
//! 3. `CREATE TABLE` in the `knievel` schema without a paired
//!    `ALTER TABLE ... ENABLE ROW LEVEL SECURITY` in the same file.
//! 4. `CREATE POLICY` whose `USING` clause does not reference
//!    `current_setting('knievel.project_id')` (the documented
//!    session-scoped tenant binding).
//!
//! Tests in `xtask/tests/fixtures/migrations/` per `TESTING.md` § 10.1.

use anyhow::{anyhow, Result};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(path: PathBuf) -> Result<()> {
    let files = collect_sql_files(&path)?;
    let mut violations = Vec::new();
    for file in &files {
        let content = fs::read_to_string(file)?;
        violations.extend(lint_file(file, &content));
    }
    if violations.is_empty() {
        println!(
            "xtask lint-migrations: {} file(s) clean in {}",
            files.len(),
            path.display()
        );
        Ok(())
    } else {
        for v in &violations {
            eprintln!("{v}");
        }
        Err(anyhow!(
            "{} migration linting violation(s)",
            violations.len()
        ))
    }
}

/// Strip SQL line comments (`-- ...`) and block comments (`/* ... */`).
/// Naive: a string literal containing `--` would be partially eaten,
/// but migrations don't put SQL keywords in user-data string literals.
fn strip_sql_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            // Skip to end of line, preserving the newline so line
            // numbers remain stable for any future error reporting.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn collect_sql_files(path: &Path) -> Result<Vec<PathBuf>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(path)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "sql") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

pub(crate) fn lint_file(file: &Path, content: &str) -> Vec<String> {
    let mut v = Vec::new();
    let f = file.display();
    // Strip SQL comments first; otherwise prose like
    // "the linter rejects CREATE TABLE in knievel" inside a `--`
    // comment trips rule 3 with a phantom table named `in`.
    let content = strip_sql_comments(content);
    let content = content.as_str();

    // Rule 1: DISABLE ROW LEVEL SECURITY rejected unconditionally.
    let disable = Regex::new(r"(?i)disable\s+row\s+level\s+security").unwrap();
    if disable.is_match(content) {
        v.push(format!(
            "{f}: rule 1 — DISABLE ROW LEVEL SECURITY is rejected"
        ));
    }

    // Rule 2: NO FORCE ROW LEVEL SECURITY rejected unconditionally.
    let no_force = Regex::new(r"(?i)no\s+force\s+row\s+level\s+security").unwrap();
    if no_force.is_match(content) {
        v.push(format!(
            "{f}: rule 2 — NO FORCE ROW LEVEL SECURITY is rejected"
        ));
    }

    // Rule 3: every CREATE TABLE in the knievel schema must have a
    // paired ENABLE ROW LEVEL SECURITY in the same file.
    let in_knievel_searchpath = Regex::new(r"(?i)set\s+search_path\s+to\s+knievel\b")
        .unwrap()
        .is_match(content);

    let create_table =
        Regex::new(r"(?i)create\s+table(?:\s+if\s+not\s+exists)?\s+(?:(\w+)\.)?(\w+)").unwrap();
    let enable_rls =
        Regex::new(r"(?i)alter\s+table\s+(?:(\w+)\.)?(\w+)\s+enable\s+row\s+level\s+security")
            .unwrap();

    let mut tables_in_knievel: Vec<String> = Vec::new();
    for cap in create_table.captures_iter(content) {
        let schema = cap.get(1).map(|m| m.as_str().to_lowercase());
        let name = cap.get(2).unwrap().as_str().to_lowercase();
        let is_knievel = match schema.as_deref() {
            Some("knievel") => true,
            None => in_knievel_searchpath,
            _ => false,
        };
        if is_knievel {
            tables_in_knievel.push(name);
        }
    }

    let mut tables_with_rls: HashSet<String> = HashSet::new();
    for cap in enable_rls.captures_iter(content) {
        let name = cap.get(2).unwrap().as_str().to_lowercase();
        tables_with_rls.insert(name);
    }

    for t in &tables_in_knievel {
        if !tables_with_rls.contains(t) {
            v.push(format!(
                "{f}: rule 3 — CREATE TABLE knievel.{t} without ENABLE ROW LEVEL SECURITY"
            ));
        }
    }

    // Rule 4: every CREATE POLICY's USING clause must reference
    // the tenant binding `knievel.project_id`. Match non-greedy
    // through to a balancing `)` keeping the body of USING(...).
    let policy = Regex::new(r"(?is)create\s+policy[^;]*?\busing\s*\(([^;]*?)\)").unwrap();
    for cap in policy.captures_iter(content) {
        let using = cap.get(1).unwrap().as_str();
        if !using.to_lowercase().contains("knievel.project_id") {
            v.push(format!(
                "{f}: rule 4 — CREATE POLICY USING does not reference current_setting('knievel.project_id')"
            ));
        }
    }

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migrations")
            .join(name);
        fs::read_to_string(&p).unwrap_or_else(|e| panic!("{} : {e}", p.display()))
    }

    fn lint(name: &str) -> Vec<String> {
        lint_file(Path::new(name), &read_fixture(name))
    }

    #[test]
    fn fixture_01_disable_rls_rejected() {
        let v = lint("01_disable_rls.sql");
        assert!(v.iter().any(|s| s.contains("rule 1")), "{v:?}");
    }

    #[test]
    fn fixture_02_no_force_rls_rejected() {
        let v = lint("02_no_force_rls.sql");
        assert!(v.iter().any(|s| s.contains("rule 2")), "{v:?}");
    }

    #[test]
    fn fixture_03_table_without_rls_rejected() {
        let v = lint("03_table_without_rls.sql");
        assert!(v.iter().any(|s| s.contains("rule 3")), "{v:?}");
    }

    #[test]
    fn fixture_04_policy_without_tenant_rejected() {
        let v = lint("04_policy_without_tenant.sql");
        assert!(v.iter().any(|s| s.contains("rule 4")), "{v:?}");
    }

    #[test]
    fn fixture_05_table_outside_knievel_accepted() {
        let v = lint("05_table_outside_knievel.sql");
        assert!(v.is_empty(), "expected clean, got {v:?}");
    }

    #[test]
    fn fixture_06_clean_table_accepted() {
        let v = lint("06_clean_table.sql");
        assert!(v.is_empty(), "expected clean, got {v:?}");
    }

    #[test]
    fn real_migrations_are_clean() {
        // Sanity: `migrations/0001_init.sql` (Phase 1.6) lints clean.
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("migrations/0001_init.sql");
        let content = fs::read_to_string(&p).unwrap();
        let v = lint_file(&p, &content);
        assert!(v.is_empty(), "0001_init.sql should be clean: {v:?}");
    }
}
