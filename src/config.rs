// Many config fields are populated by the loader today but only
// read in later phases (tracing setup in 2.2, DB connection in 2.5,
// etc.). Allow the dead-code lint at module level until consumers
// land; remove when config.rs is fully wired.
#![allow(dead_code)]

//! Layered configuration loader.
//!
//! Precedence (later wins):
//!   1. Built-in defaults.
//!   2. `config.yaml` (path from `KNIEVEL_CONFIG`, default
//!      `/etc/knievel/config.yaml`).
//!   3. Env-var overrides under the `KNIEVEL_` prefix, with `__`
//!      as the path delimiter (e.g. `KNIEVEL_API__BIND_ADDR`
//!      overrides `api.bind_addr`).
//!
//! `${VAR}` and `${VAR:default}` interpolation is applied to the
//! raw `config.yaml` text *before* parse, so secrets injected as
//! env vars can be referenced inline. An unset `${VAR}` with no
//! default is a hard error at startup.
//!
//! Spec ref: `REQUIREMENTS.md` § 10.1.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use figment::providers::{Env, Format, Yaml};
use figment::Figment;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub errors: ErrorsConfig,
    #[serde(default)]
    pub events: EventsConfig,
    #[serde(default)]
    pub decisions: DecisionsConfig,
    #[serde(default)]
    pub partitions: PartitionsConfig,
    // Sections not yet typed are tolerated by serde via the
    // `default` attribute on the top-level struct; deeper typing
    // lands per-feature.
}

#[derive(Deserialize, Debug, Clone)]
pub struct ApiConfig {
    pub bind_addr: String,
    pub public_base_url: String,
    /// Graceful-shutdown drain budget. Per `REQUIREMENTS.md` § 10.7
    /// the default is 30 s; the total budget (drain + transports
    /// flush) is bounded by `shutdown_total_timeout_secs`.
    #[serde(default = "default_shutdown_drain")]
    pub shutdown_drain_timeout_secs: u64,
    #[serde(default = "default_shutdown_total")]
    pub shutdown_total_timeout_secs: u64,
    /// Origins permitted to make cross-origin requests against the
    /// API. Empty (default) disables the CORS middleware entirely —
    /// same-origin only. Each entry is a literal origin like
    /// `https://admin.example.com`; wildcards are not supported.
    /// Consumed by the CORS layer landing in Phase 7 (`UI.md`,
    /// "CORS"); parsed today so dev configs can declare
    /// `http://localhost:5173` ahead of the middleware install.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".into(),
            public_base_url: "http://localhost:8080".into(),
            shutdown_drain_timeout_secs: default_shutdown_drain(),
            shutdown_total_timeout_secs: default_shutdown_total(),
            allowed_origins: Vec::new(),
        }
    }
}

fn default_shutdown_drain() -> u64 {
    30
}
fn default_shutdown_total() -> u64 {
    60
}

#[derive(Deserialize, Debug, Clone)]
pub struct DatabaseConfig {
    /// Connection URL. None = "no DB available" — useful in tests
    /// and the bootstrap stage of Phase 2; later phases will
    /// require it.
    pub url: Option<String>,
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default)]
    pub auto_migrate: bool,
}

// Manual Default — `#[derive(Default)]` would default `schema` to
// "" (String::default()), bypassing the per-field serde defaults
// when the entire `database` section is missing from the input.
impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            schema: default_schema(),
            max_connections: default_max_connections(),
            auto_migrate: false,
        }
    }
}

fn default_schema() -> String {
    "knievel".into()
}
fn default_max_connections() -> u32 {
    8
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "json".into()
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TracingConfig {
    #[serde(default)]
    pub otel: OtelConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct OtelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ErrorsConfig {
    #[serde(default)]
    pub sentry: SentryConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SentryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub dsn: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct EventsConfig {
    /// Bounded mpsc capacity for the events channel. Saturation
    /// surfaces as `503 event_channel_saturated` on the decision
    /// endpoint (`API.md` § 4).
    #[serde(default = "default_events_capacity")]
    pub channel_capacity: usize,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            channel_capacity: default_events_capacity(),
        }
    }
}

fn default_events_capacity() -> usize {
    crate::events::DEFAULT_CAPACITY
}

#[derive(Deserialize, Debug, Clone)]
pub struct DecisionsConfig {
    /// Global kill switch for `force.*` debug overrides
    /// (`API.md` § 1). When false, every `force.*` request is
    /// rejected with `403 force_disabled` regardless of the
    /// per-project flag or the principal's role. Defaults to
    /// true so the per-project flag stays the authoritative
    /// gate.
    #[serde(default = "default_force_overrides_enabled")]
    pub force_overrides_enabled: bool,
}

impl Default for DecisionsConfig {
    fn default() -> Self {
        Self {
            force_overrides_enabled: default_force_overrides_enabled(),
        }
    }
}

fn default_force_overrides_enabled() -> bool {
    true
}

#[derive(Deserialize, Debug, Clone)]
pub struct PartitionsConfig {
    /// `events_raw` retention window in days
    /// (`REQUIREMENTS.md` § 7.4). Spec default is 30.
    #[serde(default = "default_retention_days")]
    pub retention_days: i64,
}

impl Default for PartitionsConfig {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
        }
    }
}

fn default_retention_days() -> i64 {
    crate::partitions::RETENTION_DAYS_DEFAULT
}

/// Load layered configuration. Resolves `${VAR}` interpolation in
/// the YAML file before parse; unset vars without a default are a
/// hard error.
pub fn load() -> Result<Config> {
    let path =
        std::env::var("KNIEVEL_CONFIG").unwrap_or_else(|_| "/etc/knievel/config.yaml".into());

    let mut figment = Figment::new();

    if Path::new(&path).exists() {
        let raw = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
        let resolved = interpolate_env(&raw)?;
        figment = figment.merge(Yaml::string(&resolved));
    }

    figment = figment.merge(Env::prefixed("KNIEVEL_").split("__"));

    figment.extract().context("loading knievel config")
}

/// Resolve `${VAR}` and `${VAR:default}` against the process
/// environment. Multiple unset references are reported in one
/// error so operators see the full picture in a single boot
/// failure.
pub(crate) fn interpolate_env(input: &str) -> Result<String> {
    use regex::Regex;
    let re = Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)(?::([^}]*))?\}").unwrap();

    let mut out = String::with_capacity(input.len());
    let mut errors: Vec<String> = Vec::new();
    let mut last = 0;
    for m in re.captures_iter(input) {
        let mat = m.get(0).unwrap();
        out.push_str(&input[last..mat.start()]);
        let var = m.get(1).unwrap().as_str();
        let default = m.get(2).map(|x| x.as_str());
        match std::env::var(var) {
            Ok(v) => out.push_str(&v),
            Err(_) => match default {
                Some(d) => out.push_str(d),
                None => errors.push(var.to_string()),
            },
        }
        last = mat.end();
    }
    out.push_str(&input[last..]);

    if !errors.is_empty() {
        return Err(anyhow!(
            "config interpolation: {} unresolved variable(s) without default: {}",
            errors.len(),
            errors.join(", ")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env<F: FnOnce()>(pairs: &[(&str, &str)], f: F) {
        for (k, v) in pairs {
            std::env::set_var(k, v);
        }
        f();
        for (k, _) in pairs {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn interpolate_basic_substitution() {
        with_env(&[("KNV_HOST", "db.example")], || {
            let out = interpolate_env("host: ${KNV_HOST}").unwrap();
            assert_eq!(out, "host: db.example");
        });
    }

    #[test]
    fn interpolate_default_when_unset() {
        let out = interpolate_env("host: ${KNV_NOT_SET:fallback}").unwrap();
        assert_eq!(out, "host: fallback");
    }

    #[test]
    fn interpolate_default_can_be_empty() {
        let out = interpolate_env("dsn: ${KNV_UNSET:}").unwrap();
        assert_eq!(out, "dsn: ");
    }

    #[test]
    fn interpolate_unset_with_no_default_errors() {
        let err = interpolate_env("host: ${KNV_REQUIRED}").unwrap_err();
        assert!(format!("{err:#}").contains("KNV_REQUIRED"));
    }

    #[test]
    fn interpolate_collects_all_unset_in_one_error() {
        let err = interpolate_env("a: ${KNV_A}\nb: ${KNV_B}").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("KNV_A") && msg.contains("KNV_B"));
    }

    #[test]
    fn defaults_when_no_file_or_env() {
        // Ensure no KNIEVEL_CONFIG and no overrides leak in.
        std::env::remove_var("KNIEVEL_CONFIG");
        // Use a path that absolutely doesn't exist.
        std::env::set_var("KNIEVEL_CONFIG", "/nonexistent/path/knievel.yaml");
        let cfg = load().unwrap();
        assert_eq!(cfg.api.bind_addr, "0.0.0.0:8080");
        assert!(cfg.api.allowed_origins.is_empty());
        assert_eq!(cfg.logging.level, "info");
        assert_eq!(cfg.logging.format, "json");
        assert_eq!(cfg.database.schema, "knievel");
        assert_eq!(cfg.database.max_connections, 8);
        assert!(cfg.database.url.is_none());
        std::env::remove_var("KNIEVEL_CONFIG");
    }

    #[test]
    fn allowed_origins_parses_from_yaml() {
        let yaml = r#"
api:
  bind_addr: 0.0.0.0:9000
  public_base_url: http://localhost:9000
  allowed_origins:
    - http://localhost:5173
    - https://admin.example.com
"#;
        let cfg: Config = Figment::new().merge(Yaml::string(yaml)).extract().unwrap();
        assert_eq!(
            cfg.api.allowed_origins,
            vec![
                "http://localhost:5173".to_string(),
                "https://admin.example.com".to_string(),
            ],
        );
    }
}
