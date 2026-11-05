use crate::door::{DoorState, DoorTransition};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityEvent {
    None,
    UsageDetected { total_today: u32 },
    InactivityThresholdCrossed,
    RecoveredFromInactivity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityState {
    pub use_count: u32,
    pub last_activity_epoch_secs: Option<u64>,
    pub current_door_state: DoorState,
    pub inactivity_alert_latched: bool,
    pub tracking_date: String,
}

#[derive(Debug, Clone)]
pub struct ActivityTracker {
    use_count: u32,
    last_activity_epoch_secs: Option<u64>,
    current_door_state: DoorState,
    inactivity_threshold_secs: u64,
    inactivity_alert_enabled: bool,
    inactivity_alert_latched: bool,
    recovery_alert_enabled: bool,
    tracking_date: String,
}

impl ActivityTracker {
    pub fn new(
        initial_door_state: DoorState,
        inactivity_threshold_secs: u64,
        inactivity_alert_enabled: bool,
        recovery_alert_enabled: bool,
        current_date: String,
    ) -> Self {
        Self {
            use_count: 0,
            last_activity_epoch_secs: None,
            current_door_state: initial_door_state,
            inactivity_threshold_secs,
            inactivity_alert_enabled,
            inactivity_alert_latched: false,
            recovery_alert_enabled,
            tracking_date: current_date,
        }
    }

    pub fn restore_state(&mut self, state: ActivityState) {
        self.use_count = state.use_count;
        self.last_activity_epoch_secs = state.last_activity_epoch_secs;
        self.current_door_state = state.current_door_state;
        self.inactivity_alert_latched = state.inactivity_alert_latched;
        self.tracking_date = state.tracking_date;
    }

    pub fn export_state(&self) -> ActivityState {
        ActivityState {
            use_count: self.use_count,
            last_activity_epoch_secs: self.last_activity_epoch_secs,
            current_door_state: self.current_door_state,
            inactivity_alert_latched: self.inactivity_alert_latched,
            tracking_date: self.tracking_date.clone(),
        }
    }

    pub fn use_count(&self) -> u32 {
        self.use_count
    }

    pub fn last_activity_epoch_secs(&self) -> Option<u64> {
        self.last_activity_epoch_secs
    }

    pub fn current_door_state(&self) -> DoorState {
        self.current_door_state
    }

    pub fn is_inactivity_latched(&self) -> bool {
        self.inactivity_alert_latched
    }

    pub fn tracking_date(&self) -> &str {
        &self.tracking_date
    }

    /// Process a verified stable door state transition.
    /// ONLY CLOSED -> OPEN transitions count as a usage event.
    pub fn on_door_transition(
        &mut self,
        transition: DoorTransition,
        now_epoch_secs: u64,
    ) -> ActivityEvent {
        self.current_door_state = transition.to;

        if transition.from == DoorState::Closed && transition.to == DoorState::Open {
            self.use_count += 1;
            self.last_activity_epoch_secs = Some(now_epoch_secs);

            if self.inactivity_alert_latched {
                self.inactivity_alert_latched = false;
                if self.recovery_alert_enabled {
                    return ActivityEvent::RecoveredFromInactivity;
                }
            }

            return ActivityEvent::UsageDetected {
                total_today: self.use_count,
            };
        }

        ActivityEvent::None
    }

    /// Check if inactivity threshold has been exceeded.
    /// Emits InactivityThresholdCrossed exactly once until activity resumes.
    ///
    /// `monitoring_start_epoch_secs` is the real (wall-clock) time the device started watching
    /// the door; it is the reference while no use has ever been recorded. Both arguments must
    /// be real wall-clock times, never an unsynchronized clock or 0.
    pub fn check_inactivity(
        &mut self,
        now_epoch_secs: u64,
        monitoring_start_epoch_secs: u64,
    ) -> ActivityEvent {
        if !self.inactivity_alert_enabled || self.inactivity_alert_latched {
            return ActivityEvent::None;
        }

        let reference_time = self
            .last_activity_epoch_secs
            .unwrap_or(monitoring_start_epoch_secs);

        if now_epoch_secs.saturating_sub(reference_time) >= self.inactivity_threshold_secs {
            self.inactivity_alert_latched = true;
            return ActivityEvent::InactivityThresholdCrossed;
        }

        ActivityEvent::None
    }

    /// Perform daily calendar rollover at 00:00 JST
    pub fn rollover_day(&mut self, new_date: String) {
        if self.tracking_date != new_date {
            self.use_count = 0;
            self.tracking_date = new_date;
            // Note: last_activity_epoch_secs and inactivity_alert_latched
            // are preserved across days so continuous inactivity is tracked accurately!
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_closed_to_open_increments_count() {
        let mut tracker = ActivityTracker::new(
            DoorState::Closed,
            86400,
            true,
            true,
            "2026-10-27".to_string(),
        );
        assert_eq!(tracker.use_count(), 0);

        // CLOSED -> OPEN: increments
        let ev = tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            1000,
        );
        assert_eq!(ev, ActivityEvent::UsageDetected { total_today: 1 });
        assert_eq!(tracker.use_count(), 1);
        assert_eq!(tracker.last_activity_epoch_secs(), Some(1000));

        // OPEN -> CLOSED: does NOT increment
        let ev2 = tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Open,
                to: DoorState::Closed,
            },
            1010,
        );
        assert_eq!(ev2, ActivityEvent::None);
        assert_eq!(tracker.use_count(), 1);

        // Another CLOSED -> OPEN: increments to 2
        let ev3 = tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            2000,
        );
        assert_eq!(ev3, ActivityEvent::UsageDetected { total_today: 2 });
        assert_eq!(tracker.use_count(), 2);
    }

    #[test]
    fn test_inactivity_alert_and_single_shot_and_recovery() {
        let mut tracker = ActivityTracker::new(
            DoorState::Closed,
            86400,
            true,
            true,
            "2026-10-27".to_string(),
        );
        // Last activity at t=1000
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            1000,
        );
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Open,
                to: DoorState::Closed,
            },
            1010,
        );

        // At t=87399 (23h 59m later): no alert
        assert_eq!(tracker.check_inactivity(87399, 0), ActivityEvent::None);
        assert!(!tracker.is_inactivity_latched());

        // At t=87400 (24h later): alert fired once!
        assert_eq!(
            tracker.check_inactivity(87400, 0),
            ActivityEvent::InactivityThresholdCrossed
        );
        assert!(tracker.is_inactivity_latched());

        // At t=87500 (still inactive): NO repeated spam!
        assert_eq!(tracker.check_inactivity(87500, 0), ActivityEvent::None);

        // Next valid refrigerator use at t=90000: recovery event!
        let ev = tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            90000,
        );
        assert_eq!(ev, ActivityEvent::RecoveredFromInactivity);
        assert!(!tracker.is_inactivity_latched());
    }

    #[test]
    fn test_daily_rollover_resets_count_but_preserves_inactivity() {
        let mut tracker = ActivityTracker::new(
            DoorState::Closed,
            86400,
            true,
            true,
            "2026-10-27".to_string(),
        );
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            1000,
        );
        assert_eq!(tracker.use_count(), 1);

        tracker.rollover_day("2026-10-28".to_string());
        assert_eq!(tracker.use_count(), 0);
        assert_eq!(tracker.tracking_date(), "2026-10-28");
        assert_eq!(tracker.last_activity_epoch_secs(), Some(1000));
    }

    #[test]
    fn test_no_alert_right_after_boot_without_recorded_use() {
        // Fresh device: no use recorded yet, monitoring started at a real wall-clock time
        let boot = 1_793_000_000;
        let mut tracker =
            ActivityTracker::new(DoorState::Unknown, 86400, true, true, String::new());
        assert_eq!(
            tracker.check_inactivity(boot + 5, boot),
            ActivityEvent::None
        );
        assert_eq!(
            tracker.check_inactivity(boot + 86400, boot),
            ActivityEvent::InactivityThresholdCrossed
        );
    }

    #[test]
    fn test_state_survives_reboot() {
        let mut tracker = ActivityTracker::new(
            DoorState::Closed,
            86400,
            true,
            true,
            "2026-10-27".to_string(),
        );
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            1_793_000_000,
        );
        assert_eq!(
            tracker.check_inactivity(1_793_086_400, 0),
            ActivityEvent::InactivityThresholdCrossed
        );

        let mut rebooted =
            ActivityTracker::new(DoorState::Unknown, 86400, true, true, String::new());
        rebooted.restore_state(tracker.export_state());
        assert_eq!(rebooted.use_count(), 1);
        assert_eq!(rebooted.tracking_date(), "2026-10-27");
        // Latch survives: no duplicate alert after the reboot
        assert_eq!(
            rebooted.check_inactivity(1_793_090_000, 1_793_089_000),
            ActivityEvent::None
        );
    }
}
