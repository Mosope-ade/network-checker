mod dashboard;
mod ping;
mod store;
mod tracker;

use tracing::error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let mut port = 8787;
    let mut database = "data/netcheck.db".to_owned();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                println!("NetCheck — GUI ping monitor\n\nUsage: netcheck [--port 8787] [--database data/netcheck.db]\n\nOpen the displayed local URL and add monitoring targets through the UI.\nTargets are saved locally; no targets are preset or loaded from config files.");
                return;
            }
            "--port" | "--database" => {
                let Some(value) = args.next() else {
                    error!("{flag} requires a value");
                    std::process::exit(2);
                };
                if flag == "--port" {
                    port = value.parse().unwrap_or_else(|_| {
                        error!("Invalid port");
                        std::process::exit(2)
                    });
                } else if value.trim().is_empty() {
                    error!("Database path cannot be empty");
                    std::process::exit(2);
                } else {
                    database = value;
                }
            }
            "--daemon" | "--config" => {
                error!("Configuration-driven monitoring was retired. Start NetCheck without these flags and manage targets through the UI.");
                std::process::exit(2);
            }
            _ => {
                error!("Unknown argument: {flag}. Use --help.");
                std::process::exit(2);
            }
        }
    }
    if let Err(error) = dashboard::run(port, &database).await {
        error!(%error, "NetCheck failed");
        std::process::exit(1);
    }
}
