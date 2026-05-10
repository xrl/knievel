//! `knievel-cli` — admin CLI sibling of the knievel server binary.
//!
//! Phase 4.2 introduced `seed-demo`. The `admin` subcommand surface
//! covers operator-level tasks (today: `create-org`; future:
//! `mint-token`, `revoke-token`, `list-orgs`). Further phases add
//! `migrate` (run migrations standalone), `snapshot` (debug-inspect),
//! and a generated subcommand surface that wraps the OpenAPI client.
//!
//! Refs: `REQUIREMENTS.md` § 8 item 4; `AUTH.md` "Local
//! Development" + "Authorization"; `MIGRATION_RX.md` "Local
//! Development for RX Engineers."

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use knievel::cli::{admin, seed_demo};

#[derive(Parser)]
#[command(
    name = "knievel-cli",
    version,
    about = "knievel admin / fixtures CLI",
    long_about = "Admin CLI sibling of the knievel server binary. \
                  See `knievel-cli seed-demo --help` for the \
                  bootstrap-fixtures subcommand."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Populate a fresh knievel install with a demo org / project /
    /// advertiser / flight / ad / creative / site / zone, plus an
    /// org-admin token. Idempotent; safe to re-run.
    SeedDemo(SeedDemoFlags),

    /// Operator-level subcommands (org / token administration).
    #[command(subcommand)]
    Admin(AdminCommand),
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Provision a new tenant: one row in `organizations` plus an
    /// org-admin bootstrap token. Production-shaped sibling of
    /// `seed-demo` (no demo advertiser/campaign/flight/ad/creative/
    /// site/zone fixture chain). Idempotent; safe to re-run.
    CreateOrg(CreateOrgFlags),
}

#[derive(clap::Args)]
struct CreateOrgFlags {
    /// Postgres connection string. Defaults to the `DATABASE_URL`
    /// env var. Required (either flag or env).
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Stable external identifier for the tenant (e.g. `rx`,
    /// `acme-corp`). Used as the lookup key on re-run.
    #[arg(long)]
    external_id: String,

    /// Human-readable display name. Defaults to `--external-id`
    /// when omitted.
    #[arg(long)]
    name: Option<String>,

    /// Pre-supplied bearer in `kvl_<env>_org_<id_short>_<secret>`
    /// form. When omitted, a random `kvl_dev_org_*` is generated.
    /// Re-supplying the same token rotates the row's hash.
    #[arg(long)]
    token: Option<String>,

    /// Path to write the plaintext bearer to. Created with mode
    /// 0600 on Unix. The parent directory must already exist.
    #[arg(long)]
    write_token_to: Option<PathBuf>,
}

#[derive(clap::Args)]
struct SeedDemoFlags {
    /// Postgres connection string. Defaults to the `DATABASE_URL`
    /// env var. Required (either flag or env).
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// `external_id` of the org row. Reused if it already exists.
    #[arg(long, default_value = "demo-org")]
    org_external_id: String,

    /// `external_id` of the project row. Reused if it already
    /// exists under the org.
    #[arg(long, default_value = "demo-project")]
    project_external_id: String,

    /// Pre-supplied bearer in `kvl_<env>_org_<id_short>_<secret>`
    /// form. When omitted, a random `kvl_dev_org_*` is generated.
    /// Re-supplying the same token rotates the row's hash.
    #[arg(long)]
    token: Option<String>,

    /// Path to write the plaintext bearer to. Created with mode
    /// 0600 on Unix. The parent directory must already exist.
    #[arg(long)]
    write_token_to: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::SeedDemo(args) => match run_seed_demo(args).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("seed-demo failed: {e:#}");
                ExitCode::FAILURE
            }
        },
        Command::Admin(AdminCommand::CreateOrg(args)) => match run_create_org(args).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("admin create-org failed: {e:#}");
                ExitCode::FAILURE
            }
        },
    }
}

async fn run_seed_demo(flags: SeedDemoFlags) -> anyhow::Result<()> {
    let out = seed_demo::run(seed_demo::SeedDemoArgs {
        database_url: flags.database_url,
        org_external_id: flags.org_external_id,
        project_external_id: flags.project_external_id,
        token: flags.token,
        write_token_to: flags.write_token_to.clone(),
    })
    .await?;

    println!(
        "seed-demo: org_id={} project_id={}",
        out.org_id, out.project_id
    );
    println!(
        "  advertiser_id={} campaign_id={} flight_id={} ad_id={}",
        out.advertiser_id, out.campaign_id, out.flight_id, out.ad_id
    );
    println!(
        "  creative_id={} site_id={} zone_id={}",
        out.creative_id, out.site_id, out.zone_id
    );
    println!(
        "  priority_id={} ad_type_id={}",
        out.priority_id, out.ad_type_id
    );

    // The token is written to the file path when set; print it to
    // stdout only when no file path was supplied so accidental
    // pipes / `docker compose logs` don't leak the bearer.
    if flags.write_token_to.is_none() {
        println!("  token={}", out.token);
    } else if let Some(p) = &flags.write_token_to {
        println!("  token written to {}", p.display());
    }

    Ok(())
}

async fn run_create_org(flags: CreateOrgFlags) -> anyhow::Result<()> {
    let display_name = flags
        .name
        .clone()
        .unwrap_or_else(|| flags.external_id.clone());
    let out = admin::create_org(admin::CreateOrgArgs {
        database_url: flags.database_url,
        external_id: flags.external_id,
        name: display_name,
        token: flags.token,
        write_token_to: flags.write_token_to.clone(),
    })
    .await?;

    let status = if out.org_was_new { "created" } else { "reused" };
    println!("admin create-org: org_id={} ({status})", out.org_id);

    if flags.write_token_to.is_none() {
        println!("  token={}", out.token);
    } else if let Some(p) = &flags.write_token_to {
        println!("  token written to {}", p.display());
    }

    Ok(())
}
