use std::sync::Arc;
use tokio::time;
use tracing::{debug, info, warn};
use chrono::Local;

use crate::config::Config;
use crate::checks::gateway;
use crate::checks::dns;
use crate::diagnostics;

#[derive(Default)]
struct ThresholdTracker {
    consecutive_failures: u32,
    consecutive_successes: u32,
    down: bool,
}

impl ThresholdTracker {
    /// Returns true once a failure threshold is reached.
    fn record_failure(&mut self, threshold: u32) -> bool {
        self.consecutive_successes = 0;
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        if !self.down && self.consecutive_failures >= threshold {
            self.down = true;
            true
        } else {
            false
        }
    }

    /// Returns true once a down check has met its recovery threshold.
    fn record_success(&mut self, threshold: u32) -> bool {
        self.consecutive_failures = 0;

        if !self.down {
            return false;
        }

        self.consecutive_successes = self.consecutive_successes.saturating_add(1);
        if self.consecutive_successes >= threshold {
            self.down = false;
            self.consecutive_successes = 0;
            true
        } else {
            false
        }
    }
}

/// Runs the monitoring loop at the configured interval.
pub async fn run(config: Arc<Config>) {
    let interval_secs = config.monitor.interval;
    let mut interval = time::interval(time::Duration::from_secs(interval_secs));
    let mut cycle: u64 = 0;
    let mut gateway_state = ThresholdTracker::default();
    let mut internet_state = ThresholdTracker::default();
    let mut dns_state = ThresholdTracker::default();

    info!(
        gateway = %config.gateway.address,
        interval = interval_secs,
        failure_threshold = config.monitor.failure_threshold,
        recovery_threshold = config.monitor.recovery_threshold,
        "starting monitor"
    );

    loop {
        interval.tick().await;
        cycle += 1;

        let now = Local::now().format("%H:%M:%S").to_string();

        match gateway::ping_gateway(config.gateway.address).await {
            Ok(duration) => {
                if gateway_state.record_success(config.monitor.recovery_threshold) {
                    info!(cycle, time = %now, "🟢 Gateway recovered after stable checks");
                } else if gateway_state.down {
                    info!(
                        cycle,
                        time = %now,
                        consecutive_successes = gateway_state.consecutive_successes,
                        threshold = config.monitor.recovery_threshold,
                        "Gateway responding; confirming recovery"
                    );
                } else {
                    info!(cycle, time = %now, latency_ms = ?duration.as_millis(), "✓ Gateway reachable");
                }
                
                // Phase 4: Internet connectivity
                let internet = crate::checks::internet::check_internet(&config.internet.targets).await;
                
                if internet.reachable {
                    if internet_state.record_success(config.monitor.recovery_threshold) {
                        info!(cycle, time = %now, "🟢 Internet connectivity recovered after stable checks");
                    } else if internet_state.down {
                        info!(
                            cycle,
                            time = %now,
                            consecutive_successes = internet_state.consecutive_successes,
                            threshold = config.monitor.recovery_threshold,
                            "Internet responding; confirming recovery"
                        );
                    } else {
                        info!(
                            cycle,
                            time = %now,
                            reachable = internet.targets_reachable,
                            total = internet.targets_tested,
                            "✓ Internet reachable"
                        );
                    }

                    // Phase 5: DNS is meaningful only after external IP connectivity succeeds.
                    match dns::check_dns(&config.dns.hostname).await {
                        Ok(duration) => {
                            if dns_state.record_success(config.monitor.recovery_threshold) {
                                info!(cycle, time = %now, hostname = %config.dns.hostname, "🟢 DNS recovered after stable checks");
                            } else if dns_state.down {
                                info!(
                                    cycle,
                                    time = %now,
                                    hostname = %config.dns.hostname,
                                    consecutive_successes = dns_state.consecutive_successes,
                                    threshold = config.monitor.recovery_threshold,
                                    "DNS responding; confirming recovery"
                                );
                            } else {
                                info!(
                                    cycle,
                                    time = %now,
                                    hostname = %config.dns.hostname,
                                    latency_ms = duration.as_millis(),
                                    "✓ DNS resolution succeeded"
                                );
                            }
                        }
                        Err(error) => {
                            if dns_state.record_failure(config.monitor.failure_threshold) {
                                warn!(
                                    cycle,
                                    time = %now,
                                    hostname = %config.dns.hostname,
                                    error = %error,
                                    "🟡 DNS outage confirmed (Internet connectivity is available)"
                                );
                            } else if !dns_state.down {
                                debug!(
                                    cycle,
                                    time = %now,
                                    hostname = %config.dns.hostname,
                                    error = %error,
                                    consecutive_failures = dns_state.consecutive_failures,
                                    threshold = config.monitor.failure_threshold,
                                    "DNS check failed; awaiting confirmation"
                                );
                            }
                        }
                    }
                } else {
                    if internet_state.record_failure(config.monitor.failure_threshold) {
                        warn!(
                            cycle,
                            time = %now,
                            tested = internet.targets_tested,
                            "🟠 Internet outage confirmed (Gateway is reachable, but external targets failed)"
                        );
                    } else if !internet_state.down {
                        debug!(
                            cycle,
                            time = %now,
                            tested = internet.targets_tested,
                            consecutive_failures = internet_state.consecutive_failures,
                            threshold = config.monitor.failure_threshold,
                            "Internet check failed; awaiting confirmation"
                        );
                    }
                }
            }
            Err(err) => {
                if gateway_state.record_failure(config.monitor.failure_threshold) {
                    warn!(cycle, time = %now, error = %err, "✗ Gateway outage confirmed");

                    // Run local diagnostics when the gateway outage is confirmed.
                    let report = diagnostics::diagnose_gateway(config.gateway.address);

                    warn!(
                        interface = ?report.state.interface_name,
                        interface_up = report.state.interface_up,
                        local_ip = ?report.state.local_ip,
                        route = report.state.route_exists,
                        arp = report.state.arp_resolved,
                        diagnosis = ?report.diagnosis,
                        "Local diagnostic report"
                    );
                } else if !gateway_state.down {
                    debug!(
                        cycle,
                        time = %now,
                        error = %err,
                        consecutive_failures = gateway_state.consecutive_failures,
                        threshold = config.monitor.failure_threshold,
                        "Gateway check failed; awaiting confirmation"
                    );
                }
            }
        }
    }
}
