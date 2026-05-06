//! Cross-tenant endpoint coverage gate
//! (`REQUIREMENTS.md` § 7.1.1 gate (1), `TESTING.md` § 6.5).
//!
//! Walks the OpenAPI spec, lists every `/v1/projects/{p}/...`
//! operation, and fails if any operation lacks an entry in
//! `tests/cross_tenant_manifest.toml` proving a paired negative
//! test exists.
//!
//! Until `openapi.yaml` lands (Phase 2.8), the gate runs in
//! "skipped" mode — prints an info line and exits 0. Once the spec
//! exists and Phase 3 starts adding endpoints, every new
//! `/v1/projects/{p}/...` operation must be registered before merge.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const SPEC_PATH: &str = "openapi.yaml";
const MANIFEST_PATH: &str = "tests/cross_tenant_manifest.toml";

#[derive(Deserialize, Default)]
struct Manifest {
    #[serde(default, rename = "entry")]
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    path: String,
    method: String,
    /// Test name documented in the entry — diagnostic value only;
    /// the gate doesn't run it. Tests run via `cargo nextest`.
    #[allow(dead_code)]
    test: String,
}

pub fn run() -> Result<()> {
    let spec_path = Path::new(SPEC_PATH);
    if !spec_path.exists() {
        println!(
            "xtask check-cross-tenant: {SPEC_PATH} does not exist yet \
             (Phase 2.8 will land it); skipping."
        );
        return Ok(());
    }

    let raw = fs::read_to_string(spec_path).with_context(|| format!("reading {SPEC_PATH}"))?;
    let spec: serde_yaml::Value =
        serde_yaml::from_str(&raw).with_context(|| format!("parsing {SPEC_PATH} as YAML"))?;

    let project_scoped = collect_project_scoped_endpoints(&spec);

    let manifest: Manifest = if Path::new(MANIFEST_PATH).exists() {
        let raw = fs::read_to_string(MANIFEST_PATH)
            .with_context(|| format!("reading {MANIFEST_PATH}"))?;
        toml::from_str(&raw).with_context(|| format!("parsing {MANIFEST_PATH}"))?
    } else {
        Manifest::default()
    };

    let registered: HashSet<(String, String)> = manifest
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.method.to_uppercase()))
        .collect();

    let mut missing = Vec::new();
    for (path, method) in &project_scoped {
        if !registered.contains(&(path.clone(), method.clone())) {
            missing.push(format!("  {method} {path}"));
        }
    }

    if missing.is_empty() {
        println!(
            "xtask check-cross-tenant: {} project-scoped endpoint(s), all covered",
            project_scoped.len()
        );
        Ok(())
    } else {
        eprintln!("Project-scoped endpoints missing a cross-tenant test:");
        for m in &missing {
            eprintln!("{m}");
        }
        eprintln!(
            "\nAdd entries to {MANIFEST_PATH} per TESTING.md § 6.5; \
             every /v1/projects/{{p}}/... endpoint needs a paired test."
        );
        Err(anyhow!(
            "{} unregistered project-scoped endpoint(s)",
            missing.len()
        ))
    }
}

fn collect_project_scoped_endpoints(spec: &serde_yaml::Value) -> Vec<(String, String)> {
    let Some(paths) = spec.get("paths").and_then(|p| p.as_mapping()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (path_key, methods) in paths {
        let Some(path) = path_key.as_str() else {
            continue;
        };
        if !is_project_scoped(path) {
            continue;
        }
        let Some(methods) = methods.as_mapping() else {
            continue;
        };
        for (m_key, _) in methods {
            let Some(method) = m_key.as_str() else {
                continue;
            };
            let upper = method.to_uppercase();
            if matches!(upper.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
                out.push((path.to_string(), upper));
            }
        }
    }
    out.sort();
    out
}

/// Project-scoped paths look like `/v1/projects/{<param>}/...`.
fn is_project_scoped(path: &str) -> bool {
    let prefix = "/v1/projects/{";
    if !path.starts_with(prefix) {
        return false;
    }
    // Past the prefix, expect `<paramName>}/<more>` — i.e. `}/`.
    path[prefix.len()..].contains("}/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_scoped_detection() {
        assert!(is_project_scoped("/v1/projects/{projectId}/advertisers"));
        assert!(is_project_scoped("/v1/projects/{p}/decisions"));
        assert!(!is_project_scoped("/v1/orgs/{orgId}/projects"));
        assert!(!is_project_scoped("/v1/projects/{projectId}")); // org-level project read
        assert!(!is_project_scoped("/healthz"));
    }

    #[test]
    fn collects_endpoints_from_spec() {
        let spec = serde_yaml::from_str::<serde_yaml::Value>(
            r#"
paths:
  /healthz:
    get: {}
  /v1/orgs/{orgId}/projects:
    get: {}
    post: {}
  /v1/projects/{projectId}/advertisers:
    get: {}
    post: {}
"#,
        )
        .unwrap();
        let endpoints = collect_project_scoped_endpoints(&spec);
        assert_eq!(
            endpoints,
            vec![
                ("/v1/projects/{projectId}/advertisers".into(), "GET".into()),
                ("/v1/projects/{projectId}/advertisers".into(), "POST".into()),
            ]
        );
    }
}
