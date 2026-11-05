use crate::config::ConfigUpdate;

/// Every response line starts with this marker, so the host tool can tell responses apart
/// from ESP-IDF log lines that share the same USB serial console.
pub const RESPONSE_PREFIX: &str = "@FG ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionCommand {
    Ping,
    GetStatus,
    /// Partial update merged into the stored configuration
    SetConfig(Box<ConfigUpdate>),
    FactoryReset {
        confirmation: String,
    },
    TestLine,
    TriggerOta,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionResponse {
    Pong,
    Ok(String),
    Error(String),
    Status(String),
}

impl ProvisionResponse {
    /// Single-line wire format, e.g. `@FG OK Configuration saved`
    pub fn to_line(&self) -> String {
        let body = match self {
            ProvisionResponse::Pong => "PONG".to_string(),
            ProvisionResponse::Ok(msg) => format!("OK {msg}"),
            ProvisionResponse::Error(msg) => format!("ERROR {msg}"),
            ProvisionResponse::Status(json) => format!("STATUS {json}"),
        };
        // Keep responses on one line whatever the message contains
        let body: String = body
            .chars()
            .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
            .collect();
        format!("{RESPONSE_PREFIX}{body}")
    }
}

impl ProvisionCommand {
    pub const CONFIRMATION_TOKEN: &str = "CONFIRM_FACTORY_RESET";

    pub fn parse(line: &str) -> Self {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("PING") {
            ProvisionCommand::Ping
        } else if trimmed.eq_ignore_ascii_case("GET_STATUS") {
            ProvisionCommand::GetStatus
        } else if trimmed.eq_ignore_ascii_case("TEST_LINE") {
            ProvisionCommand::TestLine
        } else if trimmed.eq_ignore_ascii_case("TRIGGER_OTA") {
            ProvisionCommand::TriggerOta
        } else if let Some(rest) = trimmed.strip_prefix("SET_CONFIG ") {
            match serde_json::from_str::<ConfigUpdate>(rest.trim()) {
                Ok(update) => ProvisionCommand::SetConfig(Box::new(update)),
                // serde_json's message can quote the offending value, which may be a secret:
                // report only the error category and position
                Err(err) => ProvisionCommand::Unknown(format!(
                    "Invalid config JSON ({:?} error at column {})",
                    err.classify(),
                    err.column()
                )),
            }
        } else if let Some(rest) = trimmed.strip_prefix("FACTORY_RESET ") {
            ProvisionCommand::FactoryReset {
                confirmation: rest.trim().to_string(),
            }
        } else {
            ProvisionCommand::Unknown(format!(
                "Unknown command: {}",
                trimmed.split_whitespace().next().unwrap_or("")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_command_parsing() {
        assert_eq!(ProvisionCommand::parse("PING"), ProvisionCommand::Ping);
        assert_eq!(
            ProvisionCommand::parse("GET_STATUS"),
            ProvisionCommand::GetStatus
        );
        assert_eq!(
            ProvisionCommand::parse("TEST_LINE"),
            ProvisionCommand::TestLine
        );
        assert_eq!(
            ProvisionCommand::parse("FACTORY_RESET CONFIRM_FACTORY_RESET"),
            ProvisionCommand::FactoryReset {
                confirmation: "CONFIRM_FACTORY_RESET".to_string()
            }
        );
    }

    #[test]
    fn test_set_config_is_partial() {
        let cmd = ProvisionCommand::parse(r#"SET_CONFIG {"report_hour": 7}"#);
        assert_eq!(
            cmd,
            ProvisionCommand::SetConfig(Box::new(ConfigUpdate {
                report_hour: Some(7),
                ..ConfigUpdate::default()
            }))
        );
    }

    #[test]
    fn test_unknown_command_does_not_echo_arguments() {
        // A mistyped command line may contain a secret; only the command word is echoed
        assert_eq!(
            ProvisionCommand::parse("SETCONFIG {\"wifi_password\":\"hunter22\"}"),
            ProvisionCommand::Unknown("Unknown command: SETCONFIG".to_string())
        );
    }

    #[test]
    fn test_response_lines_are_prefixed_single_lines() {
        assert_eq!(ProvisionResponse::Pong.to_line(), "@FG PONG");
        assert_eq!(
            ProvisionResponse::Error("bad\r\nthing".to_string()).to_line(),
            "@FG ERROR bad  thing"
        );
    }
}
