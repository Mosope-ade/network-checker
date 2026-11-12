use std::net::IpAddr;
use tokio::task::JoinSet;
use tracing::debug;

use crate::checks::gateway::ping_gateway;

#[derive(Debug)]
pub struct InternetStatus {
    pub reachable: bool,
    pub targets_tested: usize,
    pub targets_reachable: usize,
}

/// Tests connectivity to multiple external IPs concurrently.
/// If ANY of the targets respond, the Internet is considered reachable.
pub async fn check_internet(targets: &[IpAddr]) -> InternetStatus {
    if targets.is_empty() {
        return InternetStatus {
            reachable: false,
            targets_tested: 0,
            targets_reachable: 0,
        };
    }

    let mut join_set = JoinSet::new();

    for &target in targets {
        join_set.spawn(async move { (target, ping_gateway(target).await) });
    }

    let mut targets_reachable = 0;
    let targets_tested = targets.len();

    while let Some(res) = join_set.join_next().await {
        if let Ok((target, result)) = res {
            match result {
                Ok(latency) => {
                    debug!(target = %target, latency_ms = ?latency.as_millis(), "Internet target reachable");
                    targets_reachable += 1;
                }
                Err(e) => {
                    debug!(target = %target, error = %e, "Internet target unreachable");
                }
            }
        }
    }

    InternetStatus {
        reachable: targets_reachable > 0,
        targets_tested,
        targets_reachable,
    }
}
