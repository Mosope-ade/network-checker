use serde::Deserialize;
use std::net::IpAddr;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration file not found: {0}. Copy config/monitor.example.toml to this path and explicitly configure gateway.address, internet.targets, and dns.hostname before starting daemon mode")]
    MissingFile(String),

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
        let contents = std::fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ConfigError::MissingFile(path.display().to_string())
            } else {
                ConfigError::ReadFile(error)
            }
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../config/monitor.example.toml");

    fn configured_example() -> String {
        EXAMPLE
            .replace("# address = \"YOUR_GATEWAY_IP\"", "address = \"::1\"")
            .replace("targets = []", "targets = [\"2001:db8::1\"]")
            .replace("hostname = \"\"", "hostname = \"example.invalid\"")
    }

    #[test]
    fn explicit_targets_are_required_and_preserved() {
        let config: Config = toml::from_str(&configured_example()).unwrap();
        config.validate().unwrap();
        assert_eq!(config.gateway.address, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(
            config.internet.targets,
            vec!["2001:db8::1".parse::<IpAddr>().unwrap()]
        );
        assert_eq!(config.dns.hostname, "example.invalid");
        let config: Config = toml::from_str(
            &configured_example().replace("targets = [\"2001:db8::1\"]", "targets = []"),
        )
        .unwrap();
        assert!(matches!(config.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn missing_config_returns_setup_instructions() {
        let error = Config::load(Path::new("/nonexistent-netcheck-test/config.toml")).unwrap_err();
        assert!(matches!(error, ConfigError::MissingFile(_)));
        assert!(error.to_string().contains("monitor.example.toml"));
        assert!(error.to_string().contains("gateway.address"));
    }

    #[test]
    fn shipped_config_cannot_monitor_without_explicit_target_configuration() {
        let parsed = toml::from_str::<Config>(include_str!("../config/monitor.example.toml"));
        assert!(
            parsed.is_err(),
            "The shipped configuration must not select monitoring targets for the user"
        );
    }
}
