use crate::door::DoorState;

/// Formats notification messages sent via the LINE Messaging API
pub struct ReportFormatter;

pub struct DailyReportParams<'a> {
    pub date_str: &'a str, // JST date, e.g. "2026-10-27"
    pub use_count: u32,
    pub last_use_time_str: Option<&'a str>, // e.g. "18:42", "2026-10-26 18:42" or None
    pub elapsed_since_last_use_str: Option<&'a str>, // e.g. "1時間18分前" or None
    pub door_state: DoorState,
    pub firmware_version: &'a str,
    pub is_delayed: bool,
}

impl ReportFormatter {
    pub const DISCLAIMER: &str =
        "※本通知は冷蔵庫ドアの開閉状況を示す生活活動の参考情報であり、利用者の安全を直接確認・保証するものではありません。";

    /// Human-readable elapsed time, e.g. "45分前", "1時間18分前", "2日3時間前"
    pub fn format_elapsed(secs: u64) -> String {
        let minutes = secs / 60;
        let hours = minutes / 60;
        let days = hours / 24;
        if minutes == 0 {
            "1分以内".to_string()
        } else if hours == 0 {
            format!("{minutes}分前")
        } else if days == 0 {
            format!("{}時間{}分前", hours, minutes % 60)
        } else {
            format!("{}日{}時間前", days, hours % 24)
        }
    }

    pub fn format_daily_report(params: DailyReportParams) -> String {
        let mut msg = String::new();

        if params.is_delayed {
            msg.push_str("【遅延送信】※通信障害のため送信が遅延しました\n\n");
        }

        msg.push_str(&format!("【冷蔵庫活動レポート】{}\n", params.date_str));
        msg.push_str(&format!("・本日利用回数: {} 回\n", params.use_count));

        match (params.last_use_time_str, params.elapsed_since_last_use_str) {
            (Some(time), Some(elapsed)) => {
                msg.push_str(&format!("・最終利用時刻: {time} ({elapsed})\n"));
            }
            (Some(time), None) => {
                msg.push_str(&format!("・最終利用時刻: {time}\n"));
            }
            _ => {
                msg.push_str("・最終利用時刻: なし\n");
            }
        }

        msg.push_str(&format!(
            "・現在のドア状態: {}\n",
            params.door_state.as_str_jp()
        ));
        msg.push_str(&format!("・ファームウェア: v{}\n", params.firmware_version));

        msg
    }

    pub fn format_test_report(
        door_state: DoorState,
        use_count: u32,
        firmware_version: &str,
        wifi_connected: bool,
    ) -> String {
        format!(
            "【テスト通知】Fridge Guard 機器テスト送信\n\n\
             本メッセージは本体ボタン（または管理操作）による試験送信です。\n\
             ・現在のドア状態: {}\n\
             ・本日利用回数: {} 回\n\
             ・Wi-Fi接続: {}\n\
             ・ファームウェア: v{}\n\n\
             {}",
            door_state.as_str_jp(),
            use_count,
            if wifi_connected { "良好" } else { "切断" },
            firmware_version,
            Self::DISCLAIMER
        )
    }

    pub fn format_status_report(
        date_str: &str,
        door_state: DoorState,
        use_count: u32,
        firmware_version: &str,
    ) -> String {
        format!(
            "【状況通知】Fridge Guard 現在の動作状態\n\n\
             ・対象日: {}\n\
             ・現在のドア状態: {}\n\
             ・本日利用回数: {} 回\n\
             ・ファームウェア: v{}\n",
            date_str,
            door_state.as_str_jp(),
            use_count,
            firmware_version
        )
    }

    pub fn format_inactivity_warning(hours: u32, firmware_version: &str) -> String {
        format!(
            "【活動未検知】冷蔵庫の開閉を{}時間検知していません。\n\n\
             機器は正常に動作していますが、前回の開閉から設定時間を経過しました。\n\
             必要に応じて電話や訪問などで状況を確認してください。\n\
             (ファームウェア: v{})\n\n\
             {}",
            hours,
            firmware_version,
            Self::DISCLAIMER
        )
    }

    pub fn format_inactivity_recovery(firmware_version: &str) -> String {
        format!(
            "【活動再開】冷蔵庫の利用を検知しました。\n\n\
             活動が確認されたため、通常の監視状態へ復帰しました。\n\
             (ファームウェア: v{})\n\n\
             {}",
            firmware_version,
            Self::DISCLAIMER
        )
    }

    pub fn format_door_open_too_long_warning(minutes: u32, firmware_version: &str) -> String {
        format!(
            "【ドア開放警告】冷蔵庫のドアが{}分以上開いた状態（またはセンサ配線外れ）です。\n\n\
             ドアが半開きになっているか、センサ配線が外れている可能性があります。確認してください。\n\
             (ファームウェア: v{})\n\n\
             {}",
            minutes, firmware_version, Self::DISCLAIMER
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daily_report_formatting() {
        let text = ReportFormatter::format_daily_report(DailyReportParams {
            date_str: "2026年10月27日",
            use_count: 8,
            last_use_time_str: Some("19:15"),
            elapsed_since_last_use_str: Some("45分前"),
            door_state: DoorState::Closed,
            firmware_version: "1.0.0",
            is_delayed: false,
        });

        assert!(text.contains("【冷蔵庫活動レポート】2026年10月27日"));
        assert!(text.contains("・本日利用回数: 8 回"));
        assert!(text.contains("・最終利用時刻: 19:15 (45分前)"));
        assert!(text.contains("・現在のドア状態: 閉"));
        assert!(text.contains("・ファームウェア: v1.0.0"));
        assert!(!text.contains(ReportFormatter::DISCLAIMER));
        assert!(!text.contains("【遅延送信】"));
    }

    #[test]
    fn test_delayed_report_formatting() {
        let text = ReportFormatter::format_daily_report(DailyReportParams {
            date_str: "2026年10月27日",
            use_count: 3,
            last_use_time_str: None,
            elapsed_since_last_use_str: None,
            door_state: DoorState::Open,
            firmware_version: "1.0.0",
            is_delayed: true,
        });

        assert!(text.contains("【遅延送信】"));
        assert!(text.contains("・最終利用時刻: なし"));
        assert!(text.contains("・現在のドア状態: 開"));
    }

    #[test]
    fn test_format_elapsed() {
        assert_eq!(ReportFormatter::format_elapsed(30), "1分以内");
        assert_eq!(ReportFormatter::format_elapsed(45 * 60), "45分前");
        assert_eq!(ReportFormatter::format_elapsed(78 * 60), "1時間18分前");
        assert_eq!(ReportFormatter::format_elapsed(51 * 3600), "2日3時間前");
    }

    #[test]
    fn test_test_report_has_disclaimer_and_no_safety_guarantee() {
        let text = ReportFormatter::format_test_report(DoorState::Closed, 5, "1.0.0", true);
        assert!(text.contains("【テスト通知】"));
        assert!(text.contains(ReportFormatter::DISCLAIMER));
    }
}
