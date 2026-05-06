mod config;
mod observability;
mod server;
mod system;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load()?;
    observability::init(&cfg)?;

    tracing::info!(
        bind_addr = %cfg.api.bind_addr,
        log_level = %cfg.logging.level,
        log_format = %cfg.logging.format,
        "knievel boot"
    );

    server::run(cfg).await
}
