/// Bounded exponential backoff strategy for network retries
#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    initial_delay_ms: u64,
    max_delay_ms: u64,
    factor: f32,
    max_attempts: u32,
}

impl BackoffPolicy {
    pub fn new(initial_delay_ms: u64, max_delay_ms: u64, factor: f32, max_attempts: u32) -> Self {
        Self {
            initial_delay_ms,
            max_delay_ms,
            factor,
            max_attempts,
        }
    }

    /// Standard configuration for LINE notifications and OTA checks
    pub fn default_network_policy() -> Self {
        Self::new(5_000, 300_000, 2.0, 5) // 5s, 10s, 20s, 40s, 80s up to max 300s, max 5 attempts
    }

    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Calculate delay in milliseconds for attempt index (0-indexed).
    /// Returns None if attempt >= max_attempts.
    pub fn delay_for_attempt(&self, attempt: u32) -> Option<u64> {
        if attempt >= self.max_attempts {
            return None;
        }

        let mut delay = self.initial_delay_ms as f64 * (self.factor as f64).powi(attempt as i32);
        if delay > self.max_delay_ms as f64 {
            delay = self.max_delay_ms as f64;
        }

        Some(delay as u64)
    }

    /// Delay for attempt index (0-indexed) that keeps growing up to `max_delay_ms`
    /// and then stays there, for operations that must eventually succeed.
    pub fn capped_delay_for_attempt(&self, attempt: u32) -> u64 {
        let delay =
            self.initial_delay_ms as f64 * (self.factor as f64).powi(attempt.min(64) as i32);
        delay.min(self.max_delay_ms as f64) as u64
    }
}

/// Time gate between attempts of an operation that is retried until it succeeds
/// (Wi-Fi reconnects, LINE delivery). Times are monotonic milliseconds.
#[derive(Debug, Clone)]
pub struct RetryGate {
    policy: BackoffPolicy,
    consecutive_failures: u32,
    next_attempt_ms: u64,
}

impl RetryGate {
    pub fn new(policy: BackoffPolicy) -> Self {
        Self {
            policy,
            consecutive_failures: 0,
            next_attempt_ms: 0,
        }
    }

    pub fn is_ready(&self, now_ms: u64) -> bool {
        now_ms >= self.next_attempt_ms
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.next_attempt_ms = 0;
    }

    /// Hold off the next attempt; returns the delay in milliseconds.
    pub fn record_failure(&mut self, now_ms: u64) -> u64 {
        let delay = self
            .policy
            .capped_delay_for_attempt(self.consecutive_failures);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.next_attempt_ms = now_ms.saturating_add(delay);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation_and_bounding() {
        let policy = BackoffPolicy::new(1000, 5000, 2.0, 4);

        assert_eq!(policy.delay_for_attempt(0), Some(1000));
        assert_eq!(policy.delay_for_attempt(1), Some(2000));
        assert_eq!(policy.delay_for_attempt(2), Some(4000));
        // Capped at max_delay_ms
        assert_eq!(policy.delay_for_attempt(3), Some(5000));
        // Exceeds max attempts
        assert_eq!(policy.delay_for_attempt(4), None);

        // Capped variant never gives up
        assert_eq!(policy.capped_delay_for_attempt(4), 5000);
        assert_eq!(policy.capped_delay_for_attempt(1000), 5000);
    }

    #[test]
    fn test_retry_gate_backoff_and_reset() {
        let mut gate = RetryGate::new(BackoffPolicy::new(1000, 5000, 2.0, 4));
        assert!(gate.is_ready(0));

        assert_eq!(gate.record_failure(10_000), 1000);
        assert!(!gate.is_ready(10_999));
        assert!(gate.is_ready(11_000));

        assert_eq!(gate.record_failure(11_000), 2000);
        assert_eq!(gate.record_failure(13_000), 4000);
        assert_eq!(gate.record_failure(17_000), 5000);
        assert_eq!(gate.record_failure(22_000), 5000);
        assert_eq!(gate.consecutive_failures(), 5);

        gate.record_success();
        assert!(gate.is_ready(22_001));
        assert_eq!(gate.record_failure(30_000), 1000);
    }
}
