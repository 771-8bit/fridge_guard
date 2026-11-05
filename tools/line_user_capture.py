#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Fridge Guard - Temporary LINE User ID Capture Helper
Runs a temporary local webhook receiver to capture the destination family member's LINE User ID.

WORKFLOW:
1. Create/configure dedicated LINE Official Account / Messaging API channel in LINE Developers console.
2. Family member adds the official account as friend on LINE.
3. Run this script: python tools/line_user_capture.py
4. In another terminal, run a temporary tunnel, e.g.:
      cloudflared tunnel --url http://127.0.0.1:8080
5. Copy the HTTPS tunnel URL (e.g. https://xxx.trycloudflare.com/callback) into LINE Developers console Webhook URL.
6. Family member sends any message (e.g. "こんにちは") to the LINE official account.
7. This helper validates X-Line-Signature using the Channel Secret, captures source.userId, and exits.
8. Disable the webhook URL in LINE Developers console when complete.

SECURITY RULES:
- Never commit Channel Secret or captured user ID to Git.
- Channel Secret is used only locally for signature validation; it is NEVER installed on the ESP32!
"""

import argparse
import base64
import getpass
import hashlib
import hmac
import http.server
import json
import os
import socketserver
import sys

# Webhook payloads with a few events are a few KB; refuse anything far larger
MAX_BODY_BYTES = 64 * 1024
WEBHOOK_PATH = "/callback"

captured_user_id = None
channel_secret = None

def verify_signature(body: bytes, signature: str, secret: str) -> bool:
    if not signature or not secret:
        return False
    hash_val = hmac.new(secret.encode("utf-8"), body, hashlib.sha256).digest()
    expected_signature = base64.b64encode(hash_val).decode("utf-8")
    return hmac.compare_digest(signature.strip(), expected_signature.strip())

class LineWebhookHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        global captured_user_id, channel_secret
        if self.path.split("?", 1)[0] != WEBHOOK_PATH:
            self.send_response(404)
            self.end_headers()
            return
        try:
            content_length = int(self.headers.get("Content-Length", 0))
        except ValueError:
            content_length = -1
        if not 0 <= content_length <= MAX_BODY_BYTES:
            self.send_response(413)
            self.end_headers()
            return
        body = self.rfile.read(content_length)

        signature = self.headers.get("X-Line-Signature", "")

        # Every request must carry a valid LINE signature: the tunnel URL is public
        if not verify_signature(body, signature, channel_secret):
            print("\n[WARNING] Webhook received with INVALID signature! Rejecting.")
            self.send_response(403)
            self.end_headers()
            self.wfile.write(b"Invalid signature")
            return

        try:
            data = json.loads(body.decode("utf-8", errors="replace"))
        except ValueError:
            self.send_response(400)
            self.end_headers()
            return

        events = data.get("events", [])
        if not events:
            # Verification ping from LINE Developers console
            print("\n[INFO] Received LINE Webhook verification ping. Webhook connection confirmed!")
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"OK")
            return

        for ev in events:
            source = ev.get("source", {})
            user_id = source.get("userId")
            if user_id:
                captured_user_id = user_id
                print(f"\n[SUCCESS] Successfully captured LINE User ID: {user_id}")
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"OK")
                return

        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"OK")

    def log_message(self, format, *args):
        # Suppress standard HTTP request logging
        pass

def main():
    global channel_secret, captured_user_id
    parser = argparse.ArgumentParser(description="Temporary LINE User ID Capture Webhook Receiver")
    parser.add_argument("--port", type=int, default=8080, help="Local HTTP server port (default: 8080)")
    parser.add_argument("--output", default=".line_user_id", help="Ignored local file to save captured ID")
    args = parser.parse_args()

    print("==================================================")
    print(" Fridge Guard - Temporary LINE User ID Capture")
    print("==================================================")

    env_secret = os.getenv("LINE_CHANNEL_SECRET")
    if env_secret:
        channel_secret = env_secret.strip()
        print("Loaded LINE Channel Secret from environment variable.")
    else:
        channel_secret = getpass.getpass("Enter LINE Channel Secret (input hidden): ").strip()
    if not channel_secret:
        print("Error: The Channel Secret is required to verify webhook signatures.", file=sys.stderr)
        sys.exit(1)

    print(f"\nStarting local webhook server on 127.0.0.1:{args.port} (reachable only through the tunnel)...")
    print("Instructions:")
    print("  1. In another terminal, run a temporary tunnel to this port:")
    print(f"       cloudflared tunnel --url http://127.0.0.1:{args.port}")
    print("  2. In LINE Developers console (Messaging API settings):")
    print("       Set Webhook URL to: https://<your-tunnel-url>/callback")
    print("       Enable 'Use Webhook'")
    print("  3. Have the family recipient send any message (e.g. 'test') to the official account.")
    print("  4. Waiting for incoming webhook...\n")

    # Loopback only: cloudflared connects locally, nothing else on the LAN needs to reach this
    httpd = socketserver.TCPServer(("127.0.0.1", args.port), LineWebhookHandler)
    httpd.timeout = 1.0

    try:
        while captured_user_id is None:
            httpd.handle_request()
    except KeyboardInterrupt:
        print("\nAborted by user.")
        sys.exit(1)
    finally:
        httpd.server_close()

    if captured_user_id:
        print("\n==================================================")
        print(f" Captured Destination LINE User ID: {captured_user_id}")
        print("==================================================")

        # Save to local git-ignored file
        try:
            with open(args.output, "w", encoding="utf-8") as f:
                f.write(captured_user_id.strip() + "\n")
            print(f"Saved to local file '{args.output}' (excluded by .gitignore).")
        except Exception as e:
            print(f"Note: Could not save to {args.output}: {e}")

        print("\nNext steps:")
        print("  1. In LINE Developers Console, DISABLE or REMOVE the Webhook URL (no longer needed).")
        print("  2. Terminate the cloudflared tunnel.")
        print(f"  3. Use this User ID in 'tools/provision.py' to provision your M5Stack NanoC6.")

if __name__ == "__main__":
    main()
