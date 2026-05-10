//! HTTP server bootstrap.
//!
//! Binds `poem` to `cfg.api.bind_addr`, installs SIGTERM/SIGINT
//! handlers, and runs with poem's graceful-shutdown helper. Drain
//! and total timeouts come from `ApiConfig` (defaults: 30 s drain,
//! 60 s total — `REQUIREMENTS.md` § 10.7).
//!
//! System endpoints are described via `poem-openapi`; the spec is
//! served at `/openapi.json` (`API.md` § 5). New API surface is
//! added by extending `SystemApi` (or composing additional
//! `OpenApi` impls — Phase 3+ adds them).

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use poem::endpoint::StaticFilesEndpoint;
use poem::get;
use poem::http::Method;
use poem::listener::TcpListener;
use poem::middleware::Cors;
use poem::{EndpointExt, Route, Server};
use poem_openapi::{OpenApiService, ServerObject};

use std::sync::Arc;

use crate::ad_library::AdLibraryApi;
use crate::admin_ui;
use crate::ads::AdsApi;
use crate::advertisers::AdvertisersApi;
use crate::campaigns::CampaignsApi;
use crate::config::Config;
use crate::creative_templates::CreativeTemplatesApi;
use crate::creatives::CreativesApi;
use crate::decisions::{DecisionsApi, ExplainApi};
use crate::events;
use crate::flights::FlightsApi;
use crate::image_upload::InMemoryStore;
use crate::leader::{self, LeaderHandle};
use crate::orgs::OrgApi;
use crate::partitions;
use crate::request_log::{RequestLog, RequestLogConfig};
use crate::rollup;
use crate::sites::SitesApi;
use crate::state::{AppState, DecisionFlags};
use crate::system::SystemApi;
use crate::taxonomy::TaxonomyApi;
use crate::tokens::TokensApi;
use crate::whoami::WhoamiApi;
use crate::zones::ZonesApi;

pub async fn run(cfg: Config) -> Result<()> {
    let addr = SocketAddr::from_str(&cfg.api.bind_addr)
        .with_context(|| format!("invalid api.bind_addr: {}", cfg.api.bind_addr))?;

    // Phase 5.10: build_state now returns Result. A fatal DB error
    // exits before the listener binds — kubelet sees exit(1) and
    // enters CrashLoopBackOff. The caller (main.rs) does the actual
    // process::exit(1) so tests can catch Err without killing the
    // test runner.
    let state = build_state(&cfg).await?;
    let r = routes();
    let r = mount_admin_ui(r, cfg.admin_ui.static_dir.as_deref());
    let routes = r.data(state);

    // Conditional CORS install — empty `allowed_origins` means
    // "no admin UI hitting us cross-origin," so the middleware
    // isn't installed at all (no preflight overhead, no ACAO
    // header on responses). Same-origin deploys (UI served from
    // poem's StaticFilesEndpoint at `/admin/`) want this empty.
    let app: poem::endpoint::BoxEndpoint<'static> = match cors_layer(&cfg) {
        Some(cors) => {
            tracing::info!(
                origins = ?cfg.api.allowed_origins,
                "CORS enabled"
            );
            routes.with(cors).boxed()
        }
        None => routes.boxed(),
    };

    // Per-request structured logger + x-request-id stamping.
    // Installed AFTER CORS so the log-line latency reflects the
    // full request including any preflight handling. Operators
    // can disable via `logging.request_log_enabled = false` —
    // useful for hot-path-only fleets that ship per-request
    // observability via OTel spans instead.
    let app: poem::endpoint::BoxEndpoint<'static> = if cfg.logging.request_log_enabled {
        let mw = RequestLog::new(RequestLogConfig {
            skip_paths: std::sync::Arc::new(cfg.logging.request_log_skip_paths.clone()),
            slow_ms: cfg.logging.request_log_slow_ms,
        });
        tracing::info!(
            skip_paths = ?cfg.logging.request_log_skip_paths,
            slow_ms = cfg.logging.request_log_slow_ms,
            "request logging enabled"
        );
        app.with(mw).boxed()
    } else {
        tracing::info!("request logging disabled (logging.request_log_enabled = false)");
        app
    };

    tracing::info!(
        addr = %addr,
        drain_timeout_secs = cfg.api.shutdown_drain_timeout_secs,
        total_timeout_secs = cfg.api.shutdown_total_timeout_secs,
        "knievel listening"
    );

    Server::new(TcpListener::bind(addr))
        .run_with_graceful_shutdown(
            app,
            shutdown_signal(),
            Some(Duration::from_secs(cfg.api.shutdown_drain_timeout_secs)),
        )
        .await
        .map_err(|e| anyhow!("server error: {e}"))?;

    tracing::info!("knievel exited cleanly");
    Ok(())
}

/// Build the CORS middleware from config, or `None` when
/// `allowed_origins` is empty (which means "don't install CORS
/// at all" — see `UI.md` "CORS"). Public so tests can rebuild
/// the same layer shape against fixture configs.
///
/// Header allow/expose lists track the admin-UI surface the SPA
/// actually uses today: `Authorization` for bearer tokens,
/// `Content-Type` for JSON/multipart, `Idempotency-Key` for
/// POST/PATCH replay, `If-Match` for etag-guarded PATCH (when
/// it lands), and `X-Request-Id` for support correlation
/// (`UI.md` "Error handling"). Exposed: `ETag`, `Location`,
/// `X-Request-Id`, `X-Idempotency-Replayed`. `allow_credentials`
/// is `false` because the SPA uses Authorization headers, not
/// cookies (`UI.md` "Auth"); keeping it false also removes the
/// cookie-CSRF surface and the wildcard restriction.
pub fn cors_layer(cfg: &Config) -> Option<Cors> {
    if cfg.api.allowed_origins.is_empty() {
        return None;
    }
    let cors = Cors::new()
        .allow_origins(cfg.api.allowed_origins.iter().map(String::as_str))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            "authorization",
            "content-type",
            "idempotency-key",
            "if-match",
            "x-request-id",
        ])
        .expose_headers(["etag", "location", "x-request-id", "x-idempotency-replayed"])
        .allow_credentials(false)
        .max_age(600);
    Some(cors)
}

/// Top-level routes. Phase 2 wires the system OpenAPI service
/// plus its `/openapi.json` spec endpoint; Phase 3+ adds the
/// management + decision OpenAPI services as additional
/// `OpenApiService` mounts.
pub fn routes() -> Route {
    let api = OpenApiService::new(
        (
            SystemApi,
            WhoamiApi,
            OrgApi,
            TokensApi,
            AdLibraryApi,
            AdvertisersApi,
            CampaignsApi,
            FlightsApi,
            CreativesApi,
            CreativeTemplatesApi,
            AdsApi,
            SitesApi,
            ZonesApi,
            TaxonomyApi,
            DecisionsApi,
            ExplainApi,
        ),
        "knievel",
        env!("CARGO_PKG_VERSION"),
    )
    .server(
        ServerObject::new(crate::DEFAULT_OPENAPI_SERVER_URL)
            .description(crate::DEFAULT_OPENAPI_SERVER_DESCRIPTION),
    );
    let spec = api.spec_endpoint();

    Route::new()
        .nest("/", api)
        .at("/openapi.json", spec)
        // Admin-UI runtime config (Phase 7.4). Registered
        // BEFORE the Phase 7.11 `StaticFilesEndpoint` mount at
        // `/admin/` so a `config.json` inside the SPA bundle
        // can't shadow it. Public, unauthenticated, no secrets.
        .at("/admin/config.json", get(admin_ui::config_json))
        // Public event-tracking endpoints (Phase 3.25).
        // Unauthenticated; the HMAC signature in the URL is the
        // authorization (`API.md` § 4).
        .at("/e/i/:signed", get(crate::event_endpoints::impression))
        .at("/e/c/:signed", get(crate::event_endpoints::click))
}

/// Mount the admin SPA at `/admin/` from a static directory
/// when `cfg.admin_ui.static_dir` is set. `index.html`
/// fallback gives the SPA's client-side router (TanStack
/// Router) usable history routing — any unknown path under
/// `/admin/` resolves to `index.html` so a deep-link refresh
/// to `/admin/orgs/foo/projects/bar` doesn't 404.
///
/// Empty / unset `static_dir` → no mount; `/admin/*` returns
/// 404 (headless API mode). The same image runs both shapes
/// — Phase 7.11.
pub fn mount_admin_ui(route: Route, static_dir: Option<&str>) -> Route {
    match static_dir {
        Some(dir) if !dir.is_empty() => {
            tracing::info!(static_dir = %dir, "mounting admin UI at /admin/");
            route.nest(
                "/admin",
                StaticFilesEndpoint::new(dir)
                    .index_file("index.html")
                    .fallback_to_index(),
            )
        }
        _ => route,
    }
}

/// Format an anyhow error chain as a single string for structured
/// logging. Joins each cause in the chain with `: ` so operators
/// see the full path from the outermost context label to the root
/// cause in one log field.
///
/// Example: `anyhow::Error::msg("inner").context("middle").context("outer")`
/// produces `"outer: middle: inner"`.
pub fn format_error_chain(e: &anyhow::Error) -> String {
    e.chain()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

/// Extract the sqlstate code from an anyhow error chain if any
/// cause's Display matches the sqlx pattern `(code: XXXXX)`.
fn extract_sqlstate(e: &anyhow::Error) -> Option<String> {
    for cause in e.chain() {
        if let Some(code) = extract_sqlstate_from_str(&cause.to_string()) {
            return Some(code);
        }
    }
    None
}

fn extract_sqlstate_from_str(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    for prefix in &["(code: ", "sqlstate: ", "sqlstate "] {
        if let Some(pos) = lower.find(prefix) {
            let rest = &s[pos + prefix.len()..];
            let code: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            if code.len() >= 5 {
                return Some(code.to_uppercase());
            }
        }
    }
    None
}

fn operator_hint_for_connect_sqlstate(code: &str) -> Option<&'static str> {
    match code {
        "28000" => Some(
            "sqlstate 28000 — role or authentication method rejected. \
             Check KNIEVEL_DATABASE__URL and the pg_hba.conf entry for this role.",
        ),
        "28P01" => Some(
            "sqlstate 28P01 — password did not match. \
             KNIEVEL_DATABASE__URL's password likely doesn't match the secret value. \
             Check the database secret's password key.",
        ),
        "3D000" => Some(
            "sqlstate 3D000 — database does not exist. \
             Check the database name in KNIEVEL_DATABASE__URL. \
             The database must be created before knievel starts.",
        ),
        _ => None,
    }
}

fn operator_hint_for_migrate_sqlstate(code: &str) -> Option<&'static str> {
    match code {
        "42501" => Some(
            "sqlstate 42501 — the role lacks a required privilege. \
             The database role likely needs CREATE on the knievel schema \
             or CONNECT on the database. Pre-create the schema or grant the privilege.",
        ),
        _ => None,
    }
}

/// Build `AppState` with boot-hygiene fail-fast behaviour.
///
/// - `database.url` absent + `required = false` -> warn, return DB-less state.
/// - `database.url` absent + `required = true` -> fatal `Err`.
/// - Connect failure -> retry with exponential backoff; exhaust -> `Err`.
/// - Migration failure -> non-retryable `Err`.
///
/// Returns `Result<AppState>` so the test suite can assert `Err` without
/// actually invoking `process::exit`. The `exit(1)` lives in `main.rs`.
pub async fn build_state(cfg: &Config) -> Result<AppState> {
    let jwt_verifier = crate::auth::jwt::JwtVerifier::new(cfg.auth.jwt.issuers.clone());
    if jwt_verifier.is_enabled() {
        tracing::info!(
            issuer_count = jwt_verifier.policies().len(),
            "JWT bearer verification enabled"
        );
    }

    let base_state = AppState::new()
        .with_decisions(DecisionFlags {
            force_overrides_enabled: cfg.decisions.force_overrides_enabled,
        })
        .with_admin_ui(cfg.admin_ui.clone())
        .with_jwt_verifier(jwt_verifier)
        .with_image_store(Arc::new(InMemoryStore::default()));

    let Some(url) = &cfg.database.url else {
        if cfg.database.required {
            tracing::error!(
                "database.required = true but database.url is not set; \
                 knievel cannot start. Set KNIEVEL_DATABASE__URL or \
                 database.url in the config file."
            );
            return Err(anyhow!(
                "database.required = true but database.url is not configured"
            ));
        }
        tracing::warn!("running without a database; project-scoped endpoints will 503");
        tracing::info!(
            db = "none",
            schema = "n/a",
            jwt_issuers = cfg.auth.jwt.issuers.len(),
            image_store = "in_memory",
            "knievel boot ready"
        );
        return Ok(base_state);
    };

    let retry = &cfg.database.connect_retry;
    let mut last_err: anyhow::Error = anyhow!("no attempt made");
    let mut backoff_ms = retry.initial_backoff_ms;

    for attempt in 1..=retry.attempts {
        match connect_pool(url, cfg.database.max_connections).await {
            Ok(pool) => {
                tracing::info!(attempt, "connected to Postgres");
                return finish_state_with_pool(base_state, pool, cfg).await;
            }
            Err(e) => {
                let chain = format_error_chain(&e);
                if attempt < retry.attempts {
                    tracing::warn!(
                        attempt,
                        max_attempts = retry.attempts,
                        backoff_ms,
                        error_chain = %chain,
                        "DB connect failed; will retry"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(retry.max_backoff_ms);
                }
                last_err = e;
            }
        }
    }

    let chain = format_error_chain(&last_err);
    let hint = extract_sqlstate(&last_err)
        .and_then(|code| operator_hint_for_connect_sqlstate(&code))
        .map(|h| format!("  hint: {h}"))
        .unwrap_or_default();
    tracing::error!(
        attempts = retry.attempts,
        error_chain = %chain,
        "DB connect exhausted retries.{hint}"
    );
    Err(last_err.context("DB connect exhausted retries"))
}

async fn finish_state_with_pool(
    base_state: AppState,
    pool: sqlx::PgPool,
    cfg: &Config,
) -> Result<AppState> {
    let schema_status;
    if cfg.database.auto_migrate {
        match crate::migrate::run(&pool).await {
            Ok(()) => {
                tracing::info!("auto_migrate: migrations applied");
                schema_status = "migrated";
            }
            Err(e) => {
                let chain = format_error_chain(&e);
                let hint = extract_sqlstate(&e)
                    .and_then(|code| operator_hint_for_migrate_sqlstate(&code))
                    .map(|h| format!("  hint: {h}"))
                    .unwrap_or_default();
                tracing::error!(error_chain = %chain, "auto_migrate failed.{hint}");
                return Err(e.context("auto_migrate failed"));
            }
        }
    } else {
        schema_status = "skipped";
    }

    let (sender, _flusher) = events::spawn(pool.clone(), cfg.events.channel_capacity);
    let leader_handle = LeaderHandle::new();
    let _leader_task = leader::spawn(pool.clone(), leader_handle.clone());
    let _partition_task = partitions::spawn(
        pool.clone(),
        leader_handle.clone(),
        cfg.partitions.retention_days,
    );
    let _rollup_task = rollup::spawn(pool.clone(), leader_handle.clone());

    tracing::info!(
        db = "connected",
        schema = schema_status,
        jwt_issuers = cfg.auth.jwt.issuers.len(),
        image_store = "in_memory",
        events = "on",
        leader = "follower",
        "knievel boot ready"
    );

    Ok(base_state
        .with_db(pool)
        .with_events(sender)
        .with_leader(leader_handle))
}

/// Connect with the same `after_connect` recipe `testlib` uses
/// (`SET search_path TO knievel, public`) so the `_sqlx_migrations`
/// tracking table lands inside the `knievel` schema rather than
/// `public`. Mirrors MIGRATION_RX.md "One-time provisioning"'s
/// `ALTER ROLE knievel_app SET search_path = ...`.
async fn connect_pool(url: &str, max_connections: u32) -> Result<sqlx::PgPool> {
    use sqlx::postgres::PgPoolOptions;
    PgPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET search_path TO knievel, public")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .map_err(|e| anyhow!("connect: {e}"))
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to install SIGTERM handler");
            return;
        }
    };
    let mut int = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to install SIGINT handler");
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => tracing::info!("received SIGTERM"),
        _ = int.recv()  => tracing::info!("received SIGINT"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_error_chain_joins_causes() {
        let e = anyhow::anyhow!("inner").context("middle").context("outer");
        assert_eq!(format_error_chain(&e), "outer: middle: inner");
    }

    #[test]
    fn format_error_chain_single_error() {
        let e = anyhow::anyhow!("only error");
        assert_eq!(format_error_chain(&e), "only error");
    }

    #[test]
    fn extract_sqlstate_from_str_finds_code_prefix() {
        let s = "error returned from database: permission denied (code: 42501)";
        assert_eq!(extract_sqlstate_from_str(s), Some("42501".to_string()));
    }

    #[test]
    fn extract_sqlstate_from_str_missing_returns_none() {
        let s = "connection timed out after 30s";
        assert_eq!(extract_sqlstate_from_str(s), None);
    }

    #[test]
    fn operator_hint_known_codes() {
        assert!(operator_hint_for_connect_sqlstate("28P01").is_some());
        assert!(operator_hint_for_connect_sqlstate("28000").is_some());
        assert!(operator_hint_for_connect_sqlstate("3D000").is_some());
        assert!(operator_hint_for_migrate_sqlstate("42501").is_some());
    }

    #[test]
    fn operator_hint_unknown_code_returns_none() {
        assert!(operator_hint_for_connect_sqlstate("00000").is_none());
        assert!(operator_hint_for_migrate_sqlstate("00000").is_none());
    }

    /// Bogus URL + required=true must return Err without blocking on
    /// retries (attempts=1, backoff=0).
    #[tokio::test]
    async fn build_state_bogus_url_returns_err() {
        use crate::config::{ConnectRetryConfig, DatabaseConfig};
        let cfg = Config {
            database: DatabaseConfig {
                url: Some("postgres://bad:bad@127.0.0.1:1/no_such_db".into()),
                required: true,
                auto_migrate: false,
                connect_retry: ConnectRetryConfig {
                    attempts: 1,
                    initial_backoff_ms: 0,
                    max_backoff_ms: 0,
                },
                ..DatabaseConfig::default()
            },
            ..Config::default()
        };
        let result = build_state(&cfg).await;
        assert!(result.is_err(), "expected Err from bogus DB url");
    }

    /// required=false + no URL -> Ok with no pool.
    #[tokio::test]
    async fn build_state_no_url_required_false_ok() {
        let cfg = Config::default(); // required=false, url=None
        let result = build_state(&cfg).await;
        assert!(
            result.is_ok(),
            "expected Ok for no-DB required=false config"
        );
        assert!(result.unwrap().db.is_none());
    }

    /// required=true + no URL -> Err.
    #[tokio::test]
    async fn build_state_no_url_required_true_err() {
        use crate::config::DatabaseConfig;
        let cfg = Config {
            database: DatabaseConfig {
                required: true,
                url: None,
                ..DatabaseConfig::default()
            },
            ..Config::default()
        };
        let result = build_state(&cfg).await;
        assert!(
            result.is_err(),
            "expected Err when required=true but url unset"
        );
    }
}
