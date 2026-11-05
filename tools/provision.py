#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "pyserial>=3.5",
# ]
# ///
"""
Fridge Guard - Local USB Provisioning Utility
Used to configure Wi-Fi, LINE credentials, reporting settings, and maintenance button mappings
over a physically local USB serial connection.

Only the settings you change are sent; the device merges them into its stored configuration,
saves it, and restarts to apply it. Secrets are never read back from the device.

CRITICAL SECURITY RULES:
- Never log or display secrets (passwords, tokens) in plain text.
- Never write secrets into tracked git repository files.
- Destructive factory reset requires explicit interactive confirmation.
"""

import argparse
import getpass
import json
import re
import sys
import time

try:
    import serial
    import serial.tools.list_ports
    HAS_SERIAL = True
except ImportError:
    HAS_SERIAL = False

SUPPORTED_ACTIONS = ["line_test", "status_now", "ota_check", "none"]
# Must match RESPONSE_PREFIX in crates/fridge_guard_core/src/protocol.rs
RESPONSE_PREFIX = "@FG "
# Must match the limits in crates/fridge_guard_core/src/config.rs
MAX_INACTIVITY_THRESHOLD_HOURS = 7 * 24
MAX_OPEN_TOO_LONG_MINUTES = 24 * 60
MAX_MONTHLY_MESSAGE_LIMIT = 500

def mask_secret(s: str) -> str:
    if not s:
        return "<empty>"
    if len(s) <= 8:
        return "********"
    return s[:4] + "..." + s[-4:]

def validate_user_id(uid: str) -> bool:
    # LINE user ID starts with 'U' followed by 32 hex chars (33 total)
    return bool(re.fullmatch(r"U[0-9a-fA-F]{32}", uid.strip()))

def validate_time(t: str) -> bool:
    return bool(re.fullmatch(r"([01]?[0-9]|2[0-3]):[0-5][0-9]", t.strip()))

def validate_wifi_password(p: str) -> bool:
    n = len(p.encode("utf-8"))
    return 8 <= n <= 63 or (n == 64 and bool(re.fullmatch(r"[0-9a-fA-F]{64}", p)))

def validate_line_token(t: str) -> bool:
    return 0 < len(t) <= 1024 and all(33 <= ord(c) <= 126 for c in t)

class DeviceProvisioner:
    def __init__(self, port: str, baud: int = 115200, timeout: float = 0.5):
        self.port = port
        self.baud = baud
        self.timeout = timeout
        self.ser = None

    def connect(self):
        if not HAS_SERIAL:
            raise RuntimeError("pyserial is not installed. Please run: pip install pyserial")
        ser = serial.Serial()
        ser.port = self.port
        ser.baudrate = self.baud
        ser.timeout = self.timeout
        # Keep DTR/RTS deasserted: toggling them on the ESP32-C6 USB-Serial-JTAG resets the chip
        ser.dtr = False
        ser.rts = False
        ser.open()
        self.ser = ser
        time.sleep(0.3)

    def close(self):
        if self.ser and self.ser.is_open:
            self.ser.close()

    def send_cmd(self, cmd_line: str, response_timeout: float = 5.0) -> str:
        """Send one command line and return the response body (without the prefix).

        Device log lines share the serial port; only lines carrying RESPONSE_PREFIX are responses.
        """
        if not self.ser:
            raise RuntimeError("Serial port not connected")
        self.ser.reset_input_buffer()
        self.ser.write((cmd_line.strip() + "\n").encode("utf-8"))
        self.ser.flush()
        deadline = time.monotonic() + response_timeout
        while time.monotonic() < deadline:
            line = self.ser.readline().decode("utf-8", errors="replace")
            idx = line.find(RESPONSE_PREFIX)
            if idx >= 0:
                return line[idx + len(RESPONSE_PREFIX):].strip()
        raise TimeoutError("No response from device. Is the Fridge Guard firmware running on this port?")

    def ping(self) -> bool:
        try:
            return self.send_cmd("PING", response_timeout=2.0) == "PONG"
        except TimeoutError:
            return False

    def get_status(self) -> dict:
        resp = self.send_cmd("GET_STATUS")
        if resp.startswith("STATUS "):
            return json.loads(resp[len("STATUS "):])
        raise RuntimeError(f"Unexpected status response: {resp}")

    def set_config(self, update: dict) -> str:
        resp = self.send_cmd(f"SET_CONFIG {json.dumps(update)}")
        if resp.startswith("OK"):
            return resp[2:].strip()
        raise RuntimeError(f"Device rejected config: {resp}")

    def factory_reset(self) -> str:
        return self.send_cmd("FACTORY_RESET CONFIRM_FACTORY_RESET")

    def test_line(self) -> str:
        return self.send_cmd("TEST_LINE")

    def trigger_ota(self) -> str:
        return self.send_cmd("TRIGGER_OTA")

def list_available_ports():
    if not HAS_SERIAL:
        print("Note: pyserial is not installed; cannot scan COM ports.")
        return []
    ports = list(serial.tools.list_ports.comports())
    return [p.device for p in ports]

def prompt_text(label: str, current_desc: str, required: bool, validator=None, error_msg: str = "Invalid value.", secret: bool = False):
    """Return the new value, or None to keep the current one."""
    hint = "required" if required else f"Enter = keep {current_desc}"
    while True:
        if secret:
            value = getpass.getpass(f"{label} [{hint}] (input hidden): ")
        else:
            value = input(f"{label} [{hint}]: ")
        value = value.strip() if not secret else value
        if not value:
            if required:
                print("  This value is required.")
                continue
            return None
        if validator and not validator(value):
            print(f"  {error_msg}")
            continue
        return value

def prompt_int(label: str, current: int, lo: int, hi: int, note: str = ""):
    while True:
        value = input(f"{label} [Enter = keep {current}]{note}: ").strip()
        if not value:
            return None
        if value.isdigit() and lo <= int(value) <= hi:
            return int(value)
        print(f"  Enter a whole number between {lo} and {hi}.")

def prompt_action(label: str, current: str):
    while True:
        value = input(f"{label} [Enter = keep {current}]: ").strip().lower()
        if not value:
            return None
        if value in SUPPORTED_ACTIONS:
            return value
        print(f"  Choose one of: {', '.join(SUPPORTED_ACTIONS)}")

def interactive_provision(prov: DeviceProvisioner, ota_manifest_url: str | None):
    print("\n--- Fridge Guard Interactive Provisioning ---")
    status = prov.get_status()
    wifi_configured = bool(status.get("wifi_ssid_configured"))
    line_configured = bool(status.get("line_configured"))

    print("Enter new values. Settings you leave empty keep their current value on the device.\n")
    update = {}

    ssid = prompt_text("Wi-Fi SSID (2.4 GHz)", "current network", required=not wifi_configured,
                       validator=lambda v: len(v.encode("utf-8")) <= 32, error_msg="SSID must be at most 32 bytes.")
    if ssid is not None:
        update["wifi_ssid"] = ssid
    wifi_pass = prompt_text("Wi-Fi Password", "current password", required=not wifi_configured or ssid is not None,
                            validator=validate_wifi_password,
                            error_msg="Password must be 8-63 characters (or 64 hex digits).", secret=True)
    if wifi_pass is not None:
        update["wifi_password"] = wifi_pass

    token = prompt_text("LINE Channel Access Token (long-lived)", "current token", required=not line_configured,
                        validator=lambda v: validate_line_token(v.strip()),
                        error_msg="Token must be printable ASCII without spaces.",
                        secret=True)
    if token is not None:
        update["line_channel_token"] = token.strip()
    user_id = prompt_text("Destination LINE User ID (U + 32 hex chars)", "current user ID", required=not line_configured,
                          validator=validate_user_id, error_msg="Must start with 'U' and be 33 characters.")
    if user_id is not None:
        update["line_user_id"] = user_id

    current_time = status.get("report_time", "20:00")
    report_time = prompt_text("Daily Report Time JST (HH:MM)", current_time, required=False,
                              validator=validate_time, error_msg="Use HH:MM, e.g. 20:00.")
    if report_time is not None:
        hour, minute = (int(x) for x in report_time.split(":"))
        update["report_hour"] = hour
        update["report_minute"] = minute

    inactivity = prompt_int("Inactivity alert after N hours", status.get("inactivity_threshold_hours", 24),
                            0, MAX_INACTIVITY_THRESHOLD_HOURS, " (0 = off)")
    if inactivity is not None:
        update["inactivity_threshold_hours"] = inactivity
    open_minutes = prompt_int("Door-open alert after N minutes", status.get("open_too_long_minutes", 30),
                              0, MAX_OPEN_TOO_LONG_MINUTES, " (0 = off)")
    if open_minutes is not None:
        update["open_too_long_minutes"] = open_minutes
    monthly = prompt_int("Monthly LINE message cap", status.get("monthly_messages_cap", 120),
                         1, MAX_MONTHLY_MESSAGE_LIMIT)
    if monthly is not None:
        update["monthly_message_limit"] = monthly

    print(f"\nSupported button actions: {', '.join(SUPPORTED_ACTIONS)}")
    current_short = status.get("button_short_press", "line_test")
    current_long = status.get("button_long_press", "ota_check")
    short_act = prompt_action("Maintenance Button Short Press Action", current_short)
    long_act = prompt_action("Maintenance Button Long Press Action", current_long)
    if short_act is not None or long_act is not None:
        update["button_config"] = {
            "short_press": short_act or current_short,
            "long_press": long_act or current_long,
        }

    if ota_manifest_url:
        update["ota_manifest_url"] = ota_manifest_url

    if not update:
        print("\nNothing to change.")
        return

    secret_fields = {"wifi_password", "line_channel_token", "line_user_id"}
    print("\nSettings to change:")
    for key, value in update.items():
        shown = mask_secret(value) if key in secret_fields else value
        print(f"  {key}: {shown}")

    confirm = input("\nWrite these settings to the device? (yes/no): ").strip().lower()
    if confirm not in ("y", "yes"):
        print("Operation cancelled. No changes written.")
        return
    message = prov.set_config(update)
    print(f"Success: {message}")
    print("The device restarts now; check it with --status after a few seconds.")

def main():
    parser = argparse.ArgumentParser(description="Fridge Guard USB Provisioning Utility")
    parser.add_argument("--port", help="Serial port (e.g. COM3 or /dev/ttyACM0)")
    parser.add_argument("--status", action="store_true", help="Read non-secret device status")
    parser.add_argument("--factory-reset", action="store_true", help="Perform destructive factory reset")
    parser.add_argument("--test-line", action="store_true", help="Queue a test LINE message on the device")
    parser.add_argument("--trigger-ota", action="store_true", help="Run an OTA update check on the device now")
    parser.add_argument("--ota-manifest-url", help="Also set the OTA manifest URL (HTTPS) during interactive provisioning")
    parser.add_argument("--list-ports", action="store_true", help="List available serial ports")
    args = parser.parse_args()

    if args.list_ports:
        ports = list_available_ports()
        print("Available serial ports:")
        for p in ports:
            print(f"  - {p}")
        return

    if args.ota_manifest_url and not args.ota_manifest_url.startswith("https://"):
        print("Error: --ota-manifest-url must be an https:// URL.", file=sys.stderr)
        sys.exit(1)

    port = args.port
    if not port:
        ports = list_available_ports()
        if len(ports) == 1:
            port = ports[0]
            print(f"Auto-selected serial port: {port}")
        else:
            print("Please specify --port. Available ports:")
            for p in ports:
                print(f"  - {p}")
            sys.exit(1)

    prov = DeviceProvisioner(port)
    prov.connect()

    try:
        if not prov.ping():
            print("Error: The device did not answer. Check the port and that the firmware is running.", file=sys.stderr)
            sys.exit(1)
        if args.status:
            status = prov.get_status()
            print("\n--- Device Status ---")
            for k, v in status.items():
                print(f"  {k}: {v}")
        elif args.factory_reset:
            print("\nWARNING: This will erase all Wi-Fi, LINE credentials, and local state from device NVS!")
            confirm = input("Type 'CONFIRM_FACTORY_RESET' to proceed: ").strip()
            if confirm == "CONFIRM_FACTORY_RESET":
                resp = prov.factory_reset()
                if resp.startswith("OK"):
                    print("Factory reset complete. The device restarts unprovisioned.")
                else:
                    print(f"Factory reset failed: {resp}")
                    sys.exit(1)
            else:
                print("Aborted.")
        elif args.test_line:
            print(prov.test_line())
            print("The message is sent as soon as the device has Wi-Fi and network time.")
        elif args.trigger_ota:
            print(prov.trigger_ota())
        else:
            interactive_provision(prov, args.ota_manifest_url)
    except (RuntimeError, TimeoutError) as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)
    finally:
        prov.close()

if __name__ == "__main__":
    main()
