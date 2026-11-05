use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    LineTest,
    StatusNow,
    OtaCheck,
    None,
}

impl MaintenanceAction {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "line_test" => Some(MaintenanceAction::LineTest),
            "status_now" => Some(MaintenanceAction::StatusNow),
            "ota_check" => Some(MaintenanceAction::OtaCheck),
            "none" => Some(MaintenanceAction::None),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MaintenanceAction::LineTest => "line_test",
            MaintenanceAction::StatusNow => "status_now",
            MaintenanceAction::OtaCheck => "ota_check",
            MaintenanceAction::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonGesture {
    ShortPress,
    LongPress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceButtonConfig {
    pub short_press: MaintenanceAction,
    pub long_press: MaintenanceAction,
}

impl Default for MaintenanceButtonConfig {
    fn default() -> Self {
        Self {
            short_press: MaintenanceAction::LineTest,
            long_press: MaintenanceAction::OtaCheck,
        }
    }
}

/// Gesture detector for GPIO9 maintenance button.
/// Handles debouncing, short press vs long press discrimination,
/// and throttle lockout to prevent notification storms.
#[derive(Debug, Clone)]
pub struct ButtonGestureDetector {
    is_pressed: bool,
    press_start_ms: u64,
    long_press_threshold_ms: u64,
    short_press_max_ms: u64,
    min_press_ms: u64,
    cooldown_ms: u64,
    last_event_ms: Option<u64>,
    long_press_emitted_for_current_hold: bool,
}

impl ButtonGestureDetector {
    pub fn new(
        min_press_ms: u64,
        short_press_max_ms: u64,
        long_press_threshold_ms: u64,
        cooldown_ms: u64,
    ) -> Self {
        Self {
            is_pressed: false,
            press_start_ms: 0,
            long_press_threshold_ms,
            short_press_max_ms,
            min_press_ms,
            cooldown_ms,
            last_event_ms: None,
            long_press_emitted_for_current_hold: false,
        }
    }

    pub fn default_nanoc6() -> Self {
        // min 50ms, short press up to 1500ms, long press >= 2500ms (target ~3s), cooldown 3000ms
        Self::new(50, 1500, 2500, 3000)
    }

    /// Update with current physical button state:
    /// In M5Stack NanoC6, GPIO9 button is active LOW (pressed = true / LOW, released = false / HIGH).
    pub fn update(&mut self, is_pressed_now: bool, now_ms: u64) -> Option<ButtonGesture> {
        // Respect cooldown lockout to prevent storming
        if let Some(last_event) = self.last_event_ms {
            if now_ms.saturating_sub(last_event) < self.cooldown_ms {
                self.is_pressed = is_pressed_now;
                return None;
            }
        }

        if is_pressed_now {
            if !self.is_pressed {
                // Just pressed
                self.is_pressed = true;
                self.press_start_ms = now_ms;
                self.long_press_emitted_for_current_hold = false;
            } else {
                // Continued hold
                let duration = now_ms.saturating_sub(self.press_start_ms);
                if duration >= self.long_press_threshold_ms
                    && !self.long_press_emitted_for_current_hold
                {
                    self.long_press_emitted_for_current_hold = true;
                    self.last_event_ms = Some(now_ms);
                    return Some(ButtonGesture::LongPress);
                }
            }
        } else {
            // Button released
            if self.is_pressed {
                self.is_pressed = false;
                let duration = now_ms.saturating_sub(self.press_start_ms);

                if !self.long_press_emitted_for_current_hold
                    && duration >= self.min_press_ms
                    && duration <= self.short_press_max_ms
                {
                    self.last_event_ms = Some(now_ms);
                    return Some(ButtonGesture::ShortPress);
                }
            }
        }

        None
    }
}

/// Action dispatcher that maps detected gestures to configured actions.
/// Actions run one at a time on the firmware's main thread, so no busy tracking is needed here.
#[derive(Debug, Clone)]
pub struct MaintenanceDispatcher {
    config: MaintenanceButtonConfig,
}

impl MaintenanceDispatcher {
    pub fn new(config: MaintenanceButtonConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &MaintenanceButtonConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: MaintenanceButtonConfig) {
        self.config = config;
    }

    /// Dispatch gesture according to current configuration.
    pub fn dispatch(&self, gesture: ButtonGesture) -> MaintenanceAction {
        match gesture {
            ButtonGesture::ShortPress => self.config.short_press,
            ButtonGesture::LongPress => self.config.long_press,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_press_detection() {
        let mut detector = ButtonGestureDetector::default_nanoc6();

        // Press down at t=100
        assert_eq!(detector.update(true, 100), None);
        // Release at t=400 (300ms hold -> valid short press)
        assert_eq!(detector.update(false, 400), Some(ButtonGesture::ShortPress));
    }

    #[test]
    fn test_long_press_detection() {
        let mut detector = ButtonGestureDetector::default_nanoc6();

        // Press down at t=100
        assert_eq!(detector.update(true, 100), None);
        // At t=2600 (2500ms hold) -> fires LongPress while still holding!
        assert_eq!(detector.update(true, 2600), Some(ButtonGesture::LongPress));
        // Continued hold does not re-emit
        assert_eq!(detector.update(true, 3000), None);
        // Release later does not emit ShortPress
        assert_eq!(detector.update(false, 3500), None);
    }

    #[test]
    fn test_cooldown_prevents_notification_storm() {
        let mut detector = ButtonGestureDetector::default_nanoc6();

        // Short press 1
        detector.update(true, 100);
        assert_eq!(detector.update(false, 300), Some(ButtonGesture::ShortPress));

        // Rapid second press during cooldown (< 3000ms later)
        detector.update(true, 500);
        assert_eq!(detector.update(false, 700), None);

        // After cooldown expires (t=3400)
        detector.update(true, 3500);
        assert_eq!(
            detector.update(false, 3700),
            Some(ButtonGesture::ShortPress)
        );
    }

    #[test]
    fn test_dispatcher_action_mapping() {
        let mut dispatcher = MaintenanceDispatcher::new(MaintenanceButtonConfig::default());
        assert_eq!(
            dispatcher.dispatch(ButtonGesture::ShortPress),
            MaintenanceAction::LineTest
        );
        assert_eq!(
            dispatcher.dispatch(ButtonGesture::LongPress),
            MaintenanceAction::OtaCheck
        );

        // Reconfigure mappings
        dispatcher.update_config(MaintenanceButtonConfig {
            short_press: MaintenanceAction::StatusNow,
            long_press: MaintenanceAction::None,
        });
        assert_eq!(
            dispatcher.dispatch(ButtonGesture::ShortPress),
            MaintenanceAction::StatusNow
        );
        assert_eq!(
            dispatcher.dispatch(ButtonGesture::LongPress),
            MaintenanceAction::None
        );
    }
}
