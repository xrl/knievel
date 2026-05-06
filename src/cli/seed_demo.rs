//! `knievel-cli seed-demo` — populates a fresh knievel install
//! with a sample org, project, advertiser, campaign, flight, ad,
//! creative, site, and zone, plus an org-admin token a contributor
//! can use to issue meaningful decisions immediately.
//!
//! Phase 4.2. Refs: `REQUIREMENTS.md` § 8 item 4 ("seed-demo —
//! populates a fresh knievel install with a sample org, project,
//! advertisers, flights, ads, and creatives"); `AUTH.md` "Local
//! Development" — opaque-token-only bootstrap; `MIGRATION_RX.md`
//! "Local Development for RX Engineers."
//!
//! Bootstrap path is **DB-direct** (not HTTP). Auth's chicken-and-
//! egg means there's no token in existence on a clean install, so
//! the first token has to be inserted directly. The compose
//! `knievel-seed` sidecar runs this against the same Postgres the
//! server is using; readyz polling lives at the compose layer
//! (Phase 4.1).
//!
//! Every step is **idempotent** — re-running `seed-demo` against a
//! cluster that already has the demo data finds and reuses every
//! row by `external_id` rather than failing on a unique-constraint
//! collision. The token is rewritten on each run when
//! `--write-token-to` is set.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::auth::opaque;
use crate::db;
use crate::taxonomy;

/// Inputs to `seed-demo`. The CLI populates this from clap; the
/// integration test populates it directly.
pub struct SeedDemoArgs {
    pub database_url: String,
    pub org_external_id: String,
    pub project_external_id: String,
    /// Pre-supplied token in `kvl_<env>_org_<id>_<secret>` form.
    /// When `None`, a random one is generated.
    pub token: Option<String>,
    /// Path to write the plaintext token to. The file is
    /// created with mode 0600 on Unix. The directory must already
    /// exist (we don't `mkdir -p` to avoid surprising side effects).
    pub write_token_to: Option<PathBuf>,
}

/// Output of `seed-demo` — the IDs of every seeded row, plus the
/// plaintext bearer.
pub struct SeedDemoOutput {
    pub org_id: String,
    pub project_id: String,
    pub advertiser_id: i64,
    pub campaign_id: i64,
    pub flight_id: i64,
    pub ad_id: i64,
    pub creative_id: i64,
    pub site_id: i64,
    pub zone_id: i64,
    pub priority_id: i64,
    pub ad_type_id: i64,
    pub token: String,
}

pub async fn run(args: SeedDemoArgs) -> Result<SeedDemoOutput> {
    let pool = connect(&args.database_url).await?;

    let (org_id, _) = upsert_org(&pool, &args.org_external_id).await?;
    let (project_id, project_was_new) =
        upsert_project(&pool, &org_id, &args.project_external_id).await?;
    if project_was_new {
        let mut tx = db::begin_bound(&pool, &org_id, Some(&project_id)).await?;
        taxonomy::seed_default_taxonomy(&mut tx, &org_id, &project_id)
            .await
            .context("seeding default taxonomy")?;
        tx.commit().await?;
    }

    let (priority_id, ad_type_id) = lookup_default_taxonomy(&pool, &org_id, &project_id).await?;
    let advertiser_id = upsert_advertiser(&pool, &org_id, &project_id).await?;
    let campaign_id = upsert_campaign(&pool, &org_id, &project_id, advertiser_id).await?;
    let flight_id = upsert_flight(
        &pool,
        &org_id,
        &project_id,
        campaign_id,
        priority_id,
        ad_type_id,
    )
    .await?;
    let creative_id = upsert_creative(&pool, &org_id, &project_id, advertiser_id).await?;
    let ad_id = upsert_ad(&pool, &org_id, &project_id, flight_id, creative_id).await?;
    let site_id = upsert_site(&pool, &org_id, &project_id).await?;
    let zone_id = upsert_zone(&pool, &org_id, &project_id, site_id).await?;

    let token = upsert_token(&pool, &org_id, args.token.as_deref()).await?;

    if let Some(path) = args.write_token_to.as_deref() {
        write_token_file(path, &token).context("writing token file")?;
    }

    Ok(SeedDemoOutput {
        org_id,
        project_id,
        advertiser_id,
        campaign_id,
        flight_id,
        ad_id,
        creative_id,
        site_id,
        zone_id,
        priority_id,
        ad_type_id,
        token,
    })
}

async fn connect(url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET search_path TO knievel, public")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .context("connecting to Postgres")
}

/// Upsert the demo org by deterministically deriving its `id` from
/// `external_id`. Direct `SELECT … WHERE external_id = …` on
/// `organizations` is RLS-blocked when no `knievel.org_id` is bound
/// (chicken-and-egg: we don't know the id yet); going through
/// `INSERT … ON CONFLICT (id) DO UPDATE` with the bound id sidesteps
/// the lookup entirely. The 48-bit derived id has astronomically
/// low collision probability for v0 fixture use.
///
/// Returns `(org_id, was_new)`. `was_new` is `true` exactly when
/// this call inserted a fresh row (Postgres's `xmax = 0` test on
/// the returning row).
async fn upsert_org(pool: &PgPool, external_id: &str) -> Result<(String, bool)> {
    let id = derive_org_id(external_id);
    let mut tx = db::begin_bound(pool, &id, None).await?;
    let was_new: bool = sqlx::query_scalar(
        "INSERT INTO knievel.organizations (id, external_id, name)
         VALUES ($1, $2, 'Demo Org')
         ON CONFLICT (id) DO UPDATE
           SET name = knievel.organizations.name
         RETURNING (xmax = 0)",
    )
    .bind(&id)
    .bind(external_id)
    .fetch_one(&mut *tx)
    .await
    .context("upsert org")?;
    tx.commit().await?;
    Ok((id, was_new))
}

/// Upsert the demo project. The `(org_id, external_id)` lookup is
/// RLS-bound on `knievel.org_id` (we know it by now), so a SELECT
/// finds the existing row when present. Falls back to insert with
/// a derived id when missing.
async fn upsert_project(pool: &PgPool, org_id: &str, external_id: &str) -> Result<(String, bool)> {
    let mut tx = db::begin_bound(pool, org_id, None).await?;
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM knievel.projects WHERE org_id = $1 AND external_id = $2",
    )
    .bind(org_id)
    .bind(external_id)
    .fetch_optional(&mut *tx)
    .await
    .context("lookup project")?
    {
        tx.commit().await?;
        return Ok((id, false));
    }

    let id = derive_project_id(org_id, external_id);
    sqlx::query(
        "INSERT INTO knievel.projects (id, org_id, external_id, name)
         VALUES ($1, $2, $3, 'Demo Project')",
    )
    .bind(&id)
    .bind(org_id)
    .bind(external_id)
    .execute(&mut *tx)
    .await
    .context("insert project")?;
    tx.commit().await?;
    Ok((id, true))
}

fn derive_org_id(external_id: &str) -> String {
    let h = Sha256::digest(external_id.as_bytes());
    format!("org_{}", &hex::encode(h)[..12])
}

fn derive_project_id(org_id: &str, external_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(org_id.as_bytes());
    h.update(b"/");
    h.update(external_id.as_bytes());
    format!("pj_{}", &hex::encode(h.finalize())[..12])
}

/// Resolve the `(priority_id, ad_type_id)` the demo flight will
/// reference. Picks the seeded "Standard" priority and
/// "Medium Rectangle" ad type per `taxonomy::seed_default_taxonomy`.
async fn lookup_default_taxonomy(
    pool: &PgPool,
    org_id: &str,
    project_id: &str,
) -> Result<(i64, i64)> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    let priority_id: i64 = sqlx::query_scalar(
        "SELECT id FROM knievel.priorities
         WHERE project_id = $1 AND name = 'Standard'",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await
    .context("lookup Standard priority")?;
    let ad_type_id: i64 = sqlx::query_scalar(
        "SELECT id FROM knievel.ad_types
         WHERE project_id = $1 AND name = 'Medium Rectangle'",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await
    .context("lookup Medium Rectangle ad type")?;
    tx.commit().await?;
    Ok((priority_id, ad_type_id))
}

async fn upsert_advertiser(pool: &PgPool, org_id: &str, project_id: &str) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.advertisers
         WHERE project_id = $1 AND external_id = 'demo-advertiser'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.advertisers (org_id, project_id, external_id, name)
         VALUES ($1, $2, 'demo-advertiser', 'Demo Advertiser') RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_campaign(
    pool: &PgPool,
    org_id: &str,
    project_id: &str,
    advertiser_id: i64,
) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.campaigns
         WHERE project_id = $1 AND external_id = 'demo-campaign'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.campaigns
            (org_id, project_id, advertiser_id, external_id, name)
         VALUES ($1, $2, $3, 'demo-campaign', 'Demo Campaign')
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(advertiser_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_flight(
    pool: &PgPool,
    org_id: &str,
    project_id: &str,
    campaign_id: i64,
    priority_id: i64,
    ad_type_id: i64,
) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.flights
         WHERE project_id = $1 AND external_id = 'demo-flight'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    // Always-on flight: no start/end date, ad_types matching the
    // seeded Medium Rectangle, no site/zone restriction.
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.flights
            (org_id, project_id, campaign_id, external_id, name,
             priority_id, ad_types)
         VALUES ($1, $2, $3, 'demo-flight', 'Demo Flight',
                 $4, ARRAY[$5]::bigint[])
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(campaign_id)
    .bind(priority_id)
    .bind(ad_type_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_creative(
    pool: &PgPool,
    org_id: &str,
    project_id: &str,
    advertiser_id: i64,
) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.creatives
         WHERE project_id = $1 AND external_id = 'demo-creative'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.creatives
            (org_id, project_id, advertiser_id, external_id, name,
             kind, image_url, width, height, alt, click_through_url)
         VALUES ($1, $2, $3, 'demo-creative', 'Demo Creative',
                 'image',
                 'https://demo.example.com/banner-300x250.png',
                 300, 250, 'Demo banner',
                 'https://demo.example.com/landing')
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(advertiser_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_ad(
    pool: &PgPool,
    org_id: &str,
    project_id: &str,
    flight_id: i64,
    creative_id: i64,
) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.ads
         WHERE project_id = $1 AND external_id = 'demo-ad'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.ads
            (org_id, project_id, flight_id, creative_id,
             external_id, weight)
         VALUES ($1, $2, $3, $4, 'demo-ad', 100)
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(flight_id)
    .bind(creative_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_site(pool: &PgPool, org_id: &str, project_id: &str) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.sites
         WHERE project_id = $1 AND external_id = 'demo-site'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.sites
            (org_id, project_id, external_id, name, url)
         VALUES ($1, $2, 'demo-site', 'Demo Site',
                 'https://demo.example.com')
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn upsert_zone(pool: &PgPool, org_id: &str, project_id: &str, site_id: i64) -> Result<i64> {
    let mut tx = db::begin_bound(pool, org_id, Some(project_id)).await?;
    if let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM knievel.zones
         WHERE project_id = $1 AND external_id = 'demo-zone'",
    )
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(id);
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO knievel.zones
            (org_id, project_id, site_id, external_id, name)
         VALUES ($1, $2, $3, 'demo-zone', 'Demo Zone')
         RETURNING id",
    )
    .bind(org_id)
    .bind(project_id)
    .bind(site_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Mint or reuse the demo bearer. When `supplied` is `Some(t)`,
/// parses it (must be `kvl_<env>_org_<id_short>_<secret>`) and
/// either upserts the matching row in `api_tokens` or rotates the
/// hash so the wire token always works after `seed-demo` returns.
/// When `None`, generates a random `kvl_dev_org_<id>_<secret>`.
async fn upsert_token(pool: &PgPool, org_id: &str, supplied: Option<&str>) -> Result<String> {
    let plaintext = match supplied {
        Some(t) => t.to_string(),
        None => {
            let id_short = random_hex(6)?;
            let secret = random_hex(16)?;
            format!("kvl_dev_org_{id_short}_{secret}")
        }
    };
    let parsed = opaque::parse(&plaintext).map_err(|e| anyhow!("invalid --token: {e}"))?;
    if parsed.scope != "org" {
        return Err(anyhow!(
            "--token must be org-scoped (kvl_<env>_org_<id>_<secret>)"
        ));
    }
    let db_id = parsed.db_id();
    let hash = opaque::hash(parsed.secret).context("hashing token secret")?;

    let mut tx = db::begin_bound(pool, org_id, None).await?;
    // ON CONFLICT updates the secret hash so re-running with the
    // same --token rotates the row to match the supplied secret.
    sqlx::query(
        "INSERT INTO knievel.api_tokens
            (id, org_id, scope, role, name, secret_hash)
         VALUES ($1, $2, 'org', 'org-admin', 'seed-demo bootstrap', $3)
         ON CONFLICT (id) DO UPDATE
           SET secret_hash = EXCLUDED.secret_hash,
               revoked_at  = NULL",
    )
    .bind(&db_id)
    .bind(org_id)
    .bind(&hash)
    .execute(&mut *tx)
    .await
    .context("upsert api_token")?;
    tx.commit().await?;
    Ok(plaintext)
}

fn write_token_file(path: &Path, token: &str) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .with_context(|| format!("opening {} for write", path.display()))?;
    f.write_all(token.as_bytes())?;
    f.write_all(b"\n")?;
    Ok(())
}

fn random_hex(bytes: usize) -> Result<String> {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut buf = vec![0u8; bytes];
    let mut rng = OsRng;
    rng.fill_bytes(&mut buf);
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
