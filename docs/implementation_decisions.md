# Implementation Decisions

This document records technical and architectural decisions made during the implementation of Fridge Guard.

## 1. Modular Architecture: `fridge_guard_core` and `firmware`

- **Decision**: Partitioned the codebase into:
  1. `crates/fridge_guard_core`: A pure, host-testable Rust crate containing all state machines, debouncing, inactivity detection, open-too-long monitoring, report formatting, monthly safety quota, retry backoff policies, OTA manifest parsing, button gesture detection, and USB provisioning command parsing.
  2. `firmware`: An ESP-IDF application for the ESP32-C6 (`riscv32imac-esp-espidf`) using `esp-idf-svc`, `esp-idf-hal`, and safe wrappers around ESP-IDF drivers/APIs.
- **Rationale**: Isolates domain and business logic from ESP-IDF runtime internals. Allows running 100% of acceptance and regression tests locally on developer host workstations and in GitHub Actions CI without requiring cross-compilers or hardware emulators.

## 2. Sensor Debounce Timing and Polarity

- **Decision**: Pinned software debounce duration to **200 ms**.
- **Rationale**: Magnetic reed switches (such as the Akizuki SPS-320N) exhibit mechanical bounce typically lasting 5–30 ms upon impact or separation. 200 ms provides high immunity to bounce and vibration while still capturing all human refrigerator door events.
- **Polarity**:
  - Door CLOSED: Magnet near -> Reed contact closed -> GPIO2 pulled to GND -> LOW.
  - Door OPEN: Magnet separated -> Reed contact open -> Internal pull-up -> HIGH.
  - Fail-conservative: Disconnected or severed sensor wire reads HIGH, surfacing as an open-too-long warning rather than falsely appearing closed.

## 3. Maintenance Button Gestures and Dispatch (GPIO9)

- **Decision**:
  - Minimum press threshold: 50 ms (debounce).
  - Short press duration: 50 ms to 1500 ms (default action: `line_test`).
  - Long press duration: >= 2500 ms (target ~3s, default action: `ota_check`).
  - Cooldown lockout: 3000 ms between triggered events to prevent notification storms.
- **Rationale**: Strict separation between gesture detection (`ButtonGestureDetector`) and action dispatch (`MaintenanceDispatcher`). Mappings (`line_test`, `status_now`, `ota_check`, `none`) are stored in NVS and configurable via USB provisioning without rebuilding firmware.

## 4. Daily Report Scheduling and Idempotence

- **Decision**:
  - Default schedule: 20:00 JST.
  - Scope: Aggregates refrigerator uses for the calendar day from 00:00 JST through the report time.
  - Idempotence: Successfully sent date (e.g. `2026-10-27`) is recorded in persistent NVS. Reboots after 20:00 never re-send a duplicate report.
  - Delivery: At report time a snapshot (count, last use time and elapsed time, door state) is saved to NVS first and then delivered, so a reboot or outage cannot lose it. Failed deliveries are retried with exponential backoff (5 s to 5 min).
  - Delayed report: A report delivered more than 10 minutes after its scheduled time carries a conspicuous `【遅延送信】` banner.
  - Only one undelivered report is kept: if delivery has been impossible for more than a day, the newest day's report replaces the older one.

## 5. Monthly Outbound LINE Safety Quota

- **Decision**: Default safety cap set to **120 messages/month** (below LINE's free Communication Plan limit of 200 messages/month).
- **Scope**: Every LINE message the firmware sends is checked against and counted toward the cap.
- **Priority Reserve**: The final 5 messages in any month are reserved for essential messages (daily reports, inactivity / recovery / door-open alerts); non-essential manual messages (`line_test`, `status_now`) are rejected if the quota is within this buffer.

## 6. Flash Partitioning for 4 MB M5Stack NanoC6

- **Decision**: Dual OTA partitions of **1,920 KB (0x1E0000)** each:
  - `nvs`: 0x9000 (24 KB)
  - `otadata`: 0xF000 (8 KB)
  - `phy_init`: 0x11000 (4 KB)
  - `ota_0`: 0x20000 (1,920 KB)
  - `ota_1`: 0x200000 (1,920 KB)
- **Rationale**: Fits comfortably within the 4 MB physical flash boundary (0x3E0000 < 0x400000) while providing ample room for Rust release binaries. Enforced via `tools/size_gate.py` in CI and release scripts.

## 7. Zero-Backend LINE User ID Capture

- **Decision**: Temporary local webhook receiver (`tools/line_user_capture.py`) using `http.server` + mandatory HMAC-SHA256 signature verification, listening on 127.0.0.1 behind a temporary HTTPS tunnel (Cloudflare Quick Tunnel).
- **Rationale**: Eliminates the need for any always-on cloud server or paid webhook hosting.

## 8. Firmware Threads and Time Handling

- **Decision**: Door and button sampling run on a dedicated sensor thread that sends timestamped events (`std::sync::mpsc`) to the main thread. The main thread handles events and, once per second, Wi-Fi reconnects, alerts, the daily report, LINE delivery and OTA. The USB provisioning protocol runs on a third thread.
- **Rationale**: LINE requests (up to 15 s) and OTA downloads (minutes) must never delay or drop door events.
- **Time**: Open-too-long timing uses the monotonic clock, so it works before SNTP. Use counting and inactivity need wall-clock time: door events are held until SNTP has set the clock and are then counted with their real timestamps. The inactivity reference for a device that has never recorded a use is its boot time, never 0, so a reboot cannot trigger a false inactivity alert.

## 9. Persistence of Activity State

- **Decision**: Today's use count, the last use time and the inactivity alert latch are stored in NVS. Latch changes and day rollovers are written immediately; count updates at most once per minute.
- **Rationale**: Survives power cuts and OTA restarts without duplicate or missing alerts, while keeping flash writes to a few dozen per day.

## 10. Configuration Changes

- **Decision**: `SET_CONFIG` carries only the fields to change. The device merges them into the stored configuration, validates the result, saves it, and restarts to apply it. The Wi-Fi driver runs without its own NVS storage, so credentials exist only in the firmware's namespace and a factory reset removes them.
- **Rationale**: Secrets never have to be re-entered or read back to change another setting, and every setting takes effect consistently.
