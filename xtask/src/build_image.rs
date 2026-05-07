//! `cargo xtask build-image` — local-dev wrapper that
//! (1) builds the admin UI bundle if it isn't already
//! present, then (2) runs `docker build .`.
//!
//! In CI the same two steps are split across the workflow
//! (Node setup + `pnpm build` happen as their own steps so
//! pnpm's store cache works natively, then
//! `docker/build-push-action` reads the resulting
//! `web/admin/dist/`). The xtask is the local-dev
//! convenience equivalent; `--skip-ui` substitutes an empty
//! directory so headless-API builds don't need a Node
//! toolchain.
//!
//! See `UI.md` "Deployment / Local dev wrapper".

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

const UI_DIR: &str = "web/admin";
const DIST_DIR: &str = "web/admin/dist";

pub struct Args {
    pub skip_ui: bool,
    pub tag: String,
}

pub fn run(args: Args) -> Result<()> {
    if args.skip_ui {
        // Substitute an empty dist/ so the Dockerfile's
        // `COPY web/admin/dist ...` succeeds without a
        // Node toolchain. The runtime env var
        // KNIEVEL_ADMIN_UI__STATIC_DIR can be unset at
        // runtime to skip the mount entirely.
        fs::create_dir_all(DIST_DIR).with_context(|| format!("creating empty {DIST_DIR}"))?;
        // Drop a marker so the layer hash is stable across
        // headless builds.
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
        "docker build",
        "docker",
        &["build", "-t", &args.tag, "."],
        ".",
    )?;
    println!("xtask build-image: built {}", args.tag);
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
