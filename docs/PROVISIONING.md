# Local USB Provisioning & Field Maintenance Guide

This document describes how to provision, inspect, and update configuration settings on the Fridge Guard device over a physical USB-C connection.

---

## 1. Security & Reprovisioning Boundary

To ensure absolute security against remote or wireless attackers:
- **No Inbound Wireless Management**: The device does **not** host a Wi-Fi access point, captive portal, Bluetooth service, or HTTP web server.
- **Physical USB Connection Only**: All credential updates (Wi-Fi password changes, LINE tokens, report hours, button remapping) must be performed via local USB-C. The NanoC6 USB-C port is the ESP32-C6's built-in USB-Serial-JTAG, which also carries the device log.
- **No Secret Logging**: The provisioning utility prompts for secrets with terminal echo disabled (`getpass`) and masks credentials in previews. Secrets are never read back from the device.

---

## 2. Initial Device Provisioning

1. Connect the M5Stack NanoC6 to your PC using a USB Type-C cable.
2. Verify available serial ports:
   ```bash
   uv run tools/provision.py --list-ports
   ```
3. Run the interactive provisioning tool:
   ```bash
   uv run tools/provision.py --port COM3   # (or /dev/ttyACM0 on Linux)
   ```
4. Follow the interactive prompts. On a new device the Wi-Fi and LINE fields are required; every other field shows the device's current value and keeps it if you just press Enter:
   - **Wi-Fi SSID**: The 2.4 GHz home Wi-Fi network name (up to 32 bytes).
   - **Wi-Fi Password**: WPA2/WPA3 passphrase, 8–63 characters (input hidden). Open networks are not supported.
   - **LINE Channel Access Token**: The long-lived token issued in `docs/LINE_SETUP.md` (input hidden).
   - **Destination LINE User ID**: The recipient family member's `userId` captured earlier (`U` + 32 hex characters).
   - **Daily Report Time**: Target time in JST (default: `20:00`).
   - **Inactivity alert after N hours**: Default `24`, `0` turns the alert off (max 168).
   - **Door-open alert after N minutes**: Default `30`, `0` turns the alert off (max 1440).
   - **Monthly LINE message cap**: Default `120` (max 500).
   - **Button Short / Long Press Action**: `line_test`, `status_now`, `ota_check` or `none` (defaults: `line_test` / `ota_check`).
5. Confirm the write when prompted. Only the settings you entered are sent; the device merges them into its stored configuration, validates and saves it to NVS, and **restarts automatically** to apply it.

The OTA manifest URL defaults to this repository's `release/manifest.json`. To point a device elsewhere, add `--ota-manifest-url https://...` to the interactive run.

---

## 3. Querying Non-Secret Device Status

To inspect current device health and diagnostics without revealing stored secrets:

```bash
uv run tools/provision.py --port COM3 --status
```

Sample output:
```text
--- Device Status ---
  firmware_version: 0.1.0
  hardware_id: m5stack-nanoc6-v1
  reset_reason: PowerOn
  wifi_connected: True
  wifi_ssid_configured: True
  line_configured: True
  time_synchronized: True
  door_stable_state: 閉
  daily_count: 6
  last_activity_time_str: 18:32
  last_report_sent_date: 2026-10-26
  pending_report_queued: False
  queued_messages: 0
  line_consecutive_failures: 0
  monthly_messages_sent: 27
  monthly_messages_cap: 120
  report_time: 20:00
  inactivity_threshold_hours: 24
  open_too_long_minutes: 30
  button_short_press: line_test
  button_long_press: ota_check
  ota_manifest_url: https://raw.githubusercontent.com/771-8bit/fridge_guard/main/release/manifest.json
```

`line_consecutive_failures` above 0 means LINE deliveries are failing and being retried (see the device log for the reason, e.g. an invalid token).

---

## 4. Triggering a LINE Test Message via USB

To verify LINE communication directly from the host computer:

```bash
uv run tools/provision.py --port COM3 --test-line
```

The device queues a clearly marked `【テスト通知】` message and sends it as soon as it has Wi-Fi and network time, without affecting daily report idempotence or activity counters. Like the button, it counts toward the monthly cap.

`--trigger-ota` runs an OTA update check immediately.

---

## 5. Changing Wi-Fi Credentials in the Field

If the home Wi-Fi router or password changes:
1. Bring a laptop to the refrigerator or unclip the M5Stack NanoC6 and bring it to a PC.
2. Connect via USB-C.
3. Run `uv run tools/provision.py`.
4. Enter the new SSID and password. Press Enter for every other setting to keep it (the LINE token does not have to be re-entered).
5. Confirm the write. The device restarts and connects to the new network.

---

## 6. Factory Reset (Destructive)

To completely erase all provisioned Wi-Fi credentials, tokens, and operational state from NVS:

```bash
uv run tools/provision.py --port COM3 --factory-reset
```

The tool demands an exact confirmation string (`CONFIRM_FACTORY_RESET`) before sending the destructive erase command. The device erases its own NVS namespace and the Wi-Fi driver's credential namespace, then restarts unprovisioned.

---

## 7. Protocol Reference

The tool speaks a line-based protocol over the USB serial console. Each command is one line; each response is one line starting with `@FG ` (log lines on the same port never carry this prefix).

| Command | Response |
|---|---|
| `PING` | `@FG PONG` |
| `GET_STATUS` | `@FG STATUS {json}` |
| `SET_CONFIG {json}` | `@FG OK ...` then restart, or `@FG ERROR ...` — JSON holds only the fields to change |
| `TEST_LINE` | `@FG OK ...` |
| `TRIGGER_OTA` | `@FG OK ...` |
| `FACTORY_RESET CONFIRM_FACTORY_RESET` | `@FG OK ...` then restart |
