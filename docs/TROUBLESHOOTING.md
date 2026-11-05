# Troubleshooting & Diagnostics Guide

This document covers failure modes, diagnosis procedures, and field troubleshooting for Fridge Guard.

---

## 1. What to Do if the Daily Report Does Not Arrive

> [!IMPORTANT]
> **Fundamental Serverless Limitation**:
> Because this system intentionally uses no permanent cloud server, **if the device loses power, hardware fails, or home internet completely drops, it cannot transmit an error message.**
>
> Therefore, **the absence of the expected daily report (e.g. by 20:30 JST) is itself the primary signal to check on the situation.**

### Diagnostic Checklist:
1. **Check USB Power**: Verify the USB-C power adapter is plugged in and the cable is seated (the firmware does not use the LEDs, so check with `--status` below if unsure).
2. **Check Home Wi-Fi / Internet**: Confirm the home router is online.
3. **Check LINE Token**: Log into LINE Developers Console and verify the channel has not been deleted or token revoked.
4. **Inspect Device Status via USB**:
   Connect a laptop to the NanoC6 and run:
   ```bash
   uv run tools/provision.py --status
   ```
   Check:
   - `wifi_connected`: Is Wi-Fi connected? (The device keeps retrying every 10 s to 5 min while it is not.)
   - `time_synchronized`: Has network time been set? Reports and alerts wait for it.
   - `last_report_sent_date`: Date of the last delivered report.
   - `pending_report_queued`: Is a report waiting for delivery?
   - `line_consecutive_failures`: Above 0, LINE deliveries are failing and being retried (5 s to 5 min). Persistent failures usually mean an invalid or revoked token.
   - `monthly_messages_sent` / `monthly_messages_cap`: Has the monthly cap been reached? Then further messages that month are dropped.

---

## 2. In-Field Hardware Diagnosis with GPIO9 Maintenance Button

You can verify functionality directly at the refrigerator without a computer:

### Short Press (Default: LINE Test)
- Press the onboard GPIO9 button briefly (< 1.5 seconds).
- **Result**: The recipient's phone will receive a `【テスト通知】` message containing current door state and today's use count. If the device is offline, the message is sent once Wi-Fi and network time are available.
- *Note: Does not mark the daily report as sent and does not reset activity counters. There is no LED indication.*

### Long Press (Default: Manual OTA Check)
- Press and hold the GPIO9 button for ~3 seconds.
- If a newer signed release exists, the device downloads it and restarts into it; otherwise nothing visible happens. Door sensing continues during the check.

---

## 3. Sensor & Door State Diagnosis

| Symptom | Probable Cause | Remediation |
|---|---|---|
| Refrigerator opens, but count does not increment | Gap between magnet and sensor too large | Re-align magnet closer to the SPS-320N body (target gap 5–15 mm). |
| Sudden `【ドア開放警告】` alert while door is shut | Sensor wire severed or unplugged | SPS-320N is fail-conservative: a severed cable reads HIGH (Open), also when it is already detached at power-on. Check Grove splice and cable integrity. |
| Multiple increments for a single door opening | Severe mechanical contact bounce or loose magnet | Verify 200 ms debounce setting. Ensure magnet does not vibrate against frame upon opening. |
| Door reads `開` permanently | Grove White disconnected or contact broken | Multimeter continuity test: verify contact closes when magnet is adjacent. |

---

## 4. Power Interruption & Brownout Recovery Test Procedure

The firmware is designed to survive sudden power cuts:

1. **Test Procedure**:
   - Perform 5 door openings to establish a daily use count.
   - Abruptly unplug the USB-C power cable from the NanoC6.
   - Wait 10 seconds.
   - Plug USB-C power back in.
2. **Verification**:
   - Check serial diagnostics or trigger a test notification.
   - Verify: Daily count and last activity timestamp are preserved from NVS (the count is saved at most once per minute, so uses in the last minute before the cut may be lost).
   - Verify: No `【活動未検知】` alert is sent after the restart unless the last use really was longer ago than the threshold.
   - Verify: Daily report idempotence is maintained (does not duplicate report if already sent today).
