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

    let state = build_state(&cfg).await;
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

async fn build_state(cfg: &Config) -> AppState {
    let jwt_verifier = crate::auth::jwt::JwtVerifier::new(cfg.auth.jwt.issuers.clone());
    if jwt_verifier.is_enabled() {
        tracing::info!(
            issuer_count = jwt_verifier.policies().len(),
            "JWT bearer verification enabled"
        );
    }

    let mut state = AppState::new()
        .with_decisions(DecisionFlags {
            force_overrides_enabled: cfg.decisions.force_overrides_enabled,
        })
        .with_admin_ui(cfg.admin_ui.clone())
        .with_jwt_verifier(jwt_verifier)
        // In-process store as the v0 default. The S3 / MinIO /
        // GCS-compat adapter is a 3.29 follow-up; both share the
        // `ImageStore` trait so flipping the backend is a config
        // change, not a code change.
        .with_image_store(Arc::new(InMemoryStore::default()));

    let Some(url) = &cfg.database.url else {
        tracing::info!("no database.url configured; /readyz will report ok: no_db_configured");
        return state;
    };

    let pool = match connect_pool(url, cfg.database.max_connections).await {
        Ok(p) => {
            tracing::info!("connected to Postgres");
            p
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "DB connection failed at boot; /readyz will report 503"
            );
            return state;
        }
    };

    if cfg.database.auto_migrate {
        match crate::migrate::run(&pool).await {
            Ok(()) => tracing::info!("auto_migrate: migrations applied"),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "auto_migrate failed; /readyz will report 503 until resolved"
                );
                return state;
            }
        }
    }

    // Events flusher — bounded mpsc + COPY drain
    // (Phase 3.21).
    let (sender, _flusher) = events::spawn(pool.clone(), cfg.events.channel_capacity);

    // Leader election + leader-gated maintenance loops
    // (Phases 3.22 / 3.23 / 3.24). The handles loop forever; we
    // hold them via `tokio::spawn` so the runtime keeps them
    // alive for the process lifetime, dropping them on shutdown
    // is harmless since each spawned future logs and exits.
    let leader_handle = LeaderHandle::new();
    let _leader_task = leader::spawn(pool.clone(), leader_handle.clone());
    let _partition_task = partitions::spawn(
        pool.clone(),
        leader_handle.clone(),
        cfg.partitions.retention_days,
    );
    let _rollup_task = rollup::spawn(pool.clone(), leader_handle.clone());

    state = state
        .with_db(pool)
        .with_events(sender)
        .with_leader(leader_handle);
    state
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
