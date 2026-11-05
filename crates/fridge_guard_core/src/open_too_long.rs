use crate::door::DoorState;

/// Monitors door-open duration and emits rate-limited warnings
#[derive(Debug, Clone)]
pub struct DoorOpenAlert {
    threshold_secs: u64,
    enabled: bool,
    open_since_epoch_secs: Option<u64>,
    alerted_this_episode: bool,
}

impl DoorOpenAlert {
    pub fn new(threshold_secs: u64, enabled: bool) -> Self {
        Self {
            threshold_secs,
            enabled,
            open_since_epoch_secs: None,
            alerted_this_episode: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn threshold_secs(&self) -> u64 {
        self.threshold_secs
    }

    pub fn on_door_state_changed(&mut self, state: DoorState, now_epoch_secs: u64) {
        match state {
            DoorState::Open => {
                if self.open_since_epoch_secs.is_none() {
                    self.open_since_epoch_secs = Some(now_epoch_secs);
                    self.alerted_this_episode = false;
                }
            }
            DoorState::Closed => {
                // Closed resets the open episode latch
                self.open_since_epoch_secs = None;
                self.alerted_this_episode = false;
            }
            DoorState::Unknown => {}
        }
    }

    /// Check if the door has been continuously open beyond the threshold.
    /// Returns true exactly once per open episode.
    pub fn check_alert(&mut self, now_epoch_secs: u64) -> bool {
        if !self.enabled || self.alerted_this_episode {
            return false;
        }

        if let Some(open_since) = self.open_since_epoch_secs {
            if now_epoch_secs.saturating_sub(open_since) >= self.threshold_secs {
                self.alerted_this_episode = true;
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_door_open_too_long_single_alert() {
        let mut alert = DoorOpenAlert::new(1800, true);

        // Door opens at t=100
        alert.on_door_state_changed(DoorState::Open, 100);

        // At t=1899 (29m 59s): no alert
        assert!(!alert.check_alert(1899));

        // At t=1900 (30m): alert fired!
        assert!(alert.check_alert(1900));

        // At t=2000 (still open): no repeated alert!
        assert!(!alert.check_alert(2000));

        // Door closes at t=2100
        alert.on_door_state_changed(DoorState::Closed, 2100);
        assert!(!alert.check_alert(2200));

        // Door opens again at t=2500
        alert.on_door_state_changed(DoorState::Open, 2500);
        assert!(!alert.check_alert(2500 + 1799));
        // New episode alerts once threshold is reached
        assert!(alert.check_alert(2500 + 1800));
        assert!(!alert.check_alert(2500 + 1801));
    }
}
