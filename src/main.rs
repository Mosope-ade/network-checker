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
                        error!("Enter a whole-number port from 0 to 65535. Use 0 to choose an available port automatically.");
                        std::process::exit(2)
                    });
                } else if value.trim().is_empty() {
                    error!("The database path cannot be empty. Supply a writable file path after --database.");
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
        error!(details=%error, "{}", startup_message(error.as_ref(), port));
        std::process::exit(1);
    }
}

fn startup_message(error: &(dyn std::error::Error + 'static), port: u16) -> String {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return match error.kind() {
            std::io::ErrorKind::AddrInUse=>format!("Port {port} is already in use. Close the other NetCheck instance or choose a different port with --port."),
            std::io::ErrorKind::PermissionDenied=>"NetCheck does not have permission to use this port or database folder. Check access or choose another location with --database.".into(),
            std::io::ErrorKind::NotFound=>"The database location could not be found. Check the path supplied with --database.".into(),
            _=>"NetCheck could not start. Check the error details and the selected port and database path.".into(),
        };
    }
    if let Some(error) = error.downcast_ref::<rusqlite::Error>() {
        return match error {
            rusqlite::Error::ToSqlConversionFailure(source) if source.downcast_ref::<std::fs::TryLockError>().is_some_and(|error|matches!(error,std::fs::TryLockError::WouldBlock)) => "This database is already being monitored by another NetCheck process. Open that instance or stop it before starting again.".into(),
            rusqlite::Error::FromSqlConversionFailure(..)=>"Saved monitoring settings could not be read. Preserve the database and check the error details, or choose a different database with --database.".into(),
            _=>"NetCheck could not open its monitoring database. Check the path, file permissions, and error details. Preserve existing data before attempting repairs.".into(),
        };
    }
    "NetCheck could not start. Check the error details for the selected local port and database."
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_failures_explain_the_next_action() {
        let error = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        assert!(startup_message(&error, 8787).contains("--port"));
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(startup_message(&error, 8787).contains("permission"));
    }
}
