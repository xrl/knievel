//! Build script.
//!
//! Captures the git SHA and build timestamp into compile-time env
//! vars so `/version` (Phase 2.6) can return them. Avoids the
//! vergen dep — a couple of git/date shell-outs are enough.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    // Allow CI to inject a known-good SHA via env when .git/ is
    // absent (e.g. shallow checkouts, Docker layer builds). The
    // `cargo:rerun-if-env-changed` directive ensures the build
    // script re-runs when the variable changes.
    println!("cargo:rerun-if-env-changed=GIT_REV");

    let sha = if let Ok(rev) = std::env::var("GIT_REV") {
        // CI explicit injection — trust it verbatim.
        rev
    } else {
        match run("git", &["rev-parse", "HEAD"]) {
            Some(s) => {
                let dirty = run("git", &["status", "--porcelain"])
                    .map(|o| !o.is_empty())
                    .unwrap_or(false);
                if dirty { format!("{s}-dirty") } else { s }
            }
            None => {
                // Emit a build warning so `cargo build` surfaces the
                // fact that version info is unavailable. This prevents
                // "unknown" silently making it into release images.
                println!("cargo:warning=KNIEVEL_GIT_SHA: git rev-parse failed; set GIT_REV env var or ensure .git/ is present");
                "unknown".into()
            }
        }
    };

    let timestamp = run("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]).unwrap_or_else(|| {
        println!("cargo:warning=KNIEVEL_BUILD_TIMESTAMP: date command failed; using 'unknown'");
        "unknown".into()
    });

    println!("cargo:rustc-env=KNIEVEL_GIT_SHA={sha}");
    println!("cargo:rustc-env=KNIEVEL_BUILD_TIMESTAMP={timestamp}");
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd).args(args).output().ok().and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    })
}
