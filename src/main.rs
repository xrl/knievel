mod config;

fn main() -> anyhow::Result<()> {
    // Phase 2.1: just prove the loader runs. Real bootstrap (poem
    // server, tracing, signals) lands in Phase 2.2 and 2.3.
    let cfg = config::load()?;
    println!(
        "knievel: bind_addr={} logging.level={} logging.format={}",
        cfg.api.bind_addr, cfg.logging.level, cfg.logging.format
    );
    Ok(())
}
