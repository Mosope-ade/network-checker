mod config;
mod monitor;
mod checks;
mod diagnostics;

use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

const DEFAULT_CONFIG_PATH: &str = "config/monitor.toml";

#[tokio::main]
async fn main() {
    // Initialise structured logging.
    // Respects RUST_LOG env var; defaults to INFO.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = PathBuf::from(DEFAULT_CONFIG_PATH);
    info!(path = %config_path.display(), "loading config");

    let config = match config::Config::load(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(%e, "failed to load config");
            std::process::exit(1);
        }
    };

    info!(
        gateway = %config.gateway.address,
        internet_targets = ?config.internet.targets,
        dns_hostname = %config.dns.hostname,
        system_alerts = config.alerts.system.enabled,
        telegram_alerts = config.alerts.telegram.enabled,
        "config loaded"
    );

    let config = Arc::new(config);
    monitor::run(config).await;
}
