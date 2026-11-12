mod checks;
mod config;
mod dashboard;
mod diagnostics;
mod monitor;
mod store;
mod tracker;

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

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("NetCheck — concurrent ping monitor\n\nUsage: netcheck [--port 8787] [--database data/netcheck.db]\n       netcheck --daemon [--config config/monitor.toml]\n\nOpen http://127.0.0.1:8787 for the GUI. Keep the process running to monitor.");
        return;
    }
    let mut port = 8787u16;
    let mut database = "data/netcheck.db".to_owned();
    let mut config_path = PathBuf::from(DEFAULT_CONFIG_PATH);
    let mut daemon = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--daemon" => daemon = true,
            "--port" | "--database" | "--config" => {
                let flag = &args[i];
                i += 1;
                let Some(value) = args.get(i) else {
                    error!("{flag} requires a value");
                    std::process::exit(2);
                };
                match flag.as_str() {
                    "--port" => {
                        port = value.parse().unwrap_or_else(|_| {
                            error!("Invalid port");
                            std::process::exit(2)
                        })
                    }
                    "--database" => database = value.clone(),
                    _ => config_path = PathBuf::from(value),
                }
            }
            other => {
                error!("Unknown argument: {other}. Use --help.");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if !daemon {
        if let Err(e) = dashboard::run(port, &database).await {
            error!(%e, "GUI server failed");
            std::process::exit(1);
        }
        return;
    }

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
