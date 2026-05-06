mod config;
mod observability;

fn main() -> anyhow::Result<()> {
    let cfg = config::load()?;
    observability::init(&cfg)?;

    tracing::info!(
        bind_addr = %cfg.api.bind_addr,
        log_level = %cfg.logging.level,
        log_format = %cfg.logging.format,
        "knievel startup (Phase 2.2 — server bootstrap lands in 2.3)"
    );
    Ok(())
}
