//! HTTP server bootstrap.
//!
//! Binds `poem` to `cfg.api.bind_addr`, installs SIGTERM/SIGINT
//! handlers, and runs with poem's graceful-shutdown helper. Drain
//! and total timeouts come from `ApiConfig` (defaults: 30 s drain,
//! 60 s total — `REQUIREMENTS.md` § 10.7).
//!
//! Handlers (`/healthz`, `/readyz`, `/version`, `/openapi.json`)
//! land in Phase 2.4–2.7 and are wired into `routes()` as they
//! arrive.

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use poem::listener::TcpListener;
use poem::{get, EndpointExt, Route, Server};

use crate::config::Config;
use crate::state::AppState;
use crate::system;

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

/// Routes wired so far. Each Phase 2.x task adds its endpoint
/// here; the helper is the single edit point for new top-level
/// system routes.
pub(crate) fn routes() -> Route {
    Route::new()
        .at("/healthz", get(system::healthz))
        .at("/readyz", get(system::readyz))
        .at("/version", get(system::version))
}

/// Build initial `AppState`. Today: maybe-connect to Postgres;
/// failure is non-fatal during Phase 2 bootstrap (the server
/// still starts and `/readyz` reports 503 with `db_unreachable`).
/// Phase 3+ makes a working DB connection mandatory at boot.
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
        tracing::info!(
            "no database.url configured; /readyz will report ok: no_db_configured"
        );
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
