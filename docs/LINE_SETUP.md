# LINE Account & Destination User ID Setup Guide

This guide describes how to configure the dedicated LINE Official Account and obtain the destination LINE User ID without running a permanent server or incurring monthly costs.

---

## 1. Overview & Security Architecture

- **Dedicated Account**: Use a new, dedicated LINE Official Account exclusively for this refrigerator monitor. Do not reuse a personal commercial account.
- **Zero Running Cost**: Uses LINE's free Communication Plan (コミュニケーションプラン), which provides up to 200 free push messages per month.
- **No Token-Broker Server**: Uses one long-lived Channel Access Token stored directly in the device's NVS flash memory.
- **Temporary Webhook Helper**: Captures the recipient family member's `userId` via a one-time local script (`tools/line_user_capture.py`), avoiding any permanent cloud infrastructure.

---

## 2. Step 1: Create Dedicated LINE Official Account & Messaging API Channel

1. Log in to the [LINE Developers Console](https://developers.line.biz/).
2. Create a new Provider (e.g. `Family Care`).
3. Click **Create a new channel** and select **Messaging API**.
4. Fill in:
   - **Channel name**: e.g. `冷蔵庫みまもり` (Fridge Guard)
   - **Channel description**: Refrigerator activity reporting
   - **Category / Subcategory**: Residence / Home
5. Agree to terms and create the channel.

---

## 3. Step 2: Issue Long-Lived Channel Access Token

1. In your channel settings on LINE Developers Console, navigate to the **Messaging API** tab.
2. Scroll down to **Channel access token (long-lived)**.
3. Click **Issue** (発行).
4. Copy the issued token and keep it in a temporary secure clipboard.
   *(Note: This token will be provided to `tools/provision.py` during USB provisioning; never commit it to Git.)*

---

## 4. Step 3: Capture Destination LINE User ID

1. On the **Messaging API** tab, locate the channel's **QR code**.
2. Have the recipient family member scan the QR code with their smartphone and add the account as a friend.
3. In your channel's **Basic settings** tab, copy the **Channel Secret**.
4. On your computer, run the capture helper:
   ```bash
   uv run tools/line_user_capture.py
   ```
   *(When prompted, paste the Channel Secret. It will not be echoed on screen.)*
5. In another terminal window, start a temporary HTTPS tunnel (e.g. Cloudflare Quick Tunnel):
   ```bash
   cloudflared tunnel --url http://127.0.0.1:8080
   ```
6. Copy the generated HTTPS URL, append `/callback` (e.g. `https://random-words.trycloudflare.com/callback`), and paste it into **Webhook URL** under the Messaging API tab in LINE Developers Console.
7. Toggle **Use webhook** to **Enabled**.
8. Ask the recipient family member to send any message (e.g. `こんにちは` or `登録`) in the LINE chat.
9. `tools/line_user_capture.py` will verify the `X-Line-Signature`, display the captured `userId` (starting with `U...`), save it to `.line_user_id`, and exit automatically.
10. **Disable the Webhook**: Return to the LINE Developers Console and toggle **Use webhook** to **Disabled**. You can now close the `cloudflared` tunnel.

---

## 5. Token Revocation & Replacement Procedure

If the channel access token is ever compromised or needs rotation:
1. Go to the LINE Developers Console -> Messaging API tab.
2. Click **Reissue** (再発行) on the Channel access token. This immediately invalidates the old token.
3. Connect the M5Stack NanoC6 to your PC via USB-C.
4. Run:
   ```bash
   uv run tools/provision.py
   ```
5. Enter the new token when prompted (press Enter for every other setting to keep it). The device saves the new token to NVS and restarts to use it. Short-press the GPIO9 button to send a test message with the new token.
