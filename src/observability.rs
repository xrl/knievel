//! Observability bootstrap: tracing subscriber, OTel exporter
//! init, Sentry init.
//!
//! Phase 2.2 lands the tracing subscriber (JSON or compact). OTel
//! and Sentry are stubbed today: their `enabled` config flags are
//! honored but no exporter is wired. Real OTel + Sentry land
//! alongside the first calls that benefit from them in Phase 3+.
//!
//! Spec ref: `REQUIREMENTS.md` § 10.2, § 10.3, § 10.4.

use anyhow::{anyhow, Result};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::Config;

pub fn init(cfg: &Config) -> Result<()> {
    let filter = EnvFilter::try_new(&cfg.logging.level).map_err(|e| {
        anyhow!(
            "logging.level '{}' is not a valid EnvFilter directive: {e}",
            cfg.logging.level
        )
    })?;

    match cfg.logging.format.as_str() {
        "json" => {
            let layer = fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .flatten_event(true);
            tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow!("tracing init failed: {e}"))?;
        }
        "compact" | "text" => {
            let layer = fmt::layer().compact();
            tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow!("tracing init failed: {e}"))?;
        }
        other => {
            return Err(anyhow!(
                "unsupported logging.format: '{other}' (expected json | compact | text)"
            ))
        }
    }

    if cfg.tracing.otel.enabled {
        // Phase 3+ wires OTLP exporter via opentelemetry-otlp +
        // tracing-opentelemetry (REQUIREMENTS.md § 10.3).
        tracing::info!(
            endpoint = cfg.tracing.otel.endpoint.as_deref().unwrap_or("<unset>"),
            "OTel enabled in config; exporter wiring deferred to Phase 3+"
        );
    }

    if cfg.errors.sentry.enabled {
        // Phase 3+ wires sentry crate + sentry-tower middleware
        // (REQUIREMENTS.md § 10.4).
        tracing::info!(
            environment = cfg
                .errors
                .sentry
                .environment
                .as_deref()
                .unwrap_or("<unset>"),
            "Sentry enabled in config; SDK init deferred to Phase 3+"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_format() {
        let mut cfg = Config::default();
        cfg.logging.format = "yaml".into();
        let err = init(&cfg).unwrap_err();
        assert!(format!("{err:#}").contains("unsupported logging.format"));
    }

    #[test]
    fn rejects_invalid_filter_directive() {
        let mut cfg = Config::default();
        // `knievel=POTATO` parses as a directive but POTATO is not
        // a valid level — EnvFilter::try_new returns Err.
        cfg.logging.level = "knievel=POTATO".into();
        let err = init(&cfg).unwrap_err();
        assert!(
            format!("{err:#}").contains("EnvFilter directive"),
            "{err:#}"
        );
    }

    // We can't unit-test successful init without owning the global
    // dispatcher for the rest of the test process; it's exercised
    // by the binary at runtime and in the acceptance suite.
}
