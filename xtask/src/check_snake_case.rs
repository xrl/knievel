//! Hard-rule wire-format gate: every JSON property name and
//! every operation parameter name in `openapi.yaml` is
//! `snake_case`.
//!
//! Rationale: the v0 spec is JSON+snake_case. The convention is
//! load-bearing — generated client codegen mostly handles the
//! mapping, but every doc example, every test fixture, and every
//! consumer's manual JSON construction has to agree. Mixed-case
//! drift is observable in code review but easy to miss; this gate
//! makes it loud.
//!
//! What we check:
//!
//! - For every component schema's `properties:` map, every key.
//! - For every `paths.*.*.parameters[].name`, every name.
//! - For every nested `properties:` (request/response inline
//!   schemas), every key.
//!
//! What we ignore:
//!
//! - OpenAPI structural keys (`operationId`, `requestBody`,
//!   `additionalProperties`, etc.) — those are spec metadata
//!   under `paths:` / `components:` / etc. and follow OpenAPI's
//!   own conventions, not knievel's wire format.
//! - Schema names (`AdvertiserList`) — those are PascalCase by
//!   generator convention.
//! - Path-template parameters (`{project_id}` etc.) — these flow
//!   through to URL paths, not JSON bodies; they happen to also
//!   be snake_case but the rule is incidental.

use std::fs;

use anyhow::{anyhow, Context, Result};
use serde_yaml::Value;

const SPEC_PATH: &str = "openapi.yaml";

pub fn run() -> Result<()> {
    let raw = fs::read_to_string(SPEC_PATH).with_context(|| format!("reading {SPEC_PATH}"))?;
    let spec: Value = serde_yaml::from_str(&raw)?;

    let mut violations: Vec<String> = Vec::new();
    let mut props_checked = 0_usize;
    let mut params_checked = 0_usize;

    // Component schemas — recurse into nested `properties:` so
    // request/response object types nested under `oneOf` etc.
    // also get covered.
    if let Some(schemas) = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_mapping)
    {
        for (schema_name, schema) in schemas {
            let schema_name_s = schema_name.as_str().unwrap_or("?");
            walk_properties(schema, schema_name_s, &mut violations, &mut props_checked);
        }
    }

    // Operation parameters — `parameters[].name` on every path /
    // operation.
    if let Some(paths) = spec.get("paths").and_then(Value::as_mapping) {
        for (path_key, path_item) in paths {
            let path_s = path_key.as_str().unwrap_or("?");
            let Some(operations) = path_item.as_mapping() else {
                continue;
            };
            for (method, op) in operations {
                let method_s = method.as_str().unwrap_or("?");
                let Some(params) = op.get("parameters").and_then(Value::as_sequence) else {
                    continue;
                };
                for param in params {
                    let in_kind = param.get("in").and_then(Value::as_str).unwrap_or("");
                    if in_kind == "path" {
                        // Path-template params surface in URLs;
                        // they flow through `{project_id}`-shaped
                        // OpenAPI paths and aren't user-authored
                        // JSON. We still want them snake_case for
                        // ergonomics but the rule is enforced via
                        // the path-level `{snake_case}` shape, not
                        // here.
                        continue;
                    }
                    if in_kind == "header" {
                        // HTTP headers are Title-Case-Hyphenated
                        // by RFC convention (`Idempotency-Key`,
                        // `If-Match`, `Authorization`). The
                        // snake_case rule is the JSON-body wire
                        // format; HTTP headers obey HTTP
                        // conventions instead.
                        continue;
                    }
                    let Some(name) = param.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    params_checked += 1;
                    if !is_snake_case(name) {
                        violations.push(format!(
                            "  {path_s} {method_s}: parameter `{name}` is not snake_case"
                        ));
                    }
                }
            }
        }
    }

    if violations.is_empty() {
        println!(
            "xtask check-snake-case: {props_checked} property name(s), {params_checked} parameter name(s), all snake_case"
        );
        Ok(())
    } else {
        for v in &violations {
            eprintln!("{v}");
        }
        Err(anyhow!(
            "{} snake_case violation(s) — JSON wire format is snake_case across the spec",
            violations.len()
        ))
    }
}

/// `snake_case` per the wire-format rule:
/// - lowercase ASCII letters + digits + `_`
/// - doesn't start or end with `_`
/// - no consecutive `__`
fn is_snake_case(name: &str) -> bool {
    if name.is_empty() || name.starts_with('_') || name.ends_with('_') || name.contains("__") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Recursively walk a schema, finding every `properties:` map and
/// asserting each key is snake_case. Also descends into `items:`,
/// `oneOf:`, `anyOf:`, `allOf:`, and nested objects to catch
/// inline-defined sub-schemas.
fn walk_properties(node: &Value, where_: &str, violations: &mut Vec<String>, counter: &mut usize) {
    if let Some(props) = node.get("properties").and_then(Value::as_mapping) {
        for (key, sub_schema) in props {
            let Some(name) = key.as_str() else { continue };
            *counter += 1;
            if !is_snake_case(name) {
                violations.push(format!("  {where_}: property `{name}` is not snake_case"));
            }
            walk_properties(sub_schema, &format!("{where_}.{name}"), violations, counter);
        }
    }
    if let Some(items) = node.get("items") {
        walk_properties(items, &format!("{where_}[]"), violations, counter);
    }
    for combinator in ["oneOf", "anyOf", "allOf"] {
        if let Some(seq) = node.get(combinator).and_then(Value::as_sequence) {
            for (i, sub) in seq.iter().enumerate() {
                walk_properties(
                    sub,
                    &format!("{where_}.{combinator}[{i}]"),
                    violations,
                    counter,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_correctly() {
        assert!(is_snake_case("foo"));
        assert!(is_snake_case("foo_bar"));
        assert!(is_snake_case("foo_bar_baz_42"));
        assert!(is_snake_case("a"));
        assert!(!is_snake_case(""));
        assert!(!is_snake_case("Foo"));
        assert!(!is_snake_case("fooBar"));
        assert!(!is_snake_case("FOO"));
        assert!(!is_snake_case("foo-bar"));
        assert!(!is_snake_case("_foo"));
        assert!(!is_snake_case("foo_"));
        assert!(!is_snake_case("foo__bar"));
    }
}
