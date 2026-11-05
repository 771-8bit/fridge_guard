use crate::Event;
use esp_idf_hal::gpio::{Gpio2, Gpio9, Input, PinDriver, Pull};
use fridge_guard_core::door::{Debouncer, PinLevel};
use fridge_guard_core::maintenance::ButtonGestureDetector;
use log::{error, info};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Door/button sampling period
const SENSOR_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Reed switch debounce time
const DOOR_DEBOUNCE_MS: u64 = 200;

pub struct HardwareManager<'d> {
    pub door_pin: PinDriver<'d, Gpio2, Input>,
    pub button_pin: PinDriver<'d, Gpio9, Input>,
}

impl<'d> HardwareManager<'d> {
    pub fn new(gpio2: Gpio2, gpio9: Gpio9) -> Result<Self, esp_idf_sys::EspError> {
        info!("Initializing hardware pins for M5Stack NanoC6");

        // Door sensor: GPIO2 via Grove White, with internal pull-up
        let mut door_pin = PinDriver::input(gpio2)?;
        door_pin.set_pull(Pull::Up)?;

        // Maintenance button: GPIO9 onboard button, active low with internal pull-up
        let mut button_pin = PinDriver::input(gpio9)?;
        button_pin.set_pull(Pull::Up)?;

        info!("GPIO2 (door) and GPIO9 (maintenance button) initialized successfully");
        Ok(Self {
            door_pin,
            button_pin,
        })
    }

    /// Read raw electrical level of door sensor (GPIO2)
    pub fn read_door_raw_level(&self) -> PinLevel {
        if self.door_pin.is_low() {
            PinLevel::Low
        } else {
            PinLevel::High
        }
    }

    /// Read raw button state (GPIO9). True if physically pressed (LOW).
    pub fn is_button_pressed(&self) -> bool {
        self.button_pin.is_low()
    }
}

/// Sample the door sensor and maintenance button on a dedicated thread, so network calls
/// on the main thread (LINE delivery, OTA downloads) never delay or drop door events.
/// Every event carries the instant it was detected; the thread ends when `events` is closed.
pub fn spawn_sensor_thread(
    hw: HardwareManager<'static>,
    events: Sender<Event>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("sensor".to_string())
        .stack_size(6144)
        .spawn(move || {
            let start = Instant::now();
            let mut debouncer = Debouncer::new(DOOR_DEBOUNCE_MS);
            let mut button = ButtonGestureDetector::default_nanoc6();
            let mut door_initialized = false;

            loop {
                let now_ms = start.elapsed().as_millis() as u64;

                let door_event = match debouncer.update(hw.read_door_raw_level(), now_ms) {
                    Some(transition) => Some(Event::Door {
                        transition,
                        at: Instant::now(),
                    }),
                    // Boot-time stabilization yields no transition; report the initial state
                    // separately so an already-open door (or detached sensor) is still watched
                    None if !door_initialized && debouncer.is_initialized() => {
                        door_initialized = true;
                        Some(Event::DoorInitialized {
                            state: debouncer.stable_state(),
                            at: Instant::now(),
                        })
                    }
                    None => None,
                };
                let button_event = button
                    .update(hw.is_button_pressed(), now_ms)
                    .map(Event::Button);

                for event in door_event.into_iter().chain(button_event) {
                    if events.send(event).is_err() {
                        error!("Sensor event receiver closed; stopping sensor thread");
                        return;
                    }
                }

                thread::sleep(SENSOR_POLL_INTERVAL);
            }
        })
}
