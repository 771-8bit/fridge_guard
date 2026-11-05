use crate::door::DoorState;
use crate::report::ReportFormatter;
use crate::time::JstDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSnapshot {
    pub date_str: String,
    pub use_count: u32,
    pub last_use_time_str: Option<String>,
    pub elapsed_since_last_use_str: Option<String>,
    pub door_state: DoorState,
}

impl ReportSnapshot {
    /// Capture the report contents for `date_str` (JST "YYYY-MM-DD") at `now_epoch_secs`.
    pub fn capture(
        date_str: String,
        use_count: u32,
        last_activity_epoch_secs: Option<u64>,
        now_epoch_secs: u64,
        door_state: DoorState,
    ) -> Self {
        let (last_use_time_str, elapsed_since_last_use_str) = match last_activity_epoch_secs {
            Some(last) if last <= now_epoch_secs => {
                let at = JstDateTime::from_epoch(last);
                let time = if at.date_string() == date_str {
                    at.time_string()
                } else {
                    format!("{} {}", at.date_string(), at.time_string())
                };
                (
                    Some(time),
                    Some(ReportFormatter::format_elapsed(now_epoch_secs - last)),
                )
            }
            _ => (None, None),
        };
        Self {
            date_str,
            use_count,
            last_use_time_str,
            elapsed_since_last_use_str,
            door_state,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerState {
    pub last_sent_date: Option<String>,
    pub pending_report: Option<ReportSnapshot>,
}

#[derive(Debug, Clone)]
pub struct ReportScheduler {
    report_hour_jst: u8,
    report_minute_jst: u8,
    last_sent_date: Option<String>,
    pending_report: Option<ReportSnapshot>,
}

impl ReportScheduler {
    pub fn new(report_hour_jst: u8, report_minute_jst: u8) -> Self {
        Self {
            report_hour_jst,
            report_minute_jst,
            last_sent_date: None,
            pending_report: None,
        }
    }

    pub fn last_sent_date(&self) -> Option<&str> {
        self.last_sent_date.as_deref()
    }

    pub fn pending_report(&self) -> Option<&ReportSnapshot> {
        self.pending_report.as_ref()
    }

    pub fn restore_state(&mut self, state: SchedulerState) {
        self.last_sent_date = state.last_sent_date;
        self.pending_report = state.pending_report;
    }

    pub fn export_state(&self) -> SchedulerState {
        SchedulerState {
            last_sent_date: self.last_sent_date.clone(),
            pending_report: self.pending_report.clone(),
        }
    }

    /// Check if the regular daily report is due for the given JST calendar date and time.
    /// Returns true if current time >= scheduled time AND the report has not already been sent today
    /// AND there is not already a pending snapshot queued for today.
    pub fn is_daily_report_due(&self, current_date: &str, hour_jst: u8, minute_jst: u8) -> bool {
        // If already sent today, not due
        if let Some(ref sent_date) = self.last_sent_date {
            if sent_date == current_date {
                return false;
            }
        }

        // If already queued as pending for today, not due again
        if let Some(ref pending) = self.pending_report {
            if pending.date_str == current_date {
                return false;
            }
        }

        // Check if scheduled time has arrived
        if hour_jst > self.report_hour_jst {
            return true;
        }
        if hour_jst == self.report_hour_jst && minute_jst >= self.report_minute_jst {
            return true;
        }

        false
    }

    /// Queue a snapshot for delivery. Only one report is kept: if an older report is still
    /// undelivered when the next day's report becomes due, the newer one replaces it.
    pub fn queue_pending_report(&mut self, snapshot: ReportSnapshot) {
        self.pending_report = Some(snapshot);
    }

    /// Mark the report for the given date as successfully sent
    pub fn record_successful_send(&mut self, date_str: String) {
        self.last_sent_date = Some(date_str);
        self.pending_report = None;
    }

    /// Give up on the report for the given date (e.g. monthly message cap reached),
    /// so it is neither retried nor created again for that date.
    pub fn record_dropped(&mut self, date_str: String) {
        self.record_successful_send(date_str);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_due_timing_and_idempotence() {
        let mut scheduler = ReportScheduler::new(20, 0); // 20:00 JST

        // Before 20:00: not due
        assert!(!scheduler.is_daily_report_due("2026-10-27", 19, 59));

        // At 20:00: due!
        assert!(scheduler.is_daily_report_due("2026-10-27", 20, 0));

        // After successful send: NOT due anymore today (idempotence)
        scheduler.record_successful_send("2026-10-27".to_string());
        assert!(!scheduler.is_daily_report_due("2026-10-27", 20, 1));
        assert!(!scheduler.is_daily_report_due("2026-10-27", 23, 59));

        // Next day at 20:00: due again!
        assert!(!scheduler.is_daily_report_due("2026-10-28", 12, 0));
        assert!(scheduler.is_daily_report_due("2026-10-28", 20, 0));
    }

    #[test]
    fn test_pending_report_handling_across_reboots() {
        let mut scheduler = ReportScheduler::new(20, 0);
        let snapshot = ReportSnapshot {
            date_str: "2026-10-27".to_string(),
            use_count: 5,
            last_use_time_str: Some("17:30".to_string()),
            elapsed_since_last_use_str: Some("2時間30分前".to_string()),
            door_state: DoorState::Closed,
        };

        scheduler.queue_pending_report(snapshot);
        assert!(scheduler.pending_report().is_some());

        // Simulate reboot: export and restore
        let exported = scheduler.export_state();
        let mut rebooted_scheduler = ReportScheduler::new(20, 0);
        rebooted_scheduler.restore_state(exported);

        // Pending report is preserved!
        let pending = rebooted_scheduler.pending_report().unwrap();
        assert_eq!(pending.date_str, "2026-10-27");
        assert_eq!(pending.use_count, 5);

        // Deliver pending report
        rebooted_scheduler.record_successful_send("2026-10-27".to_string());
        assert!(rebooted_scheduler.pending_report().is_none());
        assert_eq!(rebooted_scheduler.last_sent_date(), Some("2026-10-27"));
    }

    #[test]
    fn test_snapshot_capture_last_use() {
        // 2026-10-27 20:00 JST
        let now = 1_793_098_800;

        // Used today at 18:42 JST
        let snap = ReportSnapshot::capture(
            "2026-10-27".to_string(),
            3,
            Some(now - 4680),
            now,
            DoorState::Closed,
        );
        assert_eq!(snap.last_use_time_str.as_deref(), Some("18:42"));
        assert_eq!(
            snap.elapsed_since_last_use_str.as_deref(),
            Some("1時間18分前")
        );

        // Last use was on a previous day: date is included
        let snap = ReportSnapshot::capture(
            "2026-10-27".to_string(),
            0,
            Some(now - 26 * 3600),
            now,
            DoorState::Closed,
        );
        assert_eq!(snap.last_use_time_str.as_deref(), Some("2026-10-26 18:00"));
        assert_eq!(
            snap.elapsed_since_last_use_str.as_deref(),
            Some("1日2時間前")
        );

        // Never used
        let snap = ReportSnapshot::capture("2026-10-27".to_string(), 0, None, now, DoorState::Open);
        assert!(snap.last_use_time_str.is_none());
    }
}
