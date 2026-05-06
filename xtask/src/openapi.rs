//! OpenAPI spec generator + drift gate
//! (`TESTING.md` § 6.3, § 12.7).
//!
//! `cargo xtask openapi` regenerates `openapi.yaml` from the
//! binary's spec.
//! `cargo xtask openapi --check` fails CI if the committed file
//! differs.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

const SPEC_PATH: &str = "openapi.yaml";

pub fn run(check: bool) -> Result<()> {
    let spec = knievel::openapi_spec_yaml();
    let path = Path::new(SPEC_PATH);

    if check {
        let on_disk = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow!(
                    "{SPEC_PATH} does not exist; regenerate via `cargo xtask openapi`"
                ));
            }
            Err(e) => return Err(e).with_context(|| format!("reading {SPEC_PATH}")),
        };
        if on_disk == spec {
            println!(
                "xtask openapi --check: {} matches binary spec ({} bytes)",
                SPEC_PATH,
                spec.len()
            );
            Ok(())
        } else {
            eprintln!("{SPEC_PATH} drift detected. Regenerate with: cargo xtask openapi");
            Err(anyhow!("{SPEC_PATH} is out of date"))
        }
    } else {
        fs::write(path, &spec).with_context(|| format!("writing {SPEC_PATH}"))?;
        println!("xtask openapi: wrote {} ({} bytes)", SPEC_PATH, spec.len());
        Ok(())
    }
}
