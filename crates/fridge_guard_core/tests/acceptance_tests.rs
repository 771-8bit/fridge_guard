use fridge_guard_core::*;

#[test]
fn test_acc_01_debounce_and_transition_counting() {
    let mut debouncer = Debouncer::new(200);
    let mut tracker = ActivityTracker::new(
        DoorState::Closed,
        86400,
        true,
        true,
        "2026-10-27".to_string(),
    );

    // Boot: sample LOW for 200ms -> stable CLOSED, 0 count
    debouncer.update(PinLevel::Low, 0);
    debouncer.update(PinLevel::Low, 201);
    assert_eq!(debouncer.stable_state(), DoorState::Closed);
    assert_eq!(tracker.use_count(), 0);

    // Electrical bounces (high, low, high, low)
    debouncer.update(PinLevel::High, 300);
    debouncer.update(PinLevel::Low, 350);
    debouncer.update(PinLevel::High, 400);
    debouncer.update(PinLevel::Low, 450);
    assert_eq!(debouncer.stable_state(), DoorState::Closed);

    // Sustained opening
    debouncer.update(PinLevel::High, 500);
    let trans = debouncer.update(PinLevel::High, 701);
    assert!(trans.is_some());
    let ev = tracker.on_door_transition(trans.unwrap(), 701);
    assert_eq!(ev, ActivityEvent::UsageDetected { total_today: 1 });
    assert_eq!(tracker.use_count(), 1);

    // Close the door
    debouncer.update(PinLevel::Low, 1000);
    let trans_close = debouncer.update(PinLevel::Low, 1201);
    assert!(trans_close.is_some());
    let ev_close = tracker.on_door_transition(trans_close.unwrap(), 1201);
    assert_eq!(ev_close, ActivityEvent::None);
    assert_eq!(tracker.use_count(), 1); // Closing does NOT increment!
}

#[test]
fn test_acc_02_startup_open_does_not_increment_count() {
    let mut debouncer = Debouncer::new(200);
    let tracker = ActivityTracker::new(
        DoorState::Unknown,
        86400,
        true,
        true,
        "2026-10-27".to_string(),
    );

    // Boot with fridge OPEN (pin reads HIGH)
    let trans1 = debouncer.update(PinLevel::High, 0);
    assert_eq!(trans1, None);
    let trans2 = debouncer.update(PinLevel::High, 205);
    // Initial stabilization produces NO transition event
    assert_eq!(trans2, None);
    assert_eq!(debouncer.stable_state(), DoorState::Open);
    assert_eq!(tracker.use_count(), 0);
}

#[test]
fn test_acc_03_daily_rollover() {
    let mut tracker = ActivityTracker::new(
        DoorState::Closed,
        86400,
        true,
        true,
        "2026-10-27".to_string(),
    );

    // Record 5 uses on Oct 27
    for t in 1..=5 {
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Closed,
                to: DoorState::Open,
            },
            t * 1000,
        );
        tracker.on_door_transition(
            DoorTransition {
                from: DoorState::Open,
                to: DoorState::Closed,
            },
            t * 1000 + 10,
        );
    }
    assert_eq!(tracker.use_count(), 5);

    // Rollover to Oct 28
    tracker.rollover_day("2026-10-28".to_string());
    assert_eq!(tracker.use_count(), 0);
    assert_eq!(tracker.tracking_date(), "2026-10-28");
    // Last activity timestamp is preserved
    assert_eq!(tracker.last_activity_epoch_secs(), Some(5000));
}

#[test]
fn test_acc_04_report_formatting_and_disclaimer() {
    let report = ReportFormatter::format_daily_report(DailyReportParams {
        date_str: "2026年10月27日",
        use_count: 12,
        last_use_time_str: Some("19:45"),
        elapsed_since_last_use_str: Some("15分前"),
        door_state: DoorState::Closed,
        firmware_version: "1.0.0",
        is_delayed: false,
    });

    assert!(report.contains("【冷蔵庫活動レポート】2026年10月27日"));
    assert!(report.contains("・本日利用回数: 12 回"));
    assert!(report.contains("・最終利用時刻: 19:45 (15分前)"));
    assert!(report.contains("・現在のドア状態: 閉"));
    assert!(report.contains("・ファームウェア: v1.0.0"));
    // Daily report should be clean without disclaimer for high readability
    assert!(!report.contains(ReportFormatter::DISCLAIMER));
    // Must NOT claim safety confirmation!
    assert!(!report.contains("安全を確認"));

    // Test reports and alerts retain the disclaimer
    let test_msg = ReportFormatter::format_test_report(DoorState::Closed, 0, "1.0.0", true);
    assert!(test_msg.contains(ReportFormatter::DISCLAIMER));

    let alert_msg = ReportFormatter::format_door_open_too_long_warning(30, "1.0.0");
    assert!(alert_msg.contains(ReportFormatter::DISCLAIMER));
}

#[test]
fn test_acc_05_report_idempotence_and_delayed_delivery() {
    let mut scheduler = ReportScheduler::new(20, 0);

    // 19:59 - not due
    assert!(!scheduler.is_daily_report_due("2026-10-27", 19, 59));

    // 20:00 - due
    assert!(scheduler.is_daily_report_due("2026-10-27", 20, 0));

    // Suppose network is down, queue pending report snapshot
    let snapshot = ReportSnapshot {
        date_str: "2026-10-27".to_string(),
        use_count: 7,
        last_use_time_str: Some("18:00".to_string()),
        elapsed_since_last_use_str: Some("2時間前".to_string()),
        door_state: DoorState::Closed,
    };
    scheduler.queue_pending_report(snapshot);

    // Later at 21:00, not due again because pending report exists
    assert!(!scheduler.is_daily_report_due("2026-10-27", 21, 0));

    // Connectivity recovers: send pending report with delayed flag
    let pending = scheduler.pending_report().unwrap();
    let delayed_msg = ReportFormatter::format_daily_report(DailyReportParams {
        date_str: &pending.date_str,
        use_count: pending.use_count,
        last_use_time_str: pending.last_use_time_str.as_deref(),
        elapsed_since_last_use_str: pending.elapsed_since_last_use_str.as_deref(),
        door_state: pending.door_state,
        firmware_version: "1.0.0",
        is_delayed: true,
    });
    assert!(delayed_msg.contains("【遅延送信】"));

    // Record success
    scheduler.record_successful_send("2026-10-27".to_string());
    assert!(scheduler.pending_report().is_none());

    // Still on 2026-10-27 at 22:00: never duplicate!
    assert!(!scheduler.is_daily_report_due("2026-10-27", 22, 0));
}

#[test]
fn test_acc_06_inactivity_alert_and_recovery() {
    let mut tracker = ActivityTracker::new(
        DoorState::Closed,
        86400,
        true,
        true,
        "2026-10-27".to_string(),
    );

    // Refrigerator used at t=100
    tracker.on_door_transition(
        DoorTransition {
            from: DoorState::Closed,
            to: DoorState::Open,
        },
        100,
    );
    tracker.on_door_transition(
        DoorTransition {
            from: DoorState::Open,
            to: DoorState::Closed,
        },
        110,
    );

    // 24 hours later (t=86500)
    let alert = tracker.check_inactivity(86500, 0);
    assert_eq!(alert, ActivityEvent::InactivityThresholdCrossed);

    // Formatted message
    let alert_msg = ReportFormatter::format_inactivity_warning(24, "1.0.0");
    assert!(alert_msg.contains("【活動未検知】冷蔵庫の開閉を24時間検知していません。"));
    assert!(alert_msg.contains(ReportFormatter::DISCLAIMER));

    // Next hour (t=90100): NO duplicate alert
    assert_eq!(tracker.check_inactivity(90100, 0), ActivityEvent::None);

    // Grandparent uses fridge at t=95000: Recovery notification!
    let rec = tracker.on_door_transition(
        DoorTransition {
            from: DoorState::Closed,
            to: DoorState::Open,
        },
        95000,
    );
    assert_eq!(rec, ActivityEvent::RecoveredFromInactivity);
    let rec_msg = ReportFormatter::format_inactivity_recovery("1.0.0");
    assert!(rec_msg.contains("【活動再開】冷蔵庫の利用を検知しました。"));
}

#[test]
fn test_acc_07_door_open_too_long() {
    let mut alert = DoorOpenAlert::new(1800, true);

    alert.on_door_state_changed(DoorState::Open, 1000);
    assert!(!alert.check_alert(2799)); // 29m 59s
    assert!(alert.check_alert(2800)); // 30m exact -> Alert!

    let msg = ReportFormatter::format_door_open_too_long_warning(30, "1.0.0");
    assert!(msg.contains("【ドア開放警告】冷蔵庫のドアが30分以上開いた状態"));

    // Continued open: no spam
    assert!(!alert.check_alert(4000));

    // Closed: cleared
    alert.on_door_state_changed(DoorState::Closed, 4100);
}

#[test]
fn test_acc_08_monthly_quota_guard() {
    let mut quota = MonthlyQuota::new(120, 2026, 10);

    for _ in 0..115 {
        assert!(quota.record_send(2026, 10, false).is_ok());
    }
    assert_eq!(quota.sent_count(), 115);

    // Next 5 non-essential sends are blocked to preserve quota for daily reports
    assert!(matches!(
        quota.record_send(2026, 10, false),
        Err(QuotaError::ReservedForEssentialReport { .. })
    ));

    // Essential daily report IS allowed
    assert!(quota.record_send(2026, 10, true).is_ok()); // 116
}

fn sign_manifest_json(unsigned_json: &str, key: &p256::ecdsa::SigningKey) -> String {
    use p256::ecdsa::signature::Signer;
    let mut manifest: OtaManifest = serde_json::from_str(unsigned_json).unwrap();
    let sig: p256::ecdsa::Signature = key.sign(manifest.signing_message().as_bytes());
    manifest.signature = hex::encode(sig.to_bytes());
    serde_json::to_string(&manifest).unwrap()
}

#[test]
fn test_acc_09_ota_semver_decision_and_malformed_rejection() {
    let release_key = p256::ecdsa::SigningKey::from_slice(&[0x5au8; 32]).unwrap();
    let trusted = *release_key.verifying_key();

    // Malformed JSON
    assert!(matches!(
        OtaManifest::parse_and_validate("invalid json", "1.0.0", TARGET_HARDWARE_ID, &trusted),
        Err(ManifestValidationError::InvalidJson(_))
    ));

    // Equal version rejected
    let unsigned_v1 = r#"{
        "schema_version": 1,
        "version": "1.0.0",
        "asset_url": "https://example.com/fw.bin",
        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "hardware": "m5stack-nanoc6-v1",
        "timestamp": 1000,
        "signature": ""
    }"#;
    let json_v1 = sign_manifest_json(unsigned_v1, &release_key);
    assert!(matches!(
        OtaManifest::parse_and_validate(&json_v1, "1.0.0", TARGET_HARDWARE_ID, &trusted),
        Err(ManifestValidationError::EqualOrDowngradeVersion { .. })
    ));

    // Downgrade rejected
    assert!(matches!(
        OtaManifest::parse_and_validate(&json_v1, "1.1.0", TARGET_HARDWARE_ID, &trusted),
        Err(ManifestValidationError::EqualOrDowngradeVersion { .. })
    ));

    // Strictly newer, correctly signed version accepted
    let json_v2 = sign_manifest_json(&unsigned_v1.replace("1.0.0", "1.2.0"), &release_key);
    let res = OtaManifest::parse_and_validate(&json_v2, "1.0.0", TARGET_HARDWARE_ID, &trusted);
    assert!(res.is_ok());
    assert_eq!(res.unwrap().version, "1.2.0");

    // Newer version without a valid release signature rejected
    let unsigned_v2 = unsigned_v1.replace("1.0.0", "1.2.0");
    assert!(
        OtaManifest::parse_and_validate(&unsigned_v2, "1.0.0", TARGET_HARDWARE_ID, &trusted)
            .is_err()
    );
}

#[test]
fn test_acc_10_persistence_roundtrip() {
    let state = ActivityState {
        use_count: 14,
        last_activity_epoch_secs: Some(1729900000),
        current_door_state: DoorState::Closed,
        inactivity_alert_latched: true,
        tracking_date: "2026-10-27".to_string(),
    };

    let serialized = serde_json::to_string(&state).unwrap();
    let deserialized: ActivityState = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.use_count, 14);
    assert_eq!(deserialized.last_activity_epoch_secs, Some(1729900000));
    assert_eq!(deserialized.current_door_state, DoorState::Closed);
    assert!(deserialized.inactivity_alert_latched);
    assert_eq!(deserialized.tracking_date, "2026-10-27");
}
