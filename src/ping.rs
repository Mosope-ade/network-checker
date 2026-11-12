use std::{
    net::IpAddr,
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};
use surge_ping::{Client, Config, PingIdentifier, PingSequence};

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("ICMP unavailable: {0}. Check ICMP socket permissions.")]
    Unavailable(String),
    #[error("{0}")]
    NoReply(String),
    #[error("ICMP request timed out")]
    TimedOut,
}

pub async fn probe(ip: IpAddr) -> Result<f64, ProbeError> {
    static NEXT_ID: AtomicU16 = AtomicU16::new(0);
    let kind = match ip {
        IpAddr::V4(_) => surge_ping::ICMP::V4,
        IpAddr::V6(_) => surge_ping::ICMP::V6,
    };
    let client = Client::new(&Config::builder().kind(kind).build())
        .map_err(|e| ProbeError::Unavailable(e.to_string()))?;
    let id = NEXT_ID
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(std::process::id() as u16);
    let mut pinger = client.pinger(ip, PingIdentifier(id)).await;
    pinger.timeout(Duration::from_secs(2));
    match tokio::time::timeout(
        Duration::from_secs(2),
        pinger.ping(PingSequence(id), &[0; 56]),
    )
    .await
    {
        Ok(Ok((_, duration))) => Ok(duration.as_secs_f64() * 1000.),
        Ok(Err(surge_ping::SurgeError::IOError(error)))
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
            ) =>
        {
            Err(ProbeError::Unavailable(error.to_string()))
        }
        Ok(Err(surge_ping::SurgeError::Timeout { .. })) => Err(ProbeError::TimedOut),
        Ok(Err(error)) => Err(ProbeError::NoReply(error.to_string())),
        Err(_) => Err(ProbeError::TimedOut),
    }
}

impl ProbeError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => {
                "Local ICMP probes are unavailable. Check this machine's ping permissions."
            }
            Self::TimedOut => {
                "No reply within two seconds. Check the host, route, and whether ping is allowed."
            }
            Self::NoReply(_) => {
                "No ICMP reply. The host or route may be unavailable, or ping may be blocked."
            }
        }
    }
}
