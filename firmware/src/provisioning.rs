use crate::storage::StorageManager;
use crate::Event;
use fridge_guard_core::config::NonSecretStatus;
use fridge_guard_core::protocol::{ProvisionCommand, ProvisionResponse};
use log::{info, warn};
use std::io::{self, ErrorKind, Read, Write};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Longest accepted command line (SET_CONFIG with every field is well below this)
const MAX_LINE_BYTES: usize = 4096;
/// The ESP-IDF USB-Serial-JTAG console is non-blocking: poll it at this interval when idle
const STDIN_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Requests the main thread carries out on behalf of the USB provisioning protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbRequest {
    TestLine,
    TriggerOta,
    /// Restart so a new configuration (or a factory reset) takes effect
    Restart,
}

pub struct ProvisioningHandler {
    storage: Arc<Mutex<StorageManager>>,
}

impl ProvisioningHandler {
    pub fn new(storage: Arc<Mutex<StorageManager>>) -> Self {
        Self { storage }
    }

    /// Process a line received from USB serial and write the response line.
    /// Returns the request the main thread has to carry out, if any.
    pub fn handle_line(&self, line: &str, current_status: &NonSecretStatus) -> Option<UsbRequest> {
        let (response, request) = match ProvisionCommand::parse(line) {
            ProvisionCommand::Ping => (ProvisionResponse::Pong, None),
            ProvisionCommand::GetStatus => {
                let json =
                    serde_json::to_string(current_status).unwrap_or_else(|_| "{}".to_string());
                (ProvisionResponse::Status(json), None)
            }
            ProvisionCommand::SetConfig(update) => self.apply_config(*update),
            ProvisionCommand::FactoryReset { confirmation } => {
                if confirmation == ProvisionCommand::CONFIRMATION_TOKEN {
                    let result = self.storage.lock().unwrap().factory_reset();
                    // Storage is sealed either way; only a restart brings it back
                    let response = match result {
                        Ok(()) => {
                            info!("Factory reset executed via USB provisioning");
                            ProvisionResponse::Ok("Factory reset done; restarting".to_string())
                        }
                        Err(e) => ProvisionResponse::Error(format!(
                            "Factory reset failed ({e:?}); restarting"
                        )),
                    };
                    (response, Some(UsbRequest::Restart))
                } else {
                    (
                        ProvisionResponse::Error(
                            "Destructive confirmation token missing or incorrect".to_string(),
                        ),
                        None,
                    )
                }
            }
            ProvisionCommand::TestLine => (
                ProvisionResponse::Ok("LINE test message queued".to_string()),
                Some(UsbRequest::TestLine),
            ),
            ProvisionCommand::TriggerOta => (
                ProvisionResponse::Ok("OTA check requested".to_string()),
                Some(UsbRequest::TriggerOta),
            ),
            ProvisionCommand::Unknown(msg) => (ProvisionResponse::Error(msg), None),
        };

        let mut stdout = io::stdout();
        let _ = writeln!(stdout, "{}", response.to_line());
        let _ = stdout.flush();
        request
    }

    /// Merge a partial update into the stored configuration, validate and save it.
    fn apply_config(
        &self,
        update: fridge_guard_core::config::ConfigUpdate,
    ) -> (ProvisionResponse, Option<UsbRequest>) {
        let mut store = self.storage.lock().unwrap();
        let mut config = store.load_config();
        config.apply_update(update);
        if let Err(e) = config.validate() {
            return (
                ProvisionResponse::Error(format!("Configuration validation failed: {e:?}")),
                None,
            );
        }
        match store.save_config(&config) {
            Ok(()) => {
                info!("New configuration saved to NVS via USB provisioning");
                (
                    ProvisionResponse::Ok("Configuration saved; restarting to apply".to_string()),
                    Some(UsbRequest::Restart),
                )
            }
            Err(e) => (
                ProvisionResponse::Error(format!("Failed to save to NVS: {e:?}")),
                None,
            ),
        }
    }
}

/// Serve the line-based provisioning protocol on the USB serial console.
pub fn spawn_provisioning_thread(
    storage: Arc<Mutex<StorageManager>>,
    status: Arc<Mutex<NonSecretStatus>>,
    events: Sender<Event>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("usb_provisioning".to_string())
        .stack_size(8192)
        .spawn(move || {
            let handler = ProvisioningHandler::new(storage);
            let mut stdin = io::stdin();
            let mut buf = [0u8; 128];
            let mut line = Vec::with_capacity(256);
            let mut overflowed = false;

            loop {
                let n = match stdin.read(&mut buf) {
                    Ok(0) => {
                        thread::sleep(STDIN_POLL_INTERVAL);
                        continue;
                    }
                    Ok(n) => n,
                    Err(e)
                        if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                    {
                        thread::sleep(STDIN_POLL_INTERVAL);
                        continue;
                    }
                    Err(e) => {
                        warn!("USB console read error: {e}");
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };

                for &byte in &buf[..n] {
                    match byte {
                        b'\n' => {
                            if overflowed {
                                let _ = writeln!(
                                    io::stdout(),
                                    "{}",
                                    ProvisionResponse::Error("Line too long".to_string()).to_line()
                                );
                            } else {
                                let text = String::from_utf8_lossy(&line);
                                let text = text.trim();
                                if !text.is_empty() {
                                    let snapshot = status.lock().unwrap().clone();
                                    if let Some(request) = handler.handle_line(text, &snapshot) {
                                        if events.send(Event::Usb(request)).is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                            line.clear();
                            overflowed = false;
                        }
                        b'\r' => {}
                        _ if line.len() < MAX_LINE_BYTES => line.push(byte),
                        _ => overflowed = true,
                    }
                }
            }
        })
}
