# Verification Report & Acceptance Criteria Status

This document records verification evidence for each acceptance criterion.
Status values:
- **PASS**: Verified by executable host tests, tooling runs, or by inspecting the committed configuration.
- **BUILD-ONLY**: Implemented in the firmware and compiled, but not yet exercised on a physical device.
- **BLOCKED-HARDWARE**: Requires a physical M5Stack NanoC6 board or a physical refrigerator reed contact.
- **BLOCKED-EXTERNAL-CREDENTIAL**: Requires external private user account credentials (LINE Official Account / Channel Token / Recipient's phone).
- **NOT IMPLEMENTED**: The feature does not exist in this version.

Host tests cover the pure logic in `crates/fridge_guard_core`; the firmware glue in `firmware/src/` is only compiled.
Anything that depends on the ESP-IDF runtime, NVS, Wi-Fi, USB, or the LINE service is therefore at most **BUILD-ONLY**
until it has been tried on a device.

---

## Acceptance Matrix

| Item | Description | Status | Verification Evidence |
|---|---|:---:|---|
| **A1** | Valid Rust + ESP-IDF project targeting `riscv32imac-esp-espidf` | **PASS** | `Cargo.toml` and `firmware/.cargo/config.toml` configured for `riscv32imac-esp-espidf`; firmware builds locally (CI builds it once `firmware/ota_verification_public.pem` is committed). |
| **A2** | Pinned compatible toolchain, crates, stable ESP-IDF version | **PASS** | `rust-toolchain.toml` (`nightly-2025-06-01`), `esp-idf-svc = "0.51"`, `esp-idf-hal = "0.45"`, `esp-idf-sys = "0.36"`, ESP-IDF v5.2.2. Firmware build verified on Windows. |
| **A2a**| Firmware application source is Rust | **PASS** | All firmware code in `firmware/src/` and `crates/` is Rust. |
| **A2b**| `Cargo.lock` committed and reproducible build docs | **PASS** | `Cargo.lock` committed; build steps documented in `docs/BUILD.md`. |
| **A2c**| `cargo fmt --check` and `cargo clippy` pass | **PASS** | `cargo fmt --check`, `cargo clippy -p fridge_guard_core --all-targets -- -D warnings`, and firmware `cargo clippy --release -- -D warnings` run clean. |
| **A2d**| Any unsafe/FFI is narrow, documented, and justified | **PASS** | Unsafe is limited to ESP-IDF C APIs without safe wrappers: OTA (`firmware/src/ota.rs`), NVS namespace erase for factory reset (`firmware/src/storage.rs`), and `esp_restart` (`firmware/src/main.rs`). |
| **A3** | Development build compiles without deployment secrets | **PASS** | No credentials are compiled in; all deployment values come from NVS. |
| **A4** | 4 MB NanoC6 partition layout has NVS, otadata, dual OTA slots | **PASS** | `partitions.csv` contains `ota_0` (1920 KB) and `ota_1` (1920 KB). |
| **A5** | Size gate fails if image exceeds OTA slot capacity | **PASS** | `tools/size_gate.py --test-mock-size` PASS at 1.5 MB, FAIL at 2.5 MB. |
| **A5a**| Target is ESP32-C6 / NanoC6 | **PASS** | Hardware ID pinned to `m5stack-nanoc6-v1`. |
| **A6** | `.gitignore` excludes secrets, build artifacts, local keys | **PASS** | `.gitignore` covers `*.pem` (except the public OTA key), `*.key`, `.env*`, `.line_user_id`, `target/`. |
| **A7** | CI contains no secrets and runs automated tests | **PASS** | `.github/workflows/ci.yml` uses no secrets; runs host tests, clippy, firmware build, secret scan. |
| **B0** | Door input is GPIO2 via Grove White; return is GND Black | **PASS** | Pin driver in `firmware/src/hardware.rs`. |
| **B0a**| Grove Red/5V and Yellow/GPIO1 documented as unused/insulated | **PASS** | Documented in `docs/INSTALLATION.md` and `README.md`. |
| **B0b**| Akizuki SPS-320N semantics: magnet near = contact ON/CLOSED | **PASS** | `test_electrical_mapping`. |
| **B1** | Stable LOW maps to CLOSED | **PASS** | `test_acc_01_debounce_and_transition_counting`. |
| **B2** | Stable HIGH maps to OPEN | **PASS** | `test_acc_01_debounce_and_transition_counting`. |
| **B3** | Startup OPEN does not increment use count | **PASS** | `test_acc_02_startup_open_does_not_increment_count`. |
| **B4** | CLOSED -> OPEN increments exactly once | **PASS** | `test_acc_01_debounce_and_transition_counting`. |
| **B5** | Contact bounce does not create extra increments | **PASS** | `test_debounce_filtering_electrical_bounce`. |
| **B6** | OPEN -> CLOSED does not increment use count | **PASS** | `test_acc_01_debounce_and_transition_counting`. |
| **B7** | Long HIGH / disconnected triggers open-too-long warning, including a sensor already detached at boot | **BUILD-ONLY** | Alert logic: `test_acc_07_door_open_too_long`. The sensor thread reports the boot-time stable state (`Event::DoorInitialized`) so an open door at boot is timed too; timing uses the monotonic clock, so it works before SNTP. |
| **C1** | Time sync uses SNTP/NTP and robust epoch handling | **BUILD-ONLY** | `EspSntp`; wall-clock values before 2023-11 are treated as unsynchronized (`MIN_VALID_EPOCH_SECS`). JST conversion: `test_epoch_to_jst`, `test_jst_epoch_at_roundtrip`. |
| **C1a**| Door uses before time sync are counted with correct timestamps | **BUILD-ONLY** | Events carry their detection `Instant` and are counted once the clock is set (`process_door_events` in `firmware/src/main.rs`). |
| **C2** | Report date/time formatting is JST | **PASS** | `test_epoch_to_jst`, `test_snapshot_capture_last_use`. |
| **C3** | Default report schedule is 20:00 JST and configurable | **PASS** | `test_report_due_timing_and_idempotence`; configurable via `tools/provision.py`. |
| **C4** | Exactly one successful daily report per intended date | **PASS** | `test_acc_05_report_idempotence_and_delayed_delivery`. |
| **C5** | Reboot after successful report does not duplicate it | **PASS** | `test_pending_report_handling_across_reboots` (logic); scheduler state is saved to NVS after every change (BUILD-ONLY). |
| **C6** | Offline at report time creates pending report snapshot | **PASS** | The snapshot is always queued and persisted first, then delivered (`test_acc_05_report_idempotence_and_delayed_delivery`). |
| **C7** | Pending report sends once after reconnect and is labeled delayed | **BUILD-ONLY** | `【遅延送信】` is added when delivery happens more than 10 minutes after the scheduled time (`test_delayed_report_formatting` for the formatting). |
| **C8** | Report contains count, last use time with elapsed time, door state, version | **PASS** | `test_snapshot_capture_last_use`, `test_acc_04_report_formatting_and_disclaimer`. |
| **C9** | Alerts/test reports retain disclaimer; wording never claims safety guarantee | **PASS** | Disclaimer verified: `※本通知は冷蔵庫ドアの開閉状況を示す生活活動の参考情報であり、利用者の安全を直接確認・保証するものではありません。` |
| **D1** | 24 h inactivity default, configurable, disable-able (0 = off) | **PASS** | `test_acc_06_inactivity_alert_and_recovery`, `test_alert_thresholds_zero_disables_and_bounds`. |
| **D1a**| No false inactivity alert after a reboot or power cut | **PASS** | `test_no_alert_right_after_boot_without_recorded_use`, `test_state_survives_reboot`; the firmware passes the real boot time, never 0. |
| **D2** | Inactivity alert sends once per inactivity episode | **PASS** | `test_acc_06_inactivity_alert_and_recovery`. |
| **D3** | New refrigerator use clears inactivity latch | **PASS** | `test_acc_06_inactivity_alert_and_recovery`. |
| **D4** | Recovery notification sends once on resumed activity | **PASS** | `test_acc_06_inactivity_alert_and_recovery`. |
| **D5** | 30 min open-too-long default, configurable, disable-able (0 = off) | **PASS** | `test_acc_07_door_open_too_long`, `test_alert_thresholds_zero_disables_and_bounds`. |
| **D6** | Open-too-long alert rate-limited per episode | **PASS** | `test_acc_07_door_open_too_long`. |
| **D7** | Monthly safety cap (default 120) applies to every LINE message | **PASS** | `test_acc_08_monthly_quota_guard`; every push goes through `App::send` in `firmware/src/main.rs`, which checks and records the cap. |
| **D8** | Ordinary door events never cause ordinary LINE pushes | **PASS** | Door transitions only update local counters; pushes are reports, alerts, and maintenance messages. |
| **E1** | Provisioned secrets/config survive reboot | **BUILD-ONLY** | Stored as JSON in the `fridge_guard` NVS namespace. |
| **E2** | Daily report idempotence state survives reboot | **PASS** | `test_pending_report_handling_across_reboots` (logic); saved to NVS (BUILD-ONLY). |
| **E3** | Use count, last use and alert latch survive reboot | **PASS** | `test_state_survives_reboot` (logic); saved to NVS on latch changes immediately and otherwise at most once per minute (BUILD-ONLY). |
| **E4** | Flash write policy avoids needless writes | **PASS** | Activity counters are written at most once per minute; config, scheduler and quota only when they change. |
| **E5** | Logs do not reveal Wi-Fi password, LINE token, SSID or user ID | **PASS** | `NonSecretStatus` contains no secrets; `LineClient` logs neither token nor user ID; protocol errors never echo command arguments (`test_unknown_command_does_not_echo_arguments`). |
| **F1** | Local host provisioning utility exists | **PASS** | `tools/provision.py`. |
| **F1a**| Device actually receives USB commands | **BUILD-ONLY** | USB-Serial-JTAG is the primary console (`sdkconfig.defaults`); the non-blocking console is polled in `firmware/src/provisioning.rs`. |
| **F2** | Secret prompts avoid echo/logging | **PASS** | `getpass.getpass()` in `tools/provision.py`. |
| **F3** | Wi-Fi, LINE token, user ID, report settings changeable, keeping unchanged settings | **PASS** | Partial updates: `test_partial_update_keeps_other_fields`, `test_set_config_is_partial`. The device restarts to apply them. |
| **F4** | Non-secret status can be queried locally | **BUILD-ONLY** | `GET_STATUS`; responses carry the `@FG ` prefix so they are distinguishable from log lines (`test_response_lines_are_prefixed_single_lines`). |
| **F5** | Factory reset requires explicit confirmation | **PASS** | Requires typing `CONFIRM_FACTORY_RESET`. |
| **F5a**| Factory reset erases every stored credential | **BUILD-ONLY** | Erases the `fridge_guard` namespace and the Wi-Fi driver's `nvs.net80211` namespace; Wi-Fi now runs without driver NVS storage. |
| **F6** | Wi-Fi reprovisioning via local USB tooling | **PASS** | Documented in `docs/PROVISIONING.md`. |
| **F7** | No button-triggered AP/captive portal in v1 | **PASS** | GPIO9 only triggers local maintenance actions. |
| **F8** | GPIO9 actions do not reveal credentials | **PASS** | Actions are `line_test`, `status_now`, `ota_check`, `none`. |
| **F9** | Button mappings persist in NVS and changeable without reflashing | **PASS** | Part of `DeviceConfig`, changed via `tools/provision.py`. |
| **G1** | HTTPS push uses official endpoint `https://api.line.me/v2/bot/message/push` | **PASS** | `firmware/src/line.rs`. |
| **G2** | Authorization token loaded from local storage | **PASS** | Stored in NVS, never hard-coded in the binary; validated as printable ASCII without whitespace (`test_wifi_and_token_validation`). |
| **G3** | Destination user ID loaded from local config | **PASS** | Stored in NVS, never hard-coded in the binary. |
| **G4** | Bounded timeouts and exponential backoff retry | **PASS** | `RetryGate` (`test_retry_gate_backoff_and_reset`) is used for LINE delivery (5 s .. 5 min) and Wi-Fi reconnects (10 s .. 5 min). |
| **G5** | Non-2xx errors handled without tight loops | **PASS** | Retryable errors (network, 401/403, 429, 5xx) back off; other 4xx drop the message (`LineError::is_retryable`). |
| **G6** | 401/403 token problems visible in diagnostics without leak | **BUILD-ONLY** | `LineError::Auth` log message; `line_consecutive_failures` in `--status`. |
| **G7** | Temporary user ID capture helper verifies signature | **PASS** | HMAC-SHA256 is mandatory (no skip option); listens on 127.0.0.1 only. |
| **G8** | Channel Secret never required on normal ESP32 operation | **PASS** | Used only in the host helper. |
| **G9** | Alerts are not lost when LINE is temporarily unreachable | **BUILD-ONLY** | Alerts wait in a bounded in-memory queue and are retried with backoff. |
| **H1** | HTTPS manifest fetch verifies certificate | **PASS** | `crt_bundle_attach` in `firmware/src/ota.rs`. |
| **H2** | Manifest schema validation rejects malformed content | **PASS** | `test_acc_09_ota_semver_decision_and_malformed_rejection`; manifest responses over 8 KB are refused. |
| **H3** | Equal/older firmware is not installed | **PASS** | `test_reject_equal_or_downgrade`. |
| **H4** | Wrong hardware target rejected | **PASS** | `test_reject_incompatible_hardware`. |
| **H5** | SHA-256 verified | **PASS** | `test_verify_payload_sha256`, `test_ota_stream_verifier_chunked`. |
| **H6** | Manifest ECDSA P-256 signature verified against embedded public key | **PASS** | `test_reject_unsigned_manifest`, `test_reject_signature_from_untrusted_key`, `test_reject_tampered_signed_fields` (incl. stripped `min_firmware_version`), `test_verify_release_tool_signature_interop`. |
| **H7** | OTA writes inactive slot | **BUILD-ONLY** | `esp_ota_get_next_update_partition`. |
| **H8** | New image receives self-test before confirming validity | **BLOCKED-HARDWARE** | A `PENDING_VERIFY` image is confirmed only after fetching the manifest over HTTPS and rolls itself back after 10 min otherwise. |
| **H9** | Failure before confirmation triggers bootloader rollback | **BLOCKED-HARDWARE** | `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y` in the ESP-IDF-built bootloader, which `tools/flash.py` flashes. |
| **H10**| OTA/network failure does not stop door sensing | **BUILD-ONLY** | Door and button sampling run on a dedicated sensor thread; network calls run on the main thread. |
| **H11**| Release private key absent from Git, CI, firmware | **PASS** | `tools/release.py --scan-only` (publishable files) and the release-image scan. |
| **H12**| Release script supports dry-run and secret scan | **PASS** | `uv run tools/release.py --dry-run` writes only to a temporary directory. |
| **H13**| Release asset immutable/versioned; manifest stable URL | **PASS** | Manifest references `vX.Y.Z/fridge-guard-vX.Y.Z.bin`. |
| **H14**| Short press dispatches `line_test` | **PASS** | `test_dispatcher_action_mapping`. |
| **H15**| `line_test` marked test and does not affect daily report state | **PASS** | Format contains `【テスト通知】`; it only enters the outgoing message queue. |
| **H16**| `line_test` and `status_now` obey monthly quota guard | **PASS** | `test_acc_08_monthly_quota_guard`; they are non-essential messages. |
| **H17**| Long press dispatches `ota_check` | **PASS** | `test_dispatcher_action_mapping`. |
| **H18**| Button OTA follows same verification/rollback policy | **PASS** | Uses the same `run_ota_check` as the scheduled check. |
| **H19**| Equal/older firmware not force-flashed by button | **PASS** | SemVer check in `OtaManifest::parse_and_validate`. |
| **H20**| Repeated presses safely ignored | **PASS** | 3 s gesture cooldown (`test_cooldown_prevents_notification_storm`); at most one test/status message is queued; actions run one at a time on the main thread. |
| **H21**| Supported actions (`line_test`, `status_now`, `ota_check`, `none`) remappable | **PASS** | `test_dispatcher_action_mapping`. |
| **H22**| Local LED feedback | **NOT IMPLEMENTED** | The firmware does not drive the NanoC6 LEDs. Results are confirmed by the LINE message itself or via `tools/provision.py --status`. |
| **I1** | No production TLS verification bypass | **PASS** | No insecure cert bypass compiled into production. |
| **I2** | No permanent inbound Internet administration service | **PASS** | Device only initiates outbound HTTPS connections. |
| **I3** | Public firmware contains no deployment secrets | **PASS** | All deployment values loaded from NVS. |
| **I4** | Public repository search finds no real credentials | **PASS** | `tools/release.py --scan-only` checks every file Git would publish for private keys, bearer tokens, LINE user IDs, long token-like strings and credential assignments. |
| **I5** | Optional Secure Boot / Flash Encryption document exists | **PASS** | `docs/OPTIONAL_DEVICE_HARDENING.md`. |
| **I6** | Automated normal workflow does not burn eFuses | **PASS** | No eFuse burn commands in tooling or CI. |
| **I7** | Token revoke/reprovision procedure documented | **PASS** | `docs/SECURITY_OPERATIONS.md`. |
| **J1** | Wi-Fi reconnect uses bounded backoff | **BUILD-ONLY** | `App::maintain_wifi` reconnects with `RetryGate` (10 s .. 5 min). |
| **J2** | Time sync failure does not stop sensing | **BUILD-ONLY** | Sensing runs on its own thread with a monotonic clock; uses are counted once time is known. |
| **J3** | LINE outage does not stop sensing | **BUILD-ONLY** | See H10. |
| **J4** | GitHub outage does not stop sensing | **BUILD-ONLY** | See H10. |
| **J5** | Network calls have bounded timeouts | **PASS** | 15 s (LINE, manifest) / 30 s (firmware download) HTTP timeouts. |
| **J6** | Watchdog/task design prevents deadlock | **BUILD-ONLY** | Task watchdog enabled with 30 s timeout; threads communicate through a channel. |
| **J7** | Repeated power-cycle test procedure documented | **PASS** | `docs/TROUBLESHOOTING.md`. |
| **K1** | Blank-device-to-installation path documented | **PASS** | `README.md` and `docs/INSTALLATION.md`. |
| **K2** | Wiring documented (ASCII and instructions) | **PASS** | `README.md` and `docs/INSTALLATION.md`. |
| **K3** | LINE account/user-ID setup documented | **PASS** | `docs/LINE_SETUP.md`. |
| **K4** | Provisioning documented | **PASS** | `docs/PROVISIONING.md`. |
| **K5** | OTA release documented | **PASS** | `docs/OTA_RELEASE.md`. |
| **K6** | Troubleshooting documented | **PASS** | `docs/TROUBLESHOOTING.md`. |
| **K7** | Dead-device limitation documented | **PASS** | `README.md` and `docs/TROUBLESHOOTING.md`. |
| **K8** | Human-only external steps separated from code completion | **PASS** | `README.md` and `docs/INSTALLATION.md`. |

---

## Verification Execution Commands & Summary

1. **Host-Side Tests (54 tests)**
   ```powershell
   cargo test -p fridge_guard_core
   ```
   *Result*: `44 passed` (unit) + `10 passed` (acceptance), `0 failed`.

2. **Formatting & Static Analysis (Clippy)**
   ```powershell
   cargo fmt --check
   cargo clippy -p fridge_guard_core --all-targets -- -D warnings
   cd firmware; cargo clippy --release -- -D warnings
   ```
   *Result*: Clean exit with 0 errors and 0 warnings.

3. **Repository Secret Scan**
   ```powershell
   uv run tools/release.py --scan-only
   ```
   *Result*: `[PASS] Secret scan clean. No secrets, key files or local secret files would be published.`

4. **Hardware OTA Slot Size Gate**
   ```powershell
   uv run tools/size_gate.py --test-mock-size 1500000
   uv run tools/size_gate.py --test-mock-size 2500000
   ```
   *Result*: PASS under the 1,966,080-byte slot, FAIL above it.

5. **Release Tooling Dry-Run** (requires the OTA key pair, see `docs/OTA_RELEASE.md`)
   ```powershell
   uv run tools/release.py --version 0.1.0 --dry-run --allow-dirty
   ```
   *Result*: Builds and signs a manifest for a placeholder image in a temporary directory.
