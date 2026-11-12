use std::time::Duration;
use tokio::net::lookup_host;
use tokio::time::timeout;

/// Tests DNS resolution for a given hostname.
/// Returns the time it took to resolve, or an error if it failed.
pub async fn check_dns(hostname: &str) -> Result<Duration, String> {
    // lookup_host requires a port, so we just append a dummy port
    let host_and_port = format!("{}:80", hostname);
    let start = std::time::Instant::now();

    // Timeout the DNS resolution after 2 seconds
    let lookup_future = lookup_host(host_and_port);
    match timeout(Duration::from_secs(2), lookup_future).await {
        Ok(Ok(mut ips)) => {
            if ips.next().is_some() {
                Ok(start.elapsed())
            } else {
                Err("Resolved to no IP addresses".into())
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("DNS resolution timeout".into()),
    }
}
