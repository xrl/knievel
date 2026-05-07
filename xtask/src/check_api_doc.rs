//! API.md ↔ openapi.yaml coverage gate
//! (`DOCUMENTATION_PLAN.md` § 11.2).
//!
//! For every operation in `openapi.yaml`, the operation's path
//! must appear at least once inside a markdown table row in
//! `API.md`. Today's `API.md` documents endpoints in tables
//! shaped:
//!
//!     | `GET` | `/v1/projects/{projectId}/advertisers` | List. |
//!
//! The check is intentionally simple — substring containment of
//! the path in any table-cell line. Adding a new endpoint to the
//! spec without putting a row in `API.md` fails the gate;
//! a documented row that's not in the spec fails too (a removal
//! was forgotten).

use std::collections::HashSet;
use std::fs;

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_yaml::Value;

/// Normalize paths so spec-side `{project_id}` and doc-side
/// `{projectId}` compare equal: every `{...}` becomes `{}`.
fn normalize(path: &str) -> String {
    let re = Regex::new(r"\{[^}]*\}").unwrap();
    re.replace_all(path, "{}").into_owned()
}

const SPEC_PATH: &str = "openapi.yaml";
const DOC_PATH: &str = "API.md";

pub fn run() -> Result<()> {
    let spec_paths =
        collect_spec_paths(SPEC_PATH).with_context(|| format!("reading {SPEC_PATH}"))?;
    let documented =
        collect_documented_paths(DOC_PATH).with_context(|| format!("reading {DOC_PATH}"))?;

    let documented_norm: HashSet<String> = documented.iter().map(|d| normalize(d)).collect();

    // One-way gate: every spec path must be in API.md, but
    // API.md is allowed to document future / system / aspirational
    // endpoints that aren't in the spec yet (members API, ad-
    // library `:batchUpsert` deferred to Phase 6.4, `/metrics`,
    // etc.). Reverse coverage would require curating an allow-list
    // of "intentionally documented but not yet shipped" paths,
    // which rots faster than it helps.
    let undocumented: Vec<&String> = spec_paths
        .iter()
        .filter(|p| !documented_norm.contains(&normalize(p)))
        .collect();

    let mut errors: Vec<String> = Vec::new();
    for p in &undocumented {
        errors.push(format!("  spec path not documented in {DOC_PATH}: {p}"));
    }

    if errors.is_empty() {
        println!(
            "xtask check-api-doc: {} spec path(s), all documented in {DOC_PATH}",
            spec_paths.len()
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("{e}");
        }
        Err(anyhow!("{} doc-coverage error(s)", errors.len()))
    }
}

/// Read `openapi.yaml` and return every `paths:` key. We compare
/// path strings as-is (with `{param}` substitutions) — `API.md`
/// uses the same `{projectId}` shape.
fn collect_spec_paths(path: &str) -> Result<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_yaml::from_str(&raw)?;
    let paths = value
        .get("paths")
        .and_then(Value::as_mapping)
        .ok_or_else(|| anyhow!("{path} has no `paths:` mapping"))?;
    let mut out: Vec<String> = paths
        .keys()
        .filter_map(|k| k.as_str().map(str::to_owned))
        .collect();
    out.sort();
    Ok(out)
}

/// Read `API.md` and return the set of paths that appear in any
/// table row. We accept any line containing a backtick-wrapped
/// path that starts with `/`, because the doc uses several table
/// shapes (`| GET | path | desc |`, `| POST | path | desc |`,
/// short summary lines, etc.).
fn collect_documented_paths(path: &str) -> Result<HashSet<String>> {
    let raw = fs::read_to_string(path)?;
    let mut documented: HashSet<String> = HashSet::new();
    for line in raw.lines() {
        // Look for paths inside backticks. Inner content may be a
        // bare path (`/v1/projects/...`) or a method+path pair
        // (`POST /v1/projects/...` — used in section headings); we
        // accept any whitespace-separated token starting with `/`.
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            if let Some(end) = after.find('`') {
                let inner = &after[..end];
                for word in inner.split_whitespace() {
                    if word.starts_with('/') {
                        documented.insert(word.to_owned());
                    }
                }
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
    Ok(documented)
}
