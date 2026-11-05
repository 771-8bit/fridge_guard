use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use esp_idf_svc::http::client::{Configuration as HttpConfig, EspHttpConnection};
use log::info;
use serde::Serialize;
use std::fmt;
use std::time::Duration;

const LINE_PUSH_ENDPOINT: &str = "https://api.line.me/v2/bot/message/push";

#[derive(Serialize)]
struct LinePushRequest<'a> {
    to: &'a str,
    messages: Vec<LineMessage<'a>>,
}

#[derive(Serialize)]
struct LineMessage<'a> {
    #[serde(rename = "type")]
    msg_type: &'a str,
    text: &'a str,
}

#[derive(Debug)]
pub enum LineError {
    /// Token or destination user ID not provisioned
    NotProvisioned,
    /// Connection, TLS or I/O failure before an HTTP status was received
    Transport(String),
    /// 401/403: token invalid, revoked or lacking permission (fixed by re-provisioning)
    Auth(u16),
    /// 429: rate limited or the LINE plan's monthly message limit reached
    RateLimited,
    /// 5xx: LINE-side failure
    Server(u16),
    /// Other 4xx: this request itself is invalid and will never be accepted
    Rejected(u16),
}

impl LineError {
    /// Whether sending the same message again later can succeed
    pub fn is_retryable(&self) -> bool {
        !matches!(self, LineError::NotProvisioned | LineError::Rejected(_))
    }
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineError::NotProvisioned => write!(f, "LINE channel token or user ID not provisioned"),
            LineError::Transport(e) => write!(f, "LINE request failed: {e}"),
            LineError::Auth(status) => write!(
                f,
                "LINE authentication failed (HTTP {status}): token invalid, revoked, or permissions missing"
            ),
            LineError::RateLimited => write!(
                f,
                "LINE rate limit or monthly plan limit reached (HTTP 429)"
            ),
            LineError::Server(status) => write!(f, "LINE server error (HTTP {status})"),
            LineError::Rejected(status) => write!(f, "LINE rejected the message (HTTP {status})"),
        }
    }
}

pub struct LineClient;

impl LineClient {
    /// Send push message via LINE Messaging API.
    /// NEVER logs the channel access token or the destination user ID.
    pub fn send_push(channel_token: &str, user_id: &str, text: &str) -> Result<(), LineError> {
        if channel_token.is_empty() || user_id.is_empty() {
            return Err(LineError::NotProvisioned);
        }

        let payload = LinePushRequest {
            to: user_id,
            messages: vec![LineMessage {
                msg_type: "text",
                text,
            }],
        };

        let body = serde_json::to_string(&payload)
            .map_err(|e| LineError::Transport(format!("payload serialization: {e}")))?;

        let http_config = HttpConfig {
            timeout: Some(Duration::from_secs(15)),
            use_global_ca_store: true,
            crt_bundle_attach: Some(esp_idf_sys::esp_crt_bundle_attach),
            ..Default::default()
        };

        let connection = EspHttpConnection::new(&http_config)
            .map_err(|e| LineError::Transport(format!("HTTP client: {e:?}")))?;
        let mut client = HttpClient::wrap(connection);

        let auth_header = format!("Bearer {channel_token}");
        let content_length = body.len().to_string();
        let headers = [
            ("Content-Type", "application/json"),
            ("Content-Length", content_length.as_str()),
            ("Authorization", auth_header.as_str()),
        ];

        let mut request = client
            .request(Method::Post, LINE_PUSH_ENDPOINT, &headers)
            .map_err(|e| LineError::Transport(format!("POST: {e:?}")))?;

        request
            .write(body.as_bytes())
            .map_err(|e| LineError::Transport(format!("request body: {e:?}")))?;

        let response = request
            .submit()
            .map_err(|e| LineError::Transport(format!("submit: {e:?}")))?;

        match response.status() {
            200..=299 => {
                info!("LINE message accepted by Messaging API");
                Ok(())
            }
            status @ (401 | 403) => Err(LineError::Auth(status)),
            429 => Err(LineError::RateLimited),
            status @ 500..=599 => Err(LineError::Server(status)),
            status => Err(LineError::Rejected(status)),
        }
    }
}
