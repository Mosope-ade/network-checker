use serde::Deserialize;
use std::net::IpAddr;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    ReadFile(#[from] std::io::Error),

    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("invalid configuration: {0}")]
    Validation(String),
}

/// Top-level configuration.
#[derive(Debug, Deserialize)]
pub struct Config {
    pub monitor: MonitorConfig,
    pub gateway: GatewayConfig,
    pub internet: InternetConfig,
    pub dns: DnsConfig,
    pub alerts: AlertsConfig,
}

/// Controls the monitoring loop behaviour.
#[derive(Debug, Deserialize)]
pub struct MonitorConfig {
    /// Seconds between check cycles.
    pub interval: u64,
    /// Consecutive failures required before declaring an outage.
    pub failure_threshold: u32,
    /// Consecutive successes required before declaring recovery.
    pub recovery_threshold: u32,
}

/// Gateway / router to monitor.
#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    /// IP address of the gateway to ping.
    pub address: IpAddr,
}

/// External targets for Internet connectivity checks.
#[derive(Debug, Deserialize)]
pub struct InternetConfig {
    /// List of external IP addresses to probe.
    pub targets: Vec<IpAddr>,
}

/// DNS resolution check.
#[derive(Debug, Deserialize)]
pub struct DnsConfig {
    /// Hostname to resolve.
    pub hostname: String,
}

/// Notification channel configuration.
#[derive(Debug, Deserialize)]
pub struct AlertsConfig {
    pub system: SystemAlertConfig,
    pub telegram: TelegramAlertConfig,
}

/// Local system (D-Bus / notify-send) notifications.
#[derive(Debug, Deserialize)]
pub struct SystemAlertConfig {
    pub enabled: bool,
}

/// Telegram Bot API notifications.
#[derive(Debug, Deserialize)]
pub struct TelegramAlertConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub chat_id: String,
}

impl Config {
    /// Load and validate configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.monitor.interval == 0 {
            return Err(ConfigError::Validation(
                "monitor.interval must be greater than 0".into(),
            ));
        }

        if self.monitor.failure_threshold == 0 {
            return Err(ConfigError::Validation(
                "monitor.failure_threshold must be greater than 0".into(),
            ));
        }

        if self.monitor.recovery_threshold == 0 {
            return Err(ConfigError::Validation(
                "monitor.recovery_threshold must be greater than 0".into(),
            ));
        }

        if self.internet.targets.is_empty() {
            return Err(ConfigError::Validation(
                "internet.targets must contain at least one address".into(),
            ));
        }

        if self.dns.hostname.is_empty() {
            return Err(ConfigError::Validation(
                "dns.hostname must not be empty".into(),
            ));
        }

        if self.alerts.telegram.enabled
            && (self.alerts.telegram.bot_token.is_empty()
                || self.alerts.telegram.chat_id.is_empty())
        {
            return Err(ConfigError::Validation(
                "alerts.telegram is enabled but bot_token or chat_id is empty".into(),
            ));
        }

        Ok(())
    }
}
