use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod check_api_doc;
mod check_cross_tenant;
mod check_doc_fences;
mod lint_migrations;
mod openapi;
mod test_shape;

#[derive(Parser)]
#[command(name = "xtask", about = "Repo-internal CLI: linters, codegen, drift checks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lint migration files for tenant-isolation gate (REQUIREMENTS.md §7.1.1).
    LintMigrations {
        /// Directory to scan for `.sql` migration files.
        #[arg(long, default_value = "migrations")]
        path: PathBuf,
    },
    /// Verify every project-scoped endpoint has a paired cross-tenant test (TESTING.md §6.5).
    CheckCrossTenant,
    /// Generate or check the OpenAPI spec against the binary.
    Openapi {
        /// Fail if the committed `openapi.yaml` differs from the binary's spec.
        #[arg(long)]
        check: bool,
    },
    /// Verify test files follow the slicing naming convention (TESTING.md §12.5).
    TestShape,
    /// Parse every fenced code block in `.md` files (DOCUMENTATION_PLAN.md §11.2).
    CheckDocFences,
    /// Verify every OpenAPI operation has a row in API.md (DOCUMENTATION_PLAN.md §11.2).
    CheckApiDoc,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::LintMigrations { path } => lint_migrations::run(path),
        Cmd::CheckCrossTenant        => check_cross_tenant::run(),
        Cmd::Openapi { check }       => openapi::run(check),
        Cmd::TestShape               => test_shape::run(),
        Cmd::CheckDocFences          => check_doc_fences::run(),
        Cmd::CheckApiDoc             => check_api_doc::run(),
    }
}
