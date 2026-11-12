use std::net::IpAddr;
use std::time::Duration;
use surge_ping::{Client, Config, PingIdentifier, PingSequence};

/// Pings the specified IP address and returns the latency if successful.
pub async fn ping_gateway(ip: IpAddr) -> Result<Duration, String> {
    let config = match ip {
        IpAddr::V4(_) => Config::builder().kind(surge_ping::ICMP::V4).build(),
        IpAddr::V6(_) => Config::builder().kind(surge_ping::ICMP::V6).build(),
    };

    let client = Client::new(&config)
        .map_err(|e| format!("ICMP unavailable: {e}. Check ICMP socket permissions."))?;
    // We use a fixed identifier for simplicity in this check
    let mut pinger = client.pinger(ip, PingIdentifier(111)).await;

    let payload = [0; 56];

    // Timeout ping at 2 seconds
    match tokio::time::timeout(
        Duration::from_secs(2),
        pinger.ping(PingSequence(1), &payload),
    )
    .await
    {
        Ok(Ok((_, duration))) => Ok(duration),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Request timeout".to_string()),
    }
}
