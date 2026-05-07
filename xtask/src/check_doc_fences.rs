//! Doc fenced-code-block syntax check
//! (`DOCUMENTATION_PLAN.md` § 11.2).
//!
//! Walks every `.md` file in the repo, extracts each fenced code
//! block, and parses by language tag:
//!
//! - `rust` → `syn::parse_file`
//! - `yaml` → `serde_yaml::from_str::<serde_yaml::Value>`
//! - `json` → `serde_json::from_str::<serde_json::Value>`
//!
//! Blocks tagged with the suffix `,ignore` are skipped — same
//! semantics as `rustdoc`. Unknown language tags (`text`,
//! `console`, `sh`, `bash`, `sql`, etc.) are skipped without
//! complaint; this gate is opt-in by language.
//!
//! Skips entire directories that we don't author: `.git`,
//! `target`, `node_modules`, `web/admin/node_modules`, the
//! generated `knievel-ruby` checkout.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "knievel-ruby",
    "tmp",
    "vendor",
];

pub fn run() -> Result<()> {
    let mut blocks_checked = 0_usize;
    let mut blocks_skipped = 0_usize;
    let mut errors: Vec<String> = Vec::new();

    for entry in WalkDir::new(".").into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !SKIP_DIRS.iter().any(|skip| *skip == name)
    }) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let blocks = extract_fenced_blocks(path)
            .with_context(|| format!("extracting fences from {}", path.display()))?;
        for block in blocks {
            if block.ignored {
                blocks_skipped += 1;
                continue;
            }
            match block.lang.as_deref() {
                Some("rust") => match syn::parse_file(&block.body) {
                    Ok(_) => blocks_checked += 1,
                    Err(e) => errors.push(format!(
                        "{}:{}: rust block does not parse: {e}",
                        path.display(),
                        block.start_line
                    )),
                },
                Some("yaml" | "yml") => {
                    match serde_yaml::from_str::<serde_yaml::Value>(&block.body) {
                        Ok(_) => blocks_checked += 1,
                        Err(e) => errors.push(format!(
                            "{}:{}: yaml block does not parse: {e}",
                            path.display(),
                            block.start_line
                        )),
                    }
                }
                Some("json") => match serde_json::from_str::<serde_json::Value>(&block.body) {
                    Ok(_) => blocks_checked += 1,
                    Err(e) => errors.push(format!(
                        "{}:{}: json block does not parse: {e}",
                        path.display(),
                        block.start_line
                    )),
                },
                _ => blocks_skipped += 1,
            }
        }
    }

    if errors.is_empty() {
        println!(
            "xtask check-doc-fences: {blocks_checked} block(s) checked, \
             {blocks_skipped} skipped (unknown language or ,ignore)"
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("{e}");
        }
        Err(anyhow!("{} doc-fence error(s)", errors.len()))
    }
}

/// One fenced code block extracted from a markdown file.
struct FencedBlock {
    /// Language tag without modifiers — `rust`, `yaml`, etc.
    lang: Option<String>,
    /// True when the fence carried `,ignore` (or any
    /// comma-separated `ignore` modifier).
    ignored: bool,
    /// 1-based line number of the opening fence.
    start_line: usize,
    /// Raw body of the block (between the fences).
    body: String,
}

/// Tiny markdown fence parser. Handles ```lang and ```lang,ignore
/// shapes. Doesn't handle nested fences (nobody nests
/// triple-backticks inside triple-backticks); ignores indented
/// fences (we author code blocks at column 0).
fn extract_fenced_blocks(path: &Path) -> Result<Vec<FencedBlock>> {
    let content = fs::read_to_string(path)?;
    let mut blocks = Vec::new();
    let mut iter = content.lines().enumerate();
    while let Some((idx, line)) = iter.next() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") {
            continue;
        }
        let info = trimmed.trim_start_matches('`').trim();
        if info.is_empty() {
            // Closing fence with no info — but we treat any opening
            // ``` with empty info as an unmarked code block; skip.
        }
        let mut parts = info.split(',').map(str::trim);
        let lang_raw = parts.next().unwrap_or("");
        let mut ignored = false;
        for modifier in parts {
            if modifier.eq_ignore_ascii_case("ignore") {
                ignored = true;
            }
        }
        let lang = if lang_raw.is_empty() {
            None
        } else {
            Some(lang_raw.to_owned())
        };
        let start_line = idx + 1;

        // Consume body until the closing ```.
        let mut body = String::new();
        let mut closed = false;
        for (_, body_line) in iter.by_ref() {
            if body_line.trim_start().starts_with("```") {
                closed = true;
                break;
            }
            body.push_str(body_line);
            body.push('\n');
        }
        if !closed {
            return Err(anyhow!(
                "{}:{}: unclosed fenced code block",
                path.display(),
                start_line
            ));
        }
        blocks.push(FencedBlock {
            lang,
            ignored,
            start_line,
            body,
        });
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("knv_xtask_doc_{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.md");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn extracts_lang_and_body() {
        let p = write_tmp("extract", "intro\n\n```rust\nfn main() {}\n```\n\ntext\n");
        let blocks = extract_fenced_blocks(&p).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].lang.as_deref(), Some("rust"));
        assert!(!blocks[0].ignored);
        assert_eq!(blocks[0].body.trim(), "fn main() {}");
    }

    #[test]
    fn handles_ignore_modifier() {
        let p = write_tmp(
            "ignore",
            "```rust,ignore\nthis is not valid rust at all\n```\n",
        );
        let blocks = extract_fenced_blocks(&p).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].ignored);
    }

    #[test]
    fn unclosed_block_fails() {
        let p = write_tmp("unclosed", "```rust\nfn x() {}\n");
        let r = extract_fenced_blocks(&p);
        assert!(r.is_err());
    }

    #[test]
    fn unknown_language_skipped() {
        let p = write_tmp("unknown", "```text\nanything\n```\n");
        let blocks = extract_fenced_blocks(&p).unwrap();
        assert_eq!(blocks[0].lang.as_deref(), Some("text"));
        // Run-level test: skipping happens in run(), not the parser.
    }
}
