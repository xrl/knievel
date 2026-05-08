//! `cargo xtask build-image` — local-dev wrapper that
//! orchestrates the same three things release.yml's
//! `publish-image` matrix does as bare steps:
//!
//!   1. `pnpm build` → `web/admin/dist/`     (admin UI bundle)
//!   2. `cargo build --release --locked
//!         --bin knievel --bin knievel-cli`  (native release binaries)
//!   3. Stage binaries + admin bundle into a tiny `docker-context/`
//!      and run `docker build` against the runtime-only Dockerfile.
//!
//! The Dockerfile itself does NOT compile Rust — see its header.
//! Building Rust as a bare step (here, and in CI) keeps the build
//! debuggable, lets the standard `target/` cache work, and avoids
//! the QEMU-emulation tax on cross-platform release builds.
//!
//! `--skip-ui` substitutes an empty dist/ so headless-API builds
//! don't need a Node toolchain.
//!
//! See `UI.md` "Deployment / Local dev wrapper".

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

const UI_DIR: &str = "web/admin";
const DIST_DIR: &str = "web/admin/dist";
const STAGE_DIR: &str = "docker-context";

pub struct Args {
    pub skip_ui: bool,
    pub tag: String,
}

pub fn run(args: Args) -> Result<()> {
    if args.skip_ui {
        // Substitute an empty dist/ so the staging step's copy
        // succeeds without a Node toolchain. The runtime env var
        // KNIEVEL_ADMIN_UI__STATIC_DIR can be unset at runtime
        // to skip the mount entirely.
        fs::create_dir_all(DIST_DIR).with_context(|| format!("creating empty {DIST_DIR}"))?;
        fs::write(
            Path::new(DIST_DIR).join(".gitkeep"),
            b"# Empty placeholder for headless API builds (cargo xtask build-image --skip-ui).\n",
        )
        .with_context(|| format!("writing {DIST_DIR}/.gitkeep"))?;
        println!("xtask build-image: --skip-ui — skipping admin UI build");
    } else {
        run_step(
            "pnpm install",
            "pnpm",
            &["install", "--frozen-lockfile"],
            UI_DIR,
        )?;
        run_step("pnpm build", "pnpm", &["build"], UI_DIR)?;
        println!("xtask build-image: web/admin/dist/ ready");
    }

    run_step(
        "cargo build --release",
        "cargo",
        &[
            "build",
            "--release",
            "--locked",
            "--bin",
            "knievel",
            "--bin",
            "knievel-cli",
        ],
        ".",
    )?;
    // strip is best-effort; ignore failure (it's pure size, not correctness).
    let _ = Command::new("strip")
        .args(["target/release/knievel", "target/release/knievel-cli"])
        .status();

    stage_context()?;

    run_step(
        "docker build",
        "docker",
        &["build", "-t", &args.tag, STAGE_DIR],
        ".",
    )?;
    println!("xtask build-image: built {}", args.tag);
    Ok(())
}

/// Stage the tiny Docker build context the runtime Dockerfile
/// expects: `./knievel`, `./knievel-cli`, `./web/admin/dist/`,
/// `./Dockerfile`. We use a clean directory rather than the repo
/// root so the build context doesn't include `target/` or
/// `.git/` or anything else heavy.
fn stage_context() -> Result<()> {
    let stage = Path::new(STAGE_DIR);
    if stage.exists() {
        fs::remove_dir_all(stage).with_context(|| format!("clearing {STAGE_DIR}"))?;
    }
    fs::create_dir_all(stage.join("web/admin"))
        .with_context(|| format!("creating {STAGE_DIR}/web/admin"))?;

    fs::copy("target/release/knievel", stage.join("knievel"))
        .context("staging target/release/knievel")?;
    fs::copy("target/release/knievel-cli", stage.join("knievel-cli"))
        .context("staging target/release/knievel-cli")?;
    fs::copy("Dockerfile", stage.join("Dockerfile")).context("staging Dockerfile")?;

    // `cp -r web/admin/dist docker-context/web/admin/` copies the
    // tree into the pre-created parent so it lands at
    // docker-context/web/admin/dist, matching what the Dockerfile
    // expects.
    let dest_parent = stage.join("web/admin").to_string_lossy().into_owned();
    let status = Command::new("cp")
        .args(["-r", "web/admin/dist", &dest_parent])
        .status()
        .context("spawning cp")?;
    if !status.success() {
        return Err(anyhow!("cp -r web/admin/dist {dest_parent} failed: {status}"));
    }
    println!("xtask build-image: staged context at {STAGE_DIR}/");
    Ok(())
}

fn run_step(label: &str, cmd: &str, args: &[&str], cwd: &str) -> Result<()> {
    println!("xtask build-image: {label} ({cmd} {})", args.join(" "));
    let status = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("spawning `{cmd}` in {cwd}"))?;
    if !status.success() {
        return Err(anyhow!("{label} exited {status}"));
    }
    Ok(())
}
