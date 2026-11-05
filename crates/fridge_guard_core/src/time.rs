/// Japan Standard Time offset from UTC (no daylight saving)
pub const JST_OFFSET_SECS: u64 = 9 * 3600;

/// Wall-clock readings before this (2023-11-14) mean the clock has not been set by SNTP yet
pub const MIN_VALID_EPOCH_SECS: u64 = 1_700_000_000;

/// Calendar date and time of day in JST
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JstDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u8,
    pub minute: u8,
}

impl JstDateTime {
    /// Convert a Unix epoch timestamp to JST calendar date and time
    pub fn from_epoch(epoch_secs: u64) -> Self {
        let jst_secs = epoch_secs.saturating_add(JST_OFFSET_SECS);
        let (year, month, day) = civil_from_days((jst_secs / 86400) as i64);
        let rem = jst_secs % 86400;
        Self {
            year,
            month,
            day,
            hour: (rem / 3600) as u8,
            minute: ((rem % 3600) / 60) as u8,
        }
    }

    /// "YYYY-MM-DD"
    pub fn date_string(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "HH:MM"
    pub fn time_string(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// Unix epoch of `hour:minute` JST on the "YYYY-MM-DD" date, or None if the date is malformed
pub fn jst_epoch_at(date_str: &str, hour: u8, minute: u8) -> Option<u64> {
    let mut parts = date_str.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = u64::try_from(days_from_civil(year, month, day)).ok()?;
    let jst_secs = days * 86400 + u64::from(hour) * 3600 + u64::from(minute) * 60;
    jst_secs.checked_sub(JST_OFFSET_SECS)
}

/// Days since 1970-01-01 -> (year, month, day), Howard Hinnant's civil_from_days
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe as i64 + era * 400 + i64::from(month <= 2);
    (year as i32, month, day)
}

/// (year, month, day) -> days since 1970-01-01, Howard Hinnant's days_from_civil
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_to_jst() {
        // 2026-10-27 11:00:00 UTC = 2026-10-27 20:00 JST
        let t = JstDateTime::from_epoch(1_793_098_800);
        assert_eq!(t.date_string(), "2026-10-27");
        assert_eq!(t.time_string(), "20:00");

        // 15:00 UTC is already the next day in JST
        let t = JstDateTime::from_epoch(1_793_113_200);
        assert_eq!(t.date_string(), "2026-10-28");
        assert_eq!(t.time_string(), "00:00");

        // Leap day
        let t = JstDateTime::from_epoch(1_709_164_800); // 2024-02-29 00:00 UTC
        assert_eq!(t.date_string(), "2024-02-29");
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));
    }

    #[test]
    fn test_jst_epoch_at_roundtrip() {
        assert_eq!(jst_epoch_at("2026-10-27", 20, 0), Some(1_793_098_800));
        for epoch in [1_709_164_800u64, 1_793_098_800, 1_798_729_140] {
            let t = JstDateTime::from_epoch(epoch);
            assert_eq!(
                jst_epoch_at(&t.date_string(), t.hour, t.minute),
                Some(epoch - epoch % 60)
            );
        }
        assert_eq!(jst_epoch_at("2026-13-01", 0, 0), None);
        assert_eq!(jst_epoch_at("not a date", 0, 0), None);
    }
}
