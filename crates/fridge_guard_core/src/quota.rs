use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaError {
    MonthlyLimitReached { current: u32, cap: u32 },
    ReservedForEssentialReport { current: u32, cap: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaState {
    pub current_year: i32,
    pub current_month: u32,
    pub sent_count: u32,
}

#[derive(Debug, Clone)]
pub struct MonthlyQuota {
    cap: u32,
    current_year: i32,
    current_month: u32,
    sent_count: u32,
    reserved_for_essential: u32,
}

impl MonthlyQuota {
    pub fn new(cap: u32, year: i32, month: u32) -> Self {
        Self {
            cap,
            current_year: year,
            current_month: month,
            sent_count: 0,
            reserved_for_essential: 5, // Reserve last 5 messages for essential daily reports
        }
    }

    pub fn cap(&self) -> u32 {
        self.cap
    }

    pub fn sent_count(&self) -> u32 {
        self.sent_count
    }

    pub fn restore_state(&mut self, state: QuotaState) {
        self.current_year = state.current_year;
        self.current_month = state.current_month;
        self.sent_count = state.sent_count;
    }

    pub fn export_state(&self) -> QuotaState {
        QuotaState {
            current_year: self.current_year,
            current_month: self.current_month,
            sent_count: self.sent_count,
        }
    }

    pub fn current_year(&self) -> i32 {
        self.current_year
    }

    pub fn current_month(&self) -> u32 {
        self.current_month
    }

    /// Check if a message can be sent, and increment the counter if permitted.
    pub fn record_send(
        &mut self,
        year: i32,
        month: u32,
        is_essential: bool,
    ) -> Result<u32, QuotaError> {
        self.check_send(year, month, is_essential)?;
        self.sent_count += 1;
        Ok(self.sent_count)
    }

    /// Check if a message can be sent without consuming quota.
    /// Use before an attempt that may fail, then call `record_send` on success.
    pub fn check_send(
        &mut self,
        year: i32,
        month: u32,
        is_essential: bool,
    ) -> Result<(), QuotaError> {
        // Handle calendar month rollover
        if year != self.current_year || month != self.current_month {
            self.current_year = year;
            self.current_month = month;
            self.sent_count = 0;
        }

        self.check_send_current(is_essential)
    }

    /// Check if a message can be sent under current quota without triggering a month rollover
    /// (e.g. before time is synchronized).
    pub fn check_send_current(&self, is_essential: bool) -> Result<(), QuotaError> {
        if self.sent_count >= self.cap {
            return Err(QuotaError::MonthlyLimitReached {
                current: self.sent_count,
                cap: self.cap,
            });
        }

        // If not essential (e.g. test notification) and within reserved buffer
        if !is_essential
            && (self.cap.saturating_sub(self.sent_count) <= self.reserved_for_essential)
        {
            return Err(QuotaError::ReservedForEssentialReport {
                current: self.sent_count,
                cap: self.cap,
            });
        }

        Ok(())
    }

    /// Record a message sent under current month without triggering a month rollover.
    pub fn record_send_current(&mut self, is_essential: bool) -> Result<u32, QuotaError> {
        self.check_send_current(is_essential)?;
        self.sent_count += 1;
        Ok(self.sent_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monthly_quota_limits_and_priority() {
        let mut quota = MonthlyQuota::new(10, 2026, 10);

        // Send 5 non-essential messages
        for _ in 0..5 {
            assert!(quota.record_send(2026, 10, false).is_ok());
        }
        assert_eq!(quota.sent_count(), 5);

        // check_send does not consume quota
        assert!(quota.check_send(2026, 10, true).is_ok());
        assert_eq!(quota.sent_count(), 5);

        // At 5 sends remaining (cap=10, sent=5, remaining=5 <= reserved=5):
        // Non-essential is rejected to protect daily reports!
        assert_eq!(
            quota.record_send(2026, 10, false),
            Err(QuotaError::ReservedForEssentialReport {
                current: 5,
                cap: 10
            })
        );

        // Essential daily report CAN still proceed!
        assert!(quota.record_send(2026, 10, true).is_ok()); // 6
        assert!(quota.record_send(2026, 10, true).is_ok()); // 7
        assert!(quota.record_send(2026, 10, true).is_ok()); // 8
        assert!(quota.record_send(2026, 10, true).is_ok()); // 9
        assert!(quota.record_send(2026, 10, true).is_ok()); // 10

        // Cap reached: even essential is rejected
        assert_eq!(
            quota.record_send(2026, 10, true),
            Err(QuotaError::MonthlyLimitReached {
                current: 10,
                cap: 10
            })
        );

        // Rollover to next month (November): counter resets!
        assert!(quota.record_send(2026, 11, false).is_ok());
        assert_eq!(quota.sent_count(), 1);
    }

    #[test]
    fn test_send_current_without_date_rollover() {
        let mut quota = MonthlyQuota::new(10, 2026, 10);
        // Before NTP sync, messages can still be checked and sent under current month
        assert!(quota.check_send_current(false).is_ok());
        assert_eq!(quota.record_send_current(false).unwrap(), 1);
        assert_eq!(quota.current_year(), 2026);
        assert_eq!(quota.current_month(), 10);
        assert_eq!(quota.sent_count(), 1);
    }
}
