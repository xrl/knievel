//! `knievel-cli admin` — operator-level subcommands. Today the
//! only action is `create-org`; future work adds `mint-token`,
//! `revoke-token`, and `list-orgs`.
//!
//! ## `create-org`
//!
//! Provision a new tenant (one row in `knievel.organizations` plus
//! one bootstrap row in `knievel.api_tokens`) on a running knievel
//! install. This is the production-shaped sibling of `seed-demo`:
//! `seed-demo` exists to populate a fresh install with a sample
//! advertiser/campaign/flight/ad/creative/site/zone fixture chain
//! suitable for compose-up dev and CI integration tests; in
//! production you only want the org row + an org-admin token, not
//! the demo cruft.
//!
//! Why this is DB-direct rather than HTTP: an org is a tenant
//! boundary, not an HTTP-creatable resource. There is no
//! `POST /orgs` endpoint in the OpenAPI surface — see
//! `AUTH.md` "Authorization", which scopes every API role to a
//! specific org. Org creation lives at the operator tier (kubectl
//! exec into a knievel pod, run this command), not at the API
//! tier.
//!
//! Idempotent: re-running with the same `--external-id` finds the
//! existing org row by the deterministic id derivation in
//! `seed_demo::derive_org_id` and returns it unchanged. Re-running
//! with the same `--token` rotates the row's `secret_hash` rather
//! than inserting a new token row.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::seed_demo;

pub struct CreateOrgArgs {
    pub database_url: String,
    /// Stable external identifier the operator chose for the
    /// tenant (e.g. `rx`, `acme-corp`). Used as the lookup key
    /// and as the input to the deterministic id derivation, so
    /// re-running `create-org` with the same `--external-id`
    /// finds the same org row.
    pub external_id: String,
    /// Human-readable display name. Stored in
    /// `organizations.name` on first insert; preserved verbatim
    /// on re-runs so an operator-edited name isn't clobbered by
    /// a fixture default.
    pub name: String,
    /// Pre-supplied bearer in `kvl_<env>_org_<id_short>_<secret>`
    /// form. When omitted, a random `kvl_dev_org_*` is generated.
    /// Re-supplying the same token rotates the row's hash.
    pub token: Option<String>,
    /// Path to write the plaintext bearer to. Created with mode
    /// 0600 on Unix. The parent directory must already exist.
    pub write_token_to: Option<PathBuf>,
}

pub struct CreateOrgOutput {
    pub org_id: String,
    pub token: String,
    /// `true` when this call inserted a fresh `organizations`
    /// row; `false` when an existing row was reused.
    pub org_was_new: bool,
}

pub async fn create_org(args: CreateOrgArgs) -> Result<CreateOrgOutput> {
    let pool = seed_demo::connect(&args.database_url).await?;

    let (org_id, org_was_new) = seed_demo::upsert_org(&pool, &args.external_id, &args.name).await?;

    let token = seed_demo::upsert_token(
        &pool,
        &org_id,
        args.token.as_deref(),
        "create-org bootstrap",
    )
    .await?;

    if let Some(path) = args.write_token_to.as_deref() {
        seed_demo::write_token_file(path, &token)?;
    }

    Ok(CreateOrgOutput {
        org_id,
        token,
        org_was_new,
    })
}
