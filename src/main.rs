use knievel::{config, observability, server};

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

    // Phase 5.10: server::run propagates fatal boot errors (bad DB
    // config, exhausted retries, migration failure). We exit with
    // code 1 so kubelet sees the container die and enters
    // CrashLoopBackOff — operators discover the misconfiguration
    // from the exit code and the error log, not from a perpetual
    // /readyz=503 pod that appears "Running."
    if let Err(e) = server::run(cfg).await {
        tracing::error!(
            error_chain = %server::format_error_chain(&e),
            "knievel exited with a fatal error"
        );
        std::process::exit(1);
    }

    Ok(())
}
