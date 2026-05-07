//! Release-tagging workflow — bumps `Cargo.toml`'s workspace
//! version, refreshes `Cargo.lock`, rolls `CHANGELOG.md`'s
//! `[Unreleased]` section to the new version with today's date,
//! commits, tags, and prints the push commands. Pairs with
//! `RELEASE_CHECKLIST.md` (operator runs that checklist before
//! invoking this command).
//!
//! Invocation:
//!
//!     cargo xtask release-tag 0.1.7
//!     # … runs gates, edits files, commits, tags …
//!     # then operator runs:
//!     git push origin main
//!     git push origin v0.1.7
//!
//! Deliberately stops short of pushing — pushing the tag fires
//! every release workflow (image, cosign, GitHub Release,
//! release-ruby-gem.yml → gem push). The local commit + tag
//! gives the operator a chance to review before triggering the
//! full pipeline.
//!
//! What runs:
//!
//! 1. **Pre-flight gates** (`--skip-gates` to bypass for
//!    emergencies): `cargo fmt --check`, `cargo clippy
//!    --workspace --all-targets -- -D warnings`,
//!    `cargo test --workspace`, `xtask lint-migrations`,
//!    `xtask check-cross-tenant`, `xtask openapi --check`,
//!    `xtask check-doc-fences`, `xtask check-api-doc`,
//!    `xtask check-snake-case`, `xtask test-shape`.
//! 2. **Working-tree clean check.** Refuses to run with
//!    uncommitted changes.
//! 3. **Bump `Cargo.toml`** workspace.version to the target.
//! 4. **Refresh `Cargo.lock`** via `cargo build --offline -q`.
//! 5. **Roll `CHANGELOG.md`** — replace `## [Unreleased]` with
//!    `## [X.Y.Z] — YYYY-MM-DD`; insert a fresh empty
//!    `[Unreleased]` block; rewrite the bottom link references.
//! 6. **Commit + tag.** Subject `release: vX.Y.Z`. Tag is
//!    annotated (`git tag -a`), reuses the same message body.

use std::fs;
use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug)]
pub struct Args {
    pub version: String,
    pub skip_gates: bool,
}

pub fn run(args: Args) -> Result<()> {
    parse_semver(&args.version)?;
    println!("xtask release-tag: cutting v{}", args.version);

    ensure_clean_tree()?;

    if !args.skip_gates {
        run_gates()?;
    } else {
        eprintln!("WARN: --skip-gates set; releasing without local gate run");
    }

    bump_cargo_toml(&args.version)?;
    regen_openapi()?;
    refresh_cargo_lock()?;
    roll_changelog(&args.version)?;
    commit_and_tag(&args.version)?;

    println!();
    println!("xtask release-tag: done. Local commit + tag created. To publish:");
    println!();
    println!("    git push origin main");
    println!("    git push origin v{}", args.version);
    println!();
    println!("Pushing the tag fires release.yml + release-ruby-gem.yml.");
    Ok(())
}

fn parse_semver(version: &str) -> Result<()> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) {
        bail!("version must be `MAJOR.MINOR.PATCH` (got `{version}`)");
    }
    Ok(())
}

fn ensure_clean_tree() -> Result<()> {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("running git status")?;
    if !out.status.success() {
        bail!("git status failed");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.trim().is_empty() {
        bail!("working tree is not clean — commit or stash before running release-tag:\n{stdout}");
    }
    Ok(())
}

fn run_gates() -> Result<()> {
    println!("xtask release-tag: running gates…");
    for cmd_args in [
        // Cheap first.
        vec!["fmt", "--all", "--check"],
        vec!["xtask", "openapi", "--check"],
        vec!["xtask", "lint-migrations"],
        vec!["xtask", "check-cross-tenant"],
        vec!["xtask", "test-shape"],
        vec!["xtask", "check-doc-fences"],
        vec!["xtask", "check-api-doc"],
        vec!["xtask", "check-snake-case"],
        // Expensive last.
        vec![
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        vec!["test", "--workspace"],
    ] {
        let label = cmd_args.join(" ");
        eprintln!("  $ cargo {label}");
        let status = Command::new("cargo")
            .args(&cmd_args)
            .status()
            .with_context(|| format!("running cargo {label}"))?;
        if !status.success() {
            bail!("gate failed: cargo {label}");
        }
    }
    Ok(())
}

fn bump_cargo_toml(version: &str) -> Result<()> {
    let path = "Cargo.toml";
    let content = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    // Match the workspace.package's `version = "..."` line — the
    // shape `version    = "0.1.0"` we use today (extra spaces
    // before `=` allowed). Stop at the first match; the
    // workspace.package block is at the top of the file.
    let mut updated = false;
    let mut out = String::with_capacity(content.len());
    let mut in_workspace_package = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[workspace.package]") {
            in_workspace_package = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('[') && in_workspace_package {
            in_workspace_package = false;
        }
        if in_workspace_package && !updated && trimmed.starts_with("version") {
            // Preserve leading whitespace and the `version` token.
            let key = "version";
            if let Some(idx) = line.find(key) {
                let prefix = &line[..idx + key.len()];
                out.push_str(prefix);
                out.push_str(&format!("    = \"{version}\""));
                out.push('\n');
                updated = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !updated {
        bail!("could not find workspace.package version line in {path}");
    }
    fs::write(path, out).with_context(|| format!("writing {path}"))?;
    println!("xtask release-tag: bumped Cargo.toml to {version}");
    Ok(())
}

fn refresh_cargo_lock() -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "--offline", "--workspace", "--quiet"])
        .status()
        .context("running cargo build --offline to refresh Cargo.lock")?;
    if !status.success() {
        bail!("cargo build --offline failed (Cargo.lock may need a network refresh)");
    }
    Ok(())
}

/// Regenerate `openapi.yaml` after the Cargo.toml bump.
/// `info.version` in the spec is sourced from
/// `env!("CARGO_PKG_VERSION")` (see `src/lib.rs::openapi_spec_yaml`),
/// so a Cargo.toml version change makes the committed
/// `openapi.yaml` immediately stale and the openapi-drift gate
/// would fail CI on the release tag.
fn regen_openapi() -> Result<()> {
    let status = Command::new("cargo")
        .args(["xtask", "openapi"])
        .status()
        .context("running cargo xtask openapi")?;
    if !status.success() {
        bail!("cargo xtask openapi failed");
    }
    Ok(())
}

fn roll_changelog(version: &str) -> Result<()> {
    let path = "CHANGELOG.md";
    let content = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let today = today_iso();

    let mut out = String::with_capacity(content.len() + 256);
    let mut rolled_section = false;
    let mut rolled_links = false;

    for line in content.lines() {
        if !rolled_section && line.trim() == "## [Unreleased]" {
            // Insert a fresh [Unreleased] block, then stamp the
            // previous one as [X.Y.Z] — YYYY-MM-DD.
            out.push_str("## [Unreleased]\n\n### Added\n\n(none)\n\n### Changed\n\n(none)\n\n### Fixed\n\n(none)\n\n");
            out.push_str(&format!("## [{version}] — {today}\n"));
            rolled_section = true;
            continue;
        }
        // Bottom link references: rewrite [Unreleased] target +
        // insert a new [X.Y.Z] line.
        if !rolled_links && line.starts_with("[Unreleased]: ") {
            out.push_str(&format!(
                "[Unreleased]: https://github.com/knievel-ads/knievel/compare/v{version}...HEAD\n"
            ));
            // Pull previous version off the next existing tag link
            // by reading directly from `git tag` to keep the
            // compare-link accurate even if CHANGELOG was edited
            // by hand mid-cycle.
            let prev = previous_tag_or_default()?;
            out.push_str(&format!(
                "[{version}]: https://github.com/knievel-ads/knievel/compare/{prev}...v{version}\n"
            ));
            rolled_links = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    if !rolled_section {
        bail!("could not find `## [Unreleased]` heading in {path}");
    }
    if !rolled_links {
        bail!("could not find `[Unreleased]: …` link reference at the bottom of {path}");
    }

    fs::write(path, out).with_context(|| format!("writing {path}"))?;
    println!("xtask release-tag: rolled CHANGELOG.md to [{version}] — {today}");
    Ok(())
}

fn today_iso() -> String {
    // Avoids pulling a chrono/time dep just for today's date —
    // shell out to `date` in ISO format. Linux + macOS both
    // accept `+%Y-%m-%d`.
    let out = Command::new("date").arg("+%Y-%m-%d").output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_owned(),
        _ => "TODO-DATE".to_owned(),
    }
}

fn previous_tag_or_default() -> Result<String> {
    let out = Command::new("git")
        .args(["tag", "-l", "v*", "--sort=-v:refname"])
        .output()
        .context("running git tag -l")?;
    if !out.status.success() {
        bail!("git tag failed");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let prev = stdout
        .lines()
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| "v0.0.0".to_owned());
    Ok(prev)
}

fn commit_and_tag(version: &str) -> Result<()> {
    let stage_status = Command::new("git")
        .args([
            "add",
            "Cargo.toml",
            "Cargo.lock",
            "openapi.yaml",
            "CHANGELOG.md",
        ])
        .status()
        .context("git add")?;
    if !stage_status.success() {
        bail!("git add failed");
    }

    let msg = format!(
        "release: v{version}\n\nMechanical bump via `cargo xtask release-tag {version}`. \
        Pre-flight gates green at the source SHA. Pairs with the \
        RELEASE_CHECKLIST.md the operator filed in the release PR.\n\n\
        - Cargo.toml workspace.version → {version}\n\
        - Cargo.lock refreshed\n\
        - CHANGELOG.md [Unreleased] → [{version}]"
    );
    let commit_status = Command::new("git")
        .args(["commit", "-m", &msg])
        .status()
        .context("git commit")?;
    if !commit_status.success() {
        bail!("git commit failed");
    }

    let tag = format!("v{version}");
    let tag_status = Command::new("git")
        .args(["tag", "-a", &tag, "-m", &format!("Release {tag}")])
        .status()
        .context("git tag")?;
    if !tag_status.success() {
        bail!("git tag failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_accepts_xyz() {
        parse_semver("0.1.0").unwrap();
        parse_semver("10.20.300").unwrap();
    }

    #[test]
    fn semver_rejects_garbage() {
        assert!(parse_semver("0.1").is_err());
        assert!(parse_semver("v0.1.0").is_err());
        assert!(parse_semver("0.1.0-rc.1").is_err());
        assert!(parse_semver("0.1.x").is_err());
    }

    #[test]
    fn cargo_toml_bump_finds_workspace_version() {
        let dir = std::env::temp_dir().join("knv_xtask_release_tag_bump");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Cargo.toml");
        std::fs::write(
            &path,
            "[workspace]\nresolver = \"2\"\n\n\
             [workspace.package]\nedition = \"2021\"\nversion    = \"0.1.0\"\n\n\
             [package]\nname = \"x\"\nversion.workspace = true\n",
        )
        .unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let r = bump_cargo_toml("0.1.7");
        std::env::set_current_dir(&cwd).unwrap();
        r.unwrap();
        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("version    = \"0.1.7\""));
        // Per-package `version.workspace = true` line is untouched.
        assert!(updated.contains("version.workspace = true"));
    }
}
