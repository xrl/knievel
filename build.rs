//! Build script.
//!
//! Captures the git SHA and build timestamp into compile-time env
//! vars so `/version` (Phase 2.6) can return them. Avoids the
//! vergen dep — a couple of git/date shell-outs are enough.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    let sha = run("git", &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = run("git", &["status", "--porcelain"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let sha = if dirty { format!("{sha}-dirty") } else { sha };

    let timestamp =
        run("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]).unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=KNIEVEL_GIT_SHA={sha}");
    println!("cargo:rustc-env=KNIEVEL_BUILD_TIMESTAMP={timestamp}");
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}
