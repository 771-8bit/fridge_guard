use serde::{Deserialize, Serialize};

/// Electrical pin level observed on GPIO2
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLevel {
    Low,
    High,
}

/// Logical refrigerator door state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoorState {
    Unknown,
    Closed,
    Open,
}

impl DoorState {
    /// Sensor semantics on M5Stack NanoC6 Grove White (GPIO2) to Black (GND)
    /// Refrigerator CLOSED: Magnet near -> Reed contact closed -> GPIO2 shorted to GND -> LOW
    /// Refrigerator OPEN: Magnet away -> Reed contact open -> Internal pull-up -> HIGH
    pub fn from_pin_level(level: PinLevel) -> Self {
        match level {
            PinLevel::Low => DoorState::Closed,
            PinLevel::High => DoorState::Open,
        }
    }

    pub fn as_str_jp(&self) -> &'static str {
        match self {
            DoorState::Closed => "閉",
            DoorState::Open => "開",
            DoorState::Unknown => "不明",
        }
    }
}

/// Transition between two stable door states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorTransition {
    pub from: DoorState,
    pub to: DoorState,
}

/// Software debouncer for magnetic reed switch
#[derive(Debug, Clone)]
pub struct Debouncer {
    debounce_duration_ms: u64,
    stable_state: DoorState,
    candidate_state: Option<DoorState>,
    candidate_since_ms: u64,
    initialized: bool,
}

impl Debouncer {
    pub fn new(debounce_duration_ms: u64) -> Self {
        Self {
            debounce_duration_ms,
            stable_state: DoorState::Unknown,
            candidate_state: None,
            candidate_since_ms: 0,
            initialized: false,
        }
    }

    pub fn stable_state(&self) -> DoorState {
        self.stable_state
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Process a new raw pin level sample.
    /// Returns Some(DoorTransition) only when a new stable state is confirmed.
    /// Crucially: Startup stabilization sets the initial state without producing
    /// a false Closed->Open transition event.
    pub fn update(&mut self, level: PinLevel, timestamp_ms: u64) -> Option<DoorTransition> {
        let sample_state = DoorState::from_pin_level(level);

        if !self.initialized {
            // First sample: record candidate
            if let Some(cand) = self.candidate_state {
                if cand == sample_state {
                    if timestamp_ms.saturating_sub(self.candidate_since_ms)
                        >= self.debounce_duration_ms
                    {
                        // Stabilized initial state
                        self.stable_state = sample_state;
                        self.initialized = true;
                        self.candidate_state = None;
                        // Return no transition event on initial boot stabilization
                        return None;
                    }
                } else {
                    self.candidate_state = Some(sample_state);
                    self.candidate_since_ms = timestamp_ms;
                }
            } else {
                self.candidate_state = Some(sample_state);
                self.candidate_since_ms = timestamp_ms;
            }
            return None;
        }

        // Already initialized
        if sample_state == self.stable_state {
            // Reverted back to stable state, discard candidate
            self.candidate_state = None;
            return None;
        }

        // Sample differs from current stable state
        if let Some(cand) = self.candidate_state {
            if cand == sample_state {
                if timestamp_ms.saturating_sub(self.candidate_since_ms) >= self.debounce_duration_ms
                {
                    let from = self.stable_state;
                    let to = sample_state;
                    self.stable_state = to;
                    self.candidate_state = None;
                    return Some(DoorTransition { from, to });
                }
            } else {
                // Fluctuated to yet another state, restart candidate timer
                self.candidate_state = Some(sample_state);
                self.candidate_since_ms = timestamp_ms;
            }
        } else {
            // New candidate
            self.candidate_state = Some(sample_state);
            self.candidate_since_ms = timestamp_ms;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_electrical_mapping() {
        assert_eq!(DoorState::from_pin_level(PinLevel::Low), DoorState::Closed);
        assert_eq!(DoorState::from_pin_level(PinLevel::High), DoorState::Open);
    }

    #[test]
    fn test_initial_startup_does_not_emit_transition() {
        let mut debouncer = Debouncer::new(200);
        // Start with HIGH (door open)
        assert_eq!(debouncer.update(PinLevel::High, 0), None);
        assert_eq!(debouncer.update(PinLevel::High, 100), None);
        // Cross debounce threshold
        assert_eq!(debouncer.update(PinLevel::High, 201), None);
        assert!(debouncer.is_initialized());
        assert_eq!(debouncer.stable_state(), DoorState::Open);
    }

    #[test]
    fn test_debounce_filtering_electrical_bounce() {
        let mut debouncer = Debouncer::new(200);
        debouncer.update(PinLevel::Low, 0);
        debouncer.update(PinLevel::Low, 201);
        assert_eq!(debouncer.stable_state(), DoorState::Closed);

        // Rapid bouncing between LOW and HIGH
        assert_eq!(debouncer.update(PinLevel::High, 300), None);
        assert_eq!(debouncer.update(PinLevel::Low, 350), None); // Bounce back!
        assert_eq!(debouncer.update(PinLevel::High, 400), None);
        assert_eq!(debouncer.update(PinLevel::Low, 450), None); // Bounce back again!
        assert_eq!(debouncer.stable_state(), DoorState::Closed);

        // Now sustained HIGH for >= 200ms
        assert_eq!(debouncer.update(PinLevel::High, 500), None);
        assert_eq!(debouncer.update(PinLevel::High, 600), None);
        let trans = debouncer.update(PinLevel::High, 701);
        assert_eq!(
            trans,
            Some(DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open
            })
        );
        assert_eq!(debouncer.stable_state(), DoorState::Open);
    }
}
