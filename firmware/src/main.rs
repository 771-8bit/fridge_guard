mod hardware;
mod line;
mod ota;
mod provisioning;
mod storage;

use esp_idf_hal::modem::Modem;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SntpConf};
use esp_idf_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, EspWifi};
use fridge_guard_core::activity::{ActivityEvent, ActivityTracker};
use fridge_guard_core::config::{DeviceConfig, NonSecretStatus};
use fridge_guard_core::door::{DoorState, DoorTransition};
use fridge_guard_core::maintenance::{ButtonGesture, MaintenanceAction, MaintenanceDispatcher};
use fridge_guard_core::manifest::TARGET_HARDWARE_ID;
use fridge_guard_core::open_too_long::DoorOpenAlert;
use fridge_guard_core::quota::MonthlyQuota;
use fridge_guard_core::report::{DailyReportParams, ReportFormatter};
use fridge_guard_core::retry::{BackoffPolicy, RetryGate};
use fridge_guard_core::scheduler::{ReportScheduler, ReportSnapshot};
use fridge_guard_core::time::{jst_epoch_at, JstDateTime, MIN_VALID_EPOCH_SECS};
use hardware::HardwareManager;
use line::LineClient;
use log::{error, info, warn};
use ota::OtaManager;
use provisioning::UsbRequest;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use storage::StorageManager;

// Single source of truth: `version` in firmware/Cargo.toml (also embedded in the ESP-IDF app descriptor)
pub const FIRMWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

esp_idf_sys::esp_app_desc!();

/// First OTA manifest check after boot (gives Wi-Fi and SNTP time to come up)
const OTA_FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
/// Regular OTA manifest check interval
const OTA_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// Retry interval while a freshly installed image still awaits its self-test
const OTA_SELF_TEST_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// A freshly installed image that cannot reach the manifest server within this time is rolled back
const OTA_SELF_TEST_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// Housekeeping period of the main loop (Wi-Fi, alerts, reports, delivery, OTA)
const TICK_INTERVAL: Duration = Duration::from_secs(1);
/// Today's use count is written to NVS at most this often; alert latches are written at once
const ACTIVITY_SAVE_INTERVAL: Duration = Duration::from_secs(60);
/// Door events kept while the wall clock is not set yet (the oldest is dropped beyond this)
const MAX_UNTIMED_DOOR_EVENTS: usize = 256;
/// Alerts and maintenance messages waiting for delivery (the oldest is dropped beyond this)
const MAX_OUTBOX_MESSAGES: usize = 8;
/// Non-essential maintenance messages (tests, status) are dropped after this many failed attempts
const MAX_NON_ESSENTIAL_ATTEMPTS: u32 = 2;
/// A daily report delivered later than this after its scheduled time is labeled as delayed
const DELAYED_REPORT_GRACE_SECS: u64 = 10 * 60;
/// Gives the USB provisioning response time to reach the host before restarting
const RESTART_FLUSH_DELAY: Duration = Duration::from_millis(300);

/// Inputs to the main thread from the sensor and USB provisioning threads
pub enum Event {
    /// Door state after boot-time stabilization (never counted as a use)
    DoorInitialized {
        state: DoorState,
        at: Instant,
    },
    Door {
        transition: DoorTransition,
        at: Instant,
    },
    Button(ButtonGesture),
    Usb(UsbRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    InactivityAlert,
    InactivityRecovery,
    DoorOpenAlert,
    LineTest,
    StatusReport,
}

impl MessageKind {
    /// Essential messages may use the part of the monthly quota reserved for reports and alerts
    fn is_essential(self) -> bool {
        matches!(
            self,
            MessageKind::InactivityAlert
                | MessageKind::InactivityRecovery
                | MessageKind::DoorOpenAlert
        )
    }
}

struct OutgoingMessage {
    kind: MessageKind,
    text: String,
    attempts: u32,
}

enum Delivery {
    Sent,
    /// Will never be accepted (monthly cap, invalid request): give up on this message
    Dropped,
    /// Temporary failure: try again after the retry gate opens
    Retry,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("==================================================");
    info!("Starting Fridge Guard Firmware v{FIRMWARE_VERSION} on {TARGET_HARDWARE_ID}");
    info!("==================================================");

    // A freshly OTA-installed image stays unconfirmed until the self-test in the main loop passes
    let boot = Instant::now();
    let ota_pending_verify = OtaManager::is_running_image_pending_verify();
    if ota_pending_verify {
        info!("Running a newly installed OTA image; it will be confirmed after the post-update self-test");
    }

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs_default = EspDefaultNvsPartition::take()?;

    let storage = Arc::new(Mutex::new(StorageManager::new(nvs_default)?));
    let config = storage.lock().unwrap().load_config();
    if config.validate().is_err() {
        warn!("Device is not provisioned yet; configure it over USB with tools/provision.py");
    }

    let hw = HardwareManager::new(peripherals.pins.gpio2, peripherals.pins.gpio9)?;

    let wifi = if config.wifi_ssid.is_empty() {
        info!("No Wi-Fi credentials provisioned. Running offline until provisioned.");
        None
    } else {
        start_wifi(peripherals.modem, sys_loop.clone(), &config)
    };

    // SNTP client for UTC/JST time synchronization (keeps retrying until the network is up)
    let _sntp = if wifi.is_some() {
        match EspSntp::new(&SntpConf::default()) {
            Ok(sntp) => Some(sntp),
            Err(e) => {
                warn!("Failed to start SNTP: {e:?}");
                None
            }
        }
    } else {
        None
    };

    let (events_tx, events_rx) = mpsc::channel();
    let app = App::new(config, storage.clone(), wifi, boot, ota_pending_verify);
    hardware::spawn_sensor_thread(hw, events_tx.clone())?;
    provisioning::spawn_provisioning_thread(storage, app.status.clone(), events_tx)?;

    info!("Core tasks ready. Starting main loop...");
    app.run(events_rx)
}

/// Configure and start the Wi-Fi station; the main loop makes (and repeats) connection attempts.
fn start_wifi(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    config: &DeviceConfig,
) -> Option<EspWifi<'static>> {
    let mut client = ClientConfiguration {
        // Minimum accepted security; WPA2/WPA3-mixed and WPA3-only networks are accepted as well
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    };
    if client.ssid.push_str(&config.wifi_ssid).is_err()
        || client.password.push_str(&config.wifi_password).is_err()
    {
        error!("Stored Wi-Fi SSID or password exceeds Wi-Fi limits; re-provision the device");
        return None;
    }

    // No NVS for the driver: credentials live only in this firmware's namespace,
    // so a factory reset really removes them
    let mut wifi = match EspWifi::new(modem, sys_loop, None) {
        Ok(wifi) => wifi,
        Err(e) => {
            error!("Failed to initialize Wi-Fi: {e:?}");
            return None;
        }
    };
    if let Err(e) = wifi.set_configuration(&Configuration::Client(client)) {
        error!("Failed to set Wi-Fi configuration: {e:?}");
        return None;
    }
    if let Err(e) = wifi.start() {
        error!("Failed to start Wi-Fi: {e:?}");
        return None;
    }
    info!("Wi-Fi station started");
    Some(wifi)
}

/// Wall-clock time, or None while SNTP has not set the clock yet
fn wall_clock_epoch() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
        .filter(|&secs| secs >= MIN_VALID_EPOCH_SECS)
}

struct App {
    config: DeviceConfig,
    storage: Arc<Mutex<StorageManager>>,
    status: Arc<Mutex<NonSecretStatus>>,
    wifi: Option<EspWifi<'static>>,
    network_up: bool,
    wifi_retry: RetryGate,
    boot: Instant,
    /// Wall-clock time the device started watching the door (boot), known once SNTP has synced
    monitoring_start_epoch: Option<u64>,
    door_state: DoorState,
    /// Door events waiting for a valid wall clock before they can be counted
    untimed_door_events: VecDeque<(DoorTransition, Instant)>,
    activity: ActivityTracker,
    activity_dirty: bool,
    last_activity_save: Instant,
    open_alert: DoorOpenAlert,
    scheduler: ReportScheduler,
    quota: MonthlyQuota,
    dispatcher: MaintenanceDispatcher,
    outbox: VecDeque<OutgoingMessage>,
    line_retry: RetryGate,
    ota_pending_verify: bool,
    last_ota_check: Option<Instant>,
}

impl App {
    fn new(
        config: DeviceConfig,
        storage: Arc<Mutex<StorageManager>>,
        wifi: Option<EspWifi<'static>>,
        boot: Instant,
        ota_pending_verify: bool,
    ) -> Self {
        let inactivity_enabled = config.inactivity_alert_enabled();
        let mut activity = ActivityTracker::new(
            DoorState::Unknown,
            u64::from(config.inactivity_threshold_hours) * 3600,
            inactivity_enabled,
            inactivity_enabled,
            String::new(),
        );
        let open_alert = DoorOpenAlert::new(
            u64::from(config.open_too_long_minutes) * 60,
            config.open_too_long_alert_enabled(),
        );
        let mut scheduler = ReportScheduler::new(config.report_hour, config.report_minute);
        // Year/month are taken from the first message sent (or the restored state)
        let mut quota = MonthlyQuota::new(config.monthly_message_limit, 0, 0);

        {
            let store = storage.lock().unwrap();
            if let Some(state) = store.load_activity_state() {
                info!(
                    "Restored activity state (date {}, {} uses)",
                    state.tracking_date, state.use_count
                );
                activity.restore_state(state);
            }
            if let Some(state) = store.load_scheduler_state() {
                info!("Restored scheduler state from NVS");
                scheduler.restore_state(state);
            }
            if let Some(state) = store.load_quota_state() {
                info!("Restored quota state from NVS");
                quota.restore_state(state);
            }
        }

        let status = Arc::new(Mutex::new(NonSecretStatus {
            firmware_version: FIRMWARE_VERSION.to_string(),
            hardware_id: TARGET_HARDWARE_ID.to_string(),
            reset_reason: format!("{:?}", esp_idf_hal::reset::ResetReason::get()),
            wifi_connected: false,
            wifi_ssid_configured: !config.wifi_ssid.is_empty(),
            line_configured: !config.line_channel_token.is_empty()
                && !config.line_user_id.is_empty(),
            time_synchronized: false,
            door_stable_state: DoorState::Unknown.as_str_jp().to_string(),
            daily_count: 0,
            last_activity_time_str: None,
            last_report_sent_date: None,
            pending_report_queued: false,
            queued_messages: 0,
            line_consecutive_failures: 0,
            monthly_messages_sent: 0,
            monthly_messages_cap: config.monthly_message_limit,
            report_time: format!("{:02}:{:02}", config.report_hour, config.report_minute),
            inactivity_threshold_hours: config.inactivity_threshold_hours,
            open_too_long_minutes: config.open_too_long_minutes,
            button_short_press: config.button_config.short_press.as_str().to_string(),
            button_long_press: config.button_config.long_press.as_str().to_string(),
            ota_manifest_url: config.ota_manifest_url.clone(),
        }));

        Self {
            dispatcher: MaintenanceDispatcher::new(config.button_config.clone()),
            config,
            storage,
            status,
            wifi,
            network_up: false,
            wifi_retry: RetryGate::new(BackoffPolicy::new(10_000, 300_000, 2.0, u32::MAX)),
            boot,
            monitoring_start_epoch: None,
            door_state: DoorState::Unknown,
            untimed_door_events: VecDeque::new(),
            activity,
            activity_dirty: false,
            last_activity_save: Instant::now(),
            open_alert,
            scheduler,
            quota,
            outbox: VecDeque::new(),
            line_retry: RetryGate::new(BackoffPolicy::default_network_policy()),
            ota_pending_verify,
            last_ota_check: None,
        }
    }

    fn run(mut self, events: Receiver<Event>) -> ! {
        let mut next_tick = Instant::now();
        loop {
            match events.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
                Ok(event) => self.handle_event(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    error!("Sensor and provisioning threads stopped; restarting");
                    self.save_state();
                    restart();
                }
            }
            if Instant::now() >= next_tick {
                self.tick();
                next_tick = Instant::now() + TICK_INTERVAL;
            }
        }
    }

    fn now_ms(&self) -> u64 {
        self.boot.elapsed().as_millis() as u64
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::DoorInitialized { state, at } => {
                info!("Initial door state: {state:?}");
                let transition = DoorTransition {
                    from: DoorState::Unknown,
                    to: state,
                };
                self.on_door(transition, at);
            }
            Event::Door { transition, at } => {
                info!(
                    "Door transition detected: {:?} -> {:?}",
                    transition.from, transition.to
                );
                self.on_door(transition, at);
            }
            Event::Button(gesture) => {
                let action = self.dispatcher.dispatch(gesture);
                info!("Maintenance button {gesture:?} -> {action:?}");
                self.run_action(action);
            }
            Event::Usb(UsbRequest::TestLine) => self.run_action(MaintenanceAction::LineTest),
            Event::Usb(UsbRequest::TriggerOta) => self.run_action(MaintenanceAction::OtaCheck),
            Event::Usb(UsbRequest::Restart) => {
                info!("Restart requested over USB");
                self.save_state();
                thread::sleep(RESTART_FLUSH_DELAY);
                restart();
            }
        }
    }

    fn on_door(&mut self, transition: DoorTransition, at: Instant) {
        self.door_state = transition.to;
        // Open duration is relative, so it is timed on the monotonic clock (works before SNTP)
        let at_secs = at.saturating_duration_since(self.boot).as_secs();
        self.open_alert
            .on_door_state_changed(transition.to, at_secs);

        if self.untimed_door_events.len() >= MAX_UNTIMED_DOOR_EVENTS {
            warn!("Too many door events before time sync; dropping the oldest");
            self.untimed_door_events.pop_front();
        }
        self.untimed_door_events.push_back((transition, at));
        self.process_door_events();
    }

    /// Count queued door events once the wall clock is known: uses need real timestamps.
    fn process_door_events(&mut self) {
        let Some(now_epoch) = wall_clock_epoch() else {
            return;
        };
        while let Some((transition, at)) = self.untimed_door_events.pop_front() {
            let event_epoch = now_epoch.saturating_sub(at.elapsed().as_secs());
            self.rollover_if_new_day(event_epoch);
            match self.activity.on_door_transition(transition, event_epoch) {
                ActivityEvent::UsageDetected { total_today } => {
                    info!("Fridge use detected; {total_today} uses today");
                    self.activity_dirty = true;
                }
                ActivityEvent::RecoveredFromInactivity => {
                    info!("Activity resumed after an inactivity alert");
                    self.queue_message(
                        MessageKind::InactivityRecovery,
                        ReportFormatter::format_inactivity_recovery(FIRMWARE_VERSION),
                    );
                    self.save_activity();
                }
                _ => {}
            }
        }
    }

    fn rollover_if_new_day(&mut self, epoch: u64) {
        let date = JstDateTime::from_epoch(epoch).date_string();
        // "YYYY-MM-DD" strings order like dates; never roll back to an earlier day
        if date.as_str() > self.activity.tracking_date() {
            info!("Date rollover (JST): -> {date}");
            self.activity.rollover_day(date);
            self.save_activity();
        }
    }

    fn run_action(&mut self, action: MaintenanceAction) {
        match action {
            MaintenanceAction::LineTest => {
                let text = ReportFormatter::format_test_report(
                    self.door_state,
                    self.activity.use_count(),
                    FIRMWARE_VERSION,
                    self.network_up,
                );
                self.queue_message(MessageKind::LineTest, text);
            }
            MaintenanceAction::StatusNow => {
                let date = wall_clock_epoch()
                    .map(|epoch| JstDateTime::from_epoch(epoch).date_string())
                    .unwrap_or_else(|| "時刻未同期".to_string());
                let text = ReportFormatter::format_status_report(
                    &date,
                    self.door_state,
                    self.activity.use_count(),
                    FIRMWARE_VERSION,
                );
                self.queue_message(MessageKind::StatusReport, text);
            }
            MaintenanceAction::OtaCheck => self.run_ota_check(),
            MaintenanceAction::None => {}
        }
    }

    fn queue_message(&mut self, kind: MessageKind, text: String) {
        // One pending test/status message is enough, however often it is requested
        if !kind.is_essential() && self.outbox.iter().any(|m| m.kind == kind) {
            info!("{kind:?} message already queued");
            return;
        }
        if self.outbox.len() >= MAX_OUTBOX_MESSAGES {
            if let Some(dropped) = self.outbox.pop_front() {
                warn!("Outgoing message queue full; dropping {:?}", dropped.kind);
            }
        }
        info!("Queued {kind:?} message for LINE delivery");
        self.outbox.push_back(OutgoingMessage {
            kind,
            text,
            attempts: 0,
        });
    }

    fn tick(&mut self) {
        self.network_up = self.maintain_wifi();

        let wall = wall_clock_epoch();
        if let (Some(now_epoch), None) = (wall, self.monitoring_start_epoch) {
            let start = now_epoch.saturating_sub(self.boot.elapsed().as_secs());
            info!("Wall clock synchronized; door monitoring started at epoch {start}");
            self.monitoring_start_epoch = Some(start);
        }
        self.process_door_events();
        self.maintain_ota();

        if self.open_alert.check_alert(self.boot.elapsed().as_secs()) {
            warn!("Door open too long alert triggered");
            self.queue_message(
                MessageKind::DoorOpenAlert,
                ReportFormatter::format_door_open_too_long_warning(
                    self.config.open_too_long_minutes,
                    FIRMWARE_VERSION,
                ),
            );
        }

        if let (Some(now_epoch), Some(monitoring_start)) = (wall, self.monitoring_start_epoch) {
            self.rollover_if_new_day(now_epoch);

            if self.activity.check_inactivity(now_epoch, monitoring_start)
                == ActivityEvent::InactivityThresholdCrossed
            {
                warn!("Inactivity threshold crossed");
                self.queue_message(
                    MessageKind::InactivityAlert,
                    ReportFormatter::format_inactivity_warning(
                        self.config.inactivity_threshold_hours,
                        FIRMWARE_VERSION,
                    ),
                );
                self.save_activity();
            }

            let now = JstDateTime::from_epoch(now_epoch);
            let today = now.date_string();
            if self
                .scheduler
                .is_daily_report_due(&today, now.hour, now.minute)
            {
                if let Some(old) = self.scheduler.pending_report() {
                    warn!(
                        "Report for {} was never delivered; replaced by today's report",
                        old.date_str
                    );
                }
                let snapshot = ReportSnapshot::capture(
                    today,
                    self.activity.use_count(),
                    self.activity.last_activity_epoch_secs(),
                    now_epoch,
                    self.door_state,
                );
                info!("Daily report for {} captured", snapshot.date_str);
                self.scheduler.queue_pending_report(snapshot);
                self.save_scheduler();
            }
        }

        // Deliver messages whenever Wi-Fi is available; test/maintenance messages
        // can proceed even before SNTP sets the wall clock.
        if self.network_up {
            self.deliver(wall);
        }

        if self.activity_dirty && self.last_activity_save.elapsed() >= ACTIVITY_SAVE_INTERVAL {
            self.save_activity();
        }
        self.refresh_status(wall.is_some());
    }

    /// Keep the station connected: retry with growing intervals (10 s .. 5 min) while it is down.
    /// Returns true when the network is usable (associated and IP assigned).
    fn maintain_wifi(&mut self) -> bool {
        let now_ms = self.now_ms();
        let Some(wifi) = self.wifi.as_mut() else {
            return false;
        };
        if wifi.is_connected().unwrap_or(false) {
            if self.wifi_retry.consecutive_failures() > 0 {
                info!("Wi-Fi connected");
                self.wifi_retry.record_success();
            }
            return wifi.is_up().unwrap_or(false);
        }
        if self.wifi_retry.is_ready(now_ms) {
            let delay = self.wifi_retry.record_failure(now_ms);
            info!(
                "Wi-Fi not connected; connecting (next attempt in {} s if this fails)",
                delay / 1000
            );
            if let Err(e) = wifi.connect() {
                warn!("Wi-Fi connect request failed: {e:?}");
            }
        }
        false
    }

    /// Send at most one queued message per tick with priority ordering:
    /// 1. Essential alerts in outbox (InactivityAlert, Recovery, DoorOpenAlert)
    /// 2. Scheduled daily report (requires synchronized clock)
    /// 3. Non-essential maintenance messages (LineTest, StatusReport)
    ///
    /// Non-essential failures do NOT block higher-priority messages.
    fn deliver(&mut self, now_epoch: Option<u64>) {
        if !self.line_retry.is_ready(self.now_ms()) {
            return;
        }

        // Priority 1: Essential alerts in outbox
        if let Some(pos) = self.outbox.iter().position(|m| m.kind.is_essential()) {
            let (kind, text) = (self.outbox[pos].kind, self.outbox[pos].text.clone());
            match self.send(&text, true, now_epoch) {
                Delivery::Sent => {
                    info!("{kind:?} essential message delivered");
                    self.outbox.remove(pos);
                }
                Delivery::Dropped => {
                    warn!("{kind:?} essential message dropped");
                    self.outbox.remove(pos);
                }
                Delivery::Retry => {
                    self.outbox[pos].attempts += 1;
                }
            }
            return;
        }

        // Priority 2: Scheduled daily report (only when wall clock is valid)
        if let Some(epoch) = now_epoch {
            if let Some(snapshot) = self.scheduler.pending_report().cloned() {
                let scheduled_epoch = jst_epoch_at(
                    &snapshot.date_str,
                    self.config.report_hour,
                    self.config.report_minute,
                )
                .unwrap_or(epoch);
                let text = ReportFormatter::format_daily_report(DailyReportParams {
                    date_str: &snapshot.date_str,
                    use_count: snapshot.use_count,
                    last_use_time_str: snapshot.last_use_time_str.as_deref(),
                    elapsed_since_last_use_str: snapshot.elapsed_since_last_use_str.as_deref(),
                    door_state: snapshot.door_state,
                    firmware_version: FIRMWARE_VERSION,
                    is_delayed: epoch > scheduled_epoch + DELAYED_REPORT_GRACE_SECS,
                });
                match self.send(&text, true, Some(epoch)) {
                    Delivery::Sent => {
                        info!("Daily report for {} delivered", snapshot.date_str);
                        self.scheduler.record_successful_send(snapshot.date_str);
                        self.save_scheduler();
                    }
                    Delivery::Dropped => {
                        warn!("Daily report for {} dropped", snapshot.date_str);
                        self.scheduler.record_dropped(snapshot.date_str);
                        self.save_scheduler();
                    }
                    Delivery::Retry => {}
                }
                return;
            }
        }

        // Priority 3: Non-essential maintenance messages
        if let Some(pos) = self.outbox.iter().position(|m| !m.kind.is_essential()) {
            let (kind, text) = (self.outbox[pos].kind, self.outbox[pos].text.clone());
            match self.send(&text, false, now_epoch) {
                Delivery::Sent => {
                    info!("{kind:?} message delivered");
                    self.outbox.remove(pos);
                }
                Delivery::Dropped => {
                    warn!("{kind:?} message dropped");
                    self.outbox.remove(pos);
                }
                Delivery::Retry => {
                    self.outbox[pos].attempts += 1;
                    if self.outbox[pos].attempts >= MAX_NON_ESSENTIAL_ATTEMPTS {
                        warn!(
                            "{kind:?} message failed after {} attempts; dropping to prevent blocking",
                            self.outbox[pos].attempts
                        );
                        self.outbox.remove(pos);
                    }
                }
            }
        }
    }

    /// Every LINE push goes through here: monthly cap check, send, retry bookkeeping.
    fn send(&mut self, text: &str, essential: bool, now_epoch: Option<u64>) -> Delivery {
        let quota_check = match now_epoch {
            Some(epoch) => {
                let now = JstDateTime::from_epoch(epoch);
                self.quota.check_send(now.year, now.month, essential)
            }
            None => self.quota.check_send_current(essential),
        };

        if let Err(e) = quota_check {
            warn!("Monthly LINE message cap reached ({e:?})");
            return Delivery::Dropped;
        }

        match LineClient::send_push(
            &self.config.line_channel_token,
            &self.config.line_user_id,
            text,
        ) {
            Ok(()) => {
                self.line_retry.record_success();
                let _ = match now_epoch {
                    Some(epoch) => {
                        let now = JstDateTime::from_epoch(epoch);
                        self.quota.record_send(now.year, now.month, essential)
                    }
                    None => self.quota.record_send_current(essential),
                };
                self.save_quota();
                Delivery::Sent
            }
            Err(e) if e.is_retryable() => {
                let delay = self.line_retry.record_failure(self.now_ms());
                warn!("{e}; retrying in {} s", delay / 1000);
                Delivery::Retry
            }
            Err(e) => {
                error!("{e}");
                Delivery::Dropped
            }
        }
    }

    fn maintain_ota(&mut self) {
        if self.ota_pending_verify && self.boot.elapsed() >= OTA_SELF_TEST_DEADLINE {
            error!("Post-update self-test failed: manifest server unreachable. Rolling back...");
            self.save_state();
            error!("{}", OtaManager::rollback_and_reboot());
        }
        let due = match self.last_ota_check {
            None => self.boot.elapsed() >= OTA_FIRST_CHECK_DELAY,
            Some(t) if self.ota_pending_verify => t.elapsed() >= OTA_SELF_TEST_RETRY_INTERVAL,
            Some(t) => t.elapsed() >= OTA_CHECK_INTERVAL,
        };
        if self.network_up && due {
            self.run_ota_check();
        }
    }

    /// Check the OTA manifest, confirm a freshly installed image once the manifest server is
    /// reachable (post-update self-test), and install a newer signed release if one exists.
    fn run_ota_check(&mut self) {
        if !self.network_up {
            warn!("OTA check skipped: network not connected");
            return;
        }
        self.last_ota_check = Some(Instant::now());

        let update =
            match OtaManager::check_for_update(&self.config.ota_manifest_url, FIRMWARE_VERSION) {
                Ok(update) => update,
                Err(e) => {
                    warn!("OTA manifest check failed: {e}");
                    return;
                }
            };

        if self.ota_pending_verify {
            match OtaManager::mark_running_image_valid() {
                Ok(()) => {
                    info!("Post-update self-test passed; v{FIRMWARE_VERSION} confirmed");
                    self.ota_pending_verify = false;
                }
                Err(e) => {
                    error!("{e}");
                    return;
                }
            }
        }

        if let Some(manifest) = update {
            match OtaManager::perform_update(&manifest) {
                Ok(()) => {
                    info!("Restarting into v{}", manifest.version);
                    self.save_state();
                    restart();
                }
                Err(e) => error!("OTA update to v{} failed: {}", manifest.version, e),
            }
        }
    }

    fn save_activity(&mut self) {
        let state = self.activity.export_state();
        if let Err(e) = self.storage.lock().unwrap().save_activity_state(&state) {
            warn!("Failed to save activity state: {e:?}");
        }
        self.activity_dirty = false;
        self.last_activity_save = Instant::now();
    }

    fn save_scheduler(&mut self) {
        let state = self.scheduler.export_state();
        if let Err(e) = self.storage.lock().unwrap().save_scheduler_state(&state) {
            warn!("Failed to save scheduler state: {e:?}");
        }
    }

    fn save_quota(&mut self) {
        let state = self.quota.export_state();
        if let Err(e) = self.storage.lock().unwrap().save_quota_state(&state) {
            warn!("Failed to save quota state: {e:?}");
        }
    }

    /// Persist everything before a deliberate restart
    fn save_state(&mut self) {
        self.save_activity();
        self.save_scheduler();
        self.save_quota();
    }

    fn refresh_status(&self, time_synchronized: bool) {
        if let Ok(mut status) = self.status.lock() {
            status.wifi_connected = self.network_up;
            status.time_synchronized = time_synchronized;
            status.door_stable_state = self.door_state.as_str_jp().to_string();
            status.daily_count = self.activity.use_count();
            status.last_activity_time_str = self
                .activity
                .last_activity_epoch_secs()
                .map(|t| JstDateTime::from_epoch(t).time_string());
            status.last_report_sent_date = self.scheduler.last_sent_date().map(str::to_string);
            status.pending_report_queued = self.scheduler.pending_report().is_some();
            status.queued_messages = self.outbox.len() as u32;
            status.line_consecutive_failures = self.line_retry.consecutive_failures();
            status.monthly_messages_sent = self.quota.sent_count();
        }
    }
}

fn restart() -> ! {
    unsafe { esp_idf_sys::esp_restart() }
}
