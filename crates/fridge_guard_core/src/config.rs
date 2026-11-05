use crate::maintenance::MaintenanceButtonConfig;
use serde::{Deserialize, Serialize};

/// Longest inactivity threshold accepted (7 days)
pub const MAX_INACTIVITY_THRESHOLD_HOURS: u32 = 7 * 24;
/// Longest door-open-too-long threshold accepted (24 hours)
pub const MAX_OPEN_TOO_LONG_MINUTES: u32 = 24 * 60;
/// Highest monthly LINE message cap accepted
pub const MAX_MONTHLY_MESSAGE_LIMIT: u32 = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub wifi_ssid: String,
    pub wifi_password: String,
    pub line_channel_token: String,
    pub line_user_id: String,
    pub report_hour: u8,
    pub report_minute: u8,
    /// 0 disables the inactivity alert
    pub inactivity_threshold_hours: u32,
    /// 0 disables the door-open-too-long alert
    pub open_too_long_minutes: u32,
    pub monthly_message_limit: u32,
    pub button_config: MaintenanceButtonConfig,
    pub ota_manifest_url: String,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            line_channel_token: String::new(),
            line_user_id: String::new(),
            report_hour: 20,
            report_minute: 0,
            inactivity_threshold_hours: 24,
            open_too_long_minutes: 30,
            monthly_message_limit: 120,
            button_config: MaintenanceButtonConfig::default(),
            ota_manifest_url:
                "https://raw.githubusercontent.com/771-8bit/fridge_guard/main/release/manifest.json"
                    .to_string(),
        }
    }
}

/// Partial configuration sent by the provisioning tool: absent fields keep their stored value,
/// so secrets never have to be re-entered (or read back) to change another setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigUpdate {
    pub wifi_ssid: Option<String>,
    pub wifi_password: Option<String>,
    pub line_channel_token: Option<String>,
    pub line_user_id: Option<String>,
    pub report_hour: Option<u8>,
    pub report_minute: Option<u8>,
    pub inactivity_threshold_hours: Option<u32>,
    pub open_too_long_minutes: Option<u32>,
    pub monthly_message_limit: Option<u32>,
    pub button_config: Option<MaintenanceButtonConfig>,
    pub ota_manifest_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValidationError {
    EmptyWifiSsid,
    WifiSsidTooLong(usize),
    /// WPA2/WPA3 passphrase must be 8..=63 bytes, or exactly 64 hex digits
    InvalidWifiPassword,
    EmptyLineToken,
    /// Token must be printable ASCII without whitespace (it is sent in an HTTP header)
    InvalidLineToken,
    InvalidLineUserId(String),
    InvalidReportTime {
        hour: u8,
        minute: u8,
    },
    InvalidInactivityThreshold(u32),
    InvalidOpenTooLongMinutes(u32),
    InvalidMonthlyLimit(u32),
    InsecureOtaUrl(String),
}

impl DeviceConfig {
    /// Merge a partial update into this configuration (validation is a separate step).
    pub fn apply_update(&mut self, update: ConfigUpdate) {
        if let Some(v) = update.wifi_ssid {
            self.wifi_ssid = v;
        }
        if let Some(v) = update.wifi_password {
            self.wifi_password = v;
        }
        if let Some(v) = update.line_channel_token {
            self.line_channel_token = v.trim().to_string();
        }
        if let Some(v) = update.line_user_id {
            self.line_user_id = v.trim().to_string();
        }
        if let Some(v) = update.report_hour {
            self.report_hour = v;
        }
        if let Some(v) = update.report_minute {
            self.report_minute = v;
        }
        if let Some(v) = update.inactivity_threshold_hours {
            self.inactivity_threshold_hours = v;
        }
        if let Some(v) = update.open_too_long_minutes {
            self.open_too_long_minutes = v;
        }
        if let Some(v) = update.monthly_message_limit {
            self.monthly_message_limit = v;
        }
        if let Some(v) = update.button_config {
            self.button_config = v;
        }
        if let Some(v) = update.ota_manifest_url {
            self.ota_manifest_url = v.trim().to_string();
        }
    }

    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.wifi_ssid.is_empty() {
            return Err(ConfigValidationError::EmptyWifiSsid);
        }
        if self.wifi_ssid.len() > 32 {
            return Err(ConfigValidationError::WifiSsidTooLong(self.wifi_ssid.len()));
        }
        let password = self.wifi_password.as_bytes();
        let is_passphrase = (8..=63).contains(&password.len());
        let is_raw_psk = password.len() == 64 && password.iter().all(u8::is_ascii_hexdigit);
        if !is_passphrase && !is_raw_psk {
            return Err(ConfigValidationError::InvalidWifiPassword);
        }
        if self.line_channel_token.is_empty() {
            return Err(ConfigValidationError::EmptyLineToken);
        }
        if self.line_channel_token.len() > 1024
            || !self
                .line_channel_token
                .bytes()
                .all(|b| b.is_ascii_graphic())
        {
            return Err(ConfigValidationError::InvalidLineToken);
        }
        if !is_valid_line_user_id(&self.line_user_id) {
            return Err(ConfigValidationError::InvalidLineUserId(
                self.line_user_id.clone(),
            ));
        }
        if self.report_hour > 23 || self.report_minute > 59 {
            return Err(ConfigValidationError::InvalidReportTime {
                hour: self.report_hour,
                minute: self.report_minute,
            });
        }
        if self.inactivity_threshold_hours > MAX_INACTIVITY_THRESHOLD_HOURS {
            return Err(ConfigValidationError::InvalidInactivityThreshold(
                self.inactivity_threshold_hours,
            ));
        }
        if self.open_too_long_minutes > MAX_OPEN_TOO_LONG_MINUTES {
            return Err(ConfigValidationError::InvalidOpenTooLongMinutes(
                self.open_too_long_minutes,
            ));
        }
        if self.monthly_message_limit == 0 || self.monthly_message_limit > MAX_MONTHLY_MESSAGE_LIMIT
        {
            return Err(ConfigValidationError::InvalidMonthlyLimit(
                self.monthly_message_limit,
            ));
        }
        if !self.ota_manifest_url.starts_with("https://")
            || self.ota_manifest_url.len() > 256
            || !self.ota_manifest_url.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(ConfigValidationError::InsecureOtaUrl(
                self.ota_manifest_url.clone(),
            ));
        }

        Ok(())
    }

    pub fn inactivity_alert_enabled(&self) -> bool {
        self.inactivity_threshold_hours > 0
    }

    pub fn open_too_long_alert_enabled(&self) -> bool {
        self.open_too_long_minutes > 0
    }
}

/// LINE user IDs are 'U' followed by 32 hex digits
pub fn is_valid_line_user_id(user_id: &str) -> bool {
    user_id.len() == 33
        && user_id.starts_with('U')
        && user_id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Diagnostic status structure that NEVER leaks tokens or passwords
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonSecretStatus {
    pub firmware_version: String,
    pub hardware_id: String,
    pub reset_reason: String,
    pub wifi_connected: bool,
    pub wifi_ssid_configured: bool,
    pub line_configured: bool,
    pub time_synchronized: bool,
    pub door_stable_state: String,
    pub daily_count: u32,
    pub last_activity_time_str: Option<String>,
    pub last_report_sent_date: Option<String>,
    pub pending_report_queued: bool,
    pub queued_messages: u32,
    pub line_consecutive_failures: u32,
    pub monthly_messages_sent: u32,
    pub monthly_messages_cap: u32,
    pub report_time: String,
    pub inactivity_threshold_hours: u32,
    pub open_too_long_minutes: u32,
    pub button_short_press: String,
    pub button_long_press: String,
    pub ota_manifest_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> DeviceConfig {
        DeviceConfig {
            wifi_ssid: "HomeWiFi".to_string(),
            wifi_password: "SecretPassword".to_string(), // secret-scan:allow (test dummy)
            line_channel_token: "valid_token".to_string(), // secret-scan:allow (test dummy)
            line_user_id: "U1234567890abcdef1234567890abcdef".to_string(),
            ..DeviceConfig::default()
        }
    }

    #[test]
    fn test_config_validation() {
        let mut config = valid_config();
        assert!(config.validate().is_ok());

        // Invalid user ID
        config.line_user_id = "invalid".to_string();
        assert!(matches!(
            config.validate(),
            Err(ConfigValidationError::InvalidLineUserId(_))
        ));
        config.line_user_id = "U1234567890abcdef1234567890abcdeZ".to_string();
        assert!(matches!(
            config.validate(),
            Err(ConfigValidationError::InvalidLineUserId(_))
        ));

        // Invalid hour
        config.line_user_id = "U1234567890abcdef1234567890abcdef".to_string();
        config.report_hour = 25;
        assert!(matches!(
            config.validate(),
            Err(ConfigValidationError::InvalidReportTime { .. })
        ));
    }

    #[test]
    fn test_wifi_and_token_validation() {
        let mut config = valid_config();
        config.wifi_ssid = "x".repeat(33);
        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::WifiSsidTooLong(33))
        );

        let mut config = valid_config();
        config.wifi_password = "short".to_string();
        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidWifiPassword)
        );
        config.wifi_password = "a".repeat(64);
        assert!(config.validate().is_ok()); // 64 hex digits = raw PSK
        config.wifi_password = "z".repeat(64);
        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidWifiPassword)
        );

        // A token with CR/LF or spaces could inject HTTP headers
        let mut config = valid_config();
        config.line_channel_token = "abc\r\nX-Injected: 1".to_string();
        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidLineToken)
        );
    }

    #[test]
    fn test_alert_thresholds_zero_disables_and_bounds() {
        let mut config = valid_config();
        config.inactivity_threshold_hours = 0;
        config.open_too_long_minutes = 0;
        assert!(config.validate().is_ok());
        assert!(!config.inactivity_alert_enabled());
        assert!(!config.open_too_long_alert_enabled());

        config.inactivity_threshold_hours = MAX_INACTIVITY_THRESHOLD_HOURS + 1;
        assert!(matches!(
            config.validate(),
            Err(ConfigValidationError::InvalidInactivityThreshold(_))
        ));
    }

    #[test]
    fn test_partial_update_keeps_other_fields() {
        let mut config = valid_config();
        let update: ConfigUpdate =
            serde_json::from_str(r#"{"wifi_ssid": "NewRouter", "report_hour": 21}"#).unwrap();
        config.apply_update(update);

        assert_eq!(config.wifi_ssid, "NewRouter");
        assert_eq!(config.report_hour, 21);
        // Secrets that were not sent are kept
        assert_eq!(config.wifi_password, "SecretPassword");
        assert_eq!(config.line_channel_token, "valid_token");
        assert!(config.validate().is_ok());

        // Typos are rejected instead of silently ignored
        assert!(serde_json::from_str::<ConfigUpdate>(r#"{"wifi_sid": "x"}"#).is_err());
    }
}
