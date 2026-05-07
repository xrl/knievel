//! Admin-UI typed-client codegen + drift gate.
//!
//! `cargo xtask ui-client` regenerates
//! `web/admin/src/api/generated.ts` from the committed
//! `openapi.yaml` via `pnpm exec openapi-typescript`.
//! `cargo xtask ui-client --check` fails CI when the
//! committed file differs from a fresh regeneration.
//!
//! Mirrors `xtask/src/openapi.rs` exactly — same exit-code
//! shape, same error messaging, so contributors who've seen
//! one drift error recognize the other.
//!
//! See `UI.md` "OpenAPI codegen" for the design and
//! `PHASES.md` 7.3 for the phase context.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

const SPEC_PATH: &str = "openapi.yaml";
const CLIENT_PATH: &str = "web/admin/src/api/generated.ts";
const WORKDIR: &str = "web/admin";
// Drift-check writes a fresh codegen here (under `target/`,
// which is gitignored) and compares to the committed file.
// Fixed location avoids a `tempfile` dep on xtask.
const CHECK_OUT: &str = "target/xtask-ui-client-check.ts";

pub fn run(check: bool) -> Result<()> {
    if !Path::new(SPEC_PATH).exists() {
        return Err(anyhow!(
            "{SPEC_PATH} does not exist; regenerate via `cargo xtask openapi`"
        ));
    }

    if check {
        check_drift()
    } else {
        regenerate(PathBuf::from(CLIENT_PATH))?;
        let bytes = fs::metadata(CLIENT_PATH)
            .with_context(|| format!("stat {CLIENT_PATH}"))?
            .len();
        println!("xtask ui-client: wrote {CLIENT_PATH} ({bytes} bytes)");
        Ok(())
    }
}

fn check_drift() -> Result<()> {
    let on_disk = match fs::read_to_string(CLIENT_PATH) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!(
                "{CLIENT_PATH} does not exist; regenerate via `cargo xtask ui-client`"
            ));
        }
        Err(e) => return Err(e).with_context(|| format!("reading {CLIENT_PATH}")),
    };

    if let Some(parent) = Path::new(CHECK_OUT).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {} for drift output", parent.display()))?;
    }
    regenerate(PathBuf::from(CHECK_OUT))?;
    let fresh = fs::read_to_string(CHECK_OUT)
        .with_context(|| format!("reading fresh codegen output at {CHECK_OUT}"))?;

    if on_disk == fresh {
        println!(
            "xtask ui-client --check: {CLIENT_PATH} matches fresh codegen ({} bytes)",
            on_disk.len()
        );
        Ok(())
    } else {
        eprintln!("{CLIENT_PATH} drift detected. Regenerate with: cargo xtask ui-client");
        Err(anyhow!("{CLIENT_PATH} is out of date"))
    }
}

/// Shell out to `pnpm exec openapi-typescript` from inside
/// `web/admin/` so the tool resolves against the workspace's
/// pinned version rather than whatever happens to be on PATH.
/// `out_path` is passed as an absolute path so it survives the
/// `cwd` change to `web/admin/` regardless of where the caller
/// pointed it (canonical `web/admin/src/api/generated.ts` or
/// the drift-check temp under `target/`).
fn regenerate(out_path: PathBuf) -> Result<()> {
    let abs_out = if out_path.is_absolute() {
        out_path
    } else {
        std::env::current_dir()
            .context("getting current_dir")?
            .join(out_path)
    };
    let abs_out_str = abs_out
        .to_str()
        .ok_or_else(|| anyhow!("output path is not valid UTF-8: {}", abs_out.display()))?;
    let status = Command::new("pnpm")
        .args([
            "exec",
            "openapi-typescript",
            "../../openapi.yaml",
            "-o",
            abs_out_str,
        ])
        .current_dir(WORKDIR)
        .status()
        .with_context(|| format!("spawning `pnpm exec openapi-typescript` in {WORKDIR}"))?;
    if !status.success() {
        return Err(anyhow!(
            "openapi-typescript exited {} — is `pnpm install` up to date in {WORKDIR}?",
            status
        ));
    }
    Ok(())
}
