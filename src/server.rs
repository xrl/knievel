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
use poem::get;
use poem::listener::TcpListener;
use poem::{EndpointExt, Route, Server};
use poem_openapi::OpenApiService;

use crate::ads::AdsApi;
use crate::advertisers::AdvertisersApi;
use crate::campaigns::CampaignsApi;
use crate::config::Config;
use crate::creative_templates::CreativeTemplatesApi;
use crate::creatives::CreativesApi;
use crate::decisions::{DecisionsApi, ExplainApi};
use crate::flights::FlightsApi;
use crate::orgs::OrgApi;
use crate::sites::SitesApi;
use crate::state::AppState;
use crate::system::SystemApi;
use crate::taxonomy::TaxonomyApi;
use crate::tokens::TokensApi;
use crate::zones::ZonesApi;

pub async fn run(cfg: Config) -> Result<()> {
    let addr = SocketAddr::from_str(&cfg.api.bind_addr)
        .with_context(|| format!("invalid api.bind_addr: {}", cfg.api.bind_addr))?;

    let state = build_state(&cfg).await;
    let app = routes().data(state);

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

/// Top-level routes. Phase 2 wires the system OpenAPI service
/// plus its `/openapi.json` spec endpoint; Phase 3+ adds the
/// management + decision OpenAPI services as additional
/// `OpenApiService` mounts.
pub fn routes() -> Route {
    let api = OpenApiService::new(
        (
            SystemApi,
            OrgApi,
            TokensApi,
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
    );
    let spec = api.spec_endpoint();

    Route::new()
        .nest("/", api)
        .at("/openapi.json", spec)
        // Public event-tracking endpoints (Phase 3.25).
        // Unauthenticated; the HMAC signature in the URL is the
        // authorization (`API.md` § 4).
        .at("/e/i/:signed", get(crate::event_endpoints::impression))
        .at("/e/c/:signed", get(crate::event_endpoints::click))
}

async fn build_state(cfg: &Config) -> AppState {
    let mut state = AppState::new();

    if let Some(url) = &cfg.database.url {
        match sqlx::PgPool::connect(url).await {
            Ok(pool) => {
                tracing::info!("connected to Postgres");
                state = state.with_db(pool);
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "DB connection failed at boot; /readyz will report 503"
                );
            }
        }
    } else {
        tracing::info!("no database.url configured; /readyz will report ok: no_db_configured");
    }

    state
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
