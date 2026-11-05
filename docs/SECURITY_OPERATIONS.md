# Security Operations & Threat Model

This document outlines the threat model, credential classifications, operational safeguards, and incident response procedures for Fridge Guard.

---

## 1. Threat Model & Security Posture

Fridge Guard is designed for residential family IoT deployment with strict zero-recurring-cost constraints.

### Key Threats Addressed:
1. **Accidental Credential Publication**: Mitigated by strict separation of secrets from the git repository and public firmware binaries.
2. **Malicious or Corrupted OTA Updates**: Mitigated by verified HTTPS, an ECDSA P-256 manifest signature (binding hardware, version, and image SHA-256) checked against a public key compiled into the firmware, SemVer downgrade prevention, and automatic bootloader rollback. A compromised GitHub account alone cannot push firmware to devices without the offline signing key.
3. **Notification Flooding / API Billable Spam**: Mitigated by gesture debouncing and cooldown, de-duplicated test/status messages, retry backoff, and a monthly safety quota (default 120 messages/month) enforced on every LINE message.
4. **Network Eavesdropping / Man-in-the-Middle**: Mitigated by mandatory TLS certificate validation using Mozilla root CAs. Certificate verification bypass is never permitted.
5. **Remote Intrusion**: Mitigated by eliminating all inbound listening ports, wireless administration portals, captive portals, and remote shells.

---

## 2. Secret Classification Matrix

| Data Item | Classification | Permitted Location |
|---|---|---|
| **Wi-Fi Password** | Secret | Home router, Device NVS flash memory |
| **LINE Channel Access Token** | Secret | LINE Developers Console, Device NVS flash memory |
| **LINE Channel Secret** | Secret | Developer workstation only (for one-time webhook verification) |
| **OTA Signing Private Key** | Critical Secret | Developer offline storage (`~/.config/fridge-guard/keys/`) |
| **Destination LINE User ID** | Private Identifier | LINE Console, Device NVS flash memory |
| **Public OTA Verification Key** | Public | `firmware/ota_verification_public.pem` (committed, embedded in firmware) |
| **Firmware Binaries & Hashes** | Public | GitHub Releases, `release/manifest.json` |

---

## 3. Dedicated Long-Lived LINE Token Tradeoff

LINE documentation recommends short-lived tokens with periodic refresh for enterprise services.

**Engineering Tradeoff**:
Fridge Guard intentionally permits a **dedicated long-lived Channel Access Token** to eliminate the need for a 24/7 cloud server, token-broker backend, or recurring hosting bills.

**Compensating Controls**:
1. **Dedicated Account**: The channel is used exclusively for this device and family.
2. **Outbound Monthly Quota**: The firmware caps all transmissions at 120 messages per month by default.
3. **No Webhook Server**: Normal operation requires no inbound server or webhook endpoint.
4. **Immediate Revocation**: If a token is suspected leaked, it can be revoked instantly from the LINE Developers console and re-provisioned via USB.

---

## 4. Incident Response: Credential Revocation & Reprovisioning

If your Wi-Fi password or LINE token is suspected compromised:

1. **Revoke LINE Access Token**:
   - Open [LINE Developers Console](https://developers.line.biz/).
   - Navigate to your channel's **Messaging API** tab.
   - Click **Reissue** (再発行). The old token is immediately invalidated.
2. **Reprovision the Device**:
   - Connect the M5Stack NanoC6 to your PC via USB-C.
   - Run:
     ```bash
     uv run tools/provision.py
     ```
   - Enter the newly issued token and, if needed, updated Wi-Fi credentials (press Enter to keep the rest).
   - Confirm the write; the device saves to NVS and restarts.
3. **Verify**:
   - Press the GPIO9 maintenance button to send a test message and confirm new credentials work.

---

## 5. Incident Response: OTA Signing Key Loss or Compromise

The verification key is compiled into the firmware, so rotating it always requires a USB reflash of each device.

- **Key lost**: OTA updates are no longer possible. Generate a new pair (`uv run tools/release.py --generate-key` after removing the old public key file), commit the new `firmware/ota_verification_public.pem`, and reflash each device over USB.
- **Key suspected leaked**: Treat every device as able to accept attacker-signed firmware. Generate a new pair, reflash every device over USB, and do not publish any further release signed with the old key. Rotating the key through OTA is not safe, because an attacker holding the old key could do the same.

---

## 6. Physical Flash-at-Rest Boundary

In standard baseline configuration, the ESP32-C6 flash is unencrypted. An attacker with physical possession of the board and specialized SPI flash dumpers could theoretically extract stored NVS strings (Wi-Fi password and LINE token).

Before giving away or disposing of a device, run a factory reset (`uv run tools/provision.py --factory-reset`). It erases the firmware's NVS namespace and the Wi-Fi driver's credential namespace. NVS erasure marks entries as erased rather than overwriting every flash page, so for strong guarantees also erase the whole flash (`espflash erase-flash`) or revoke the LINE token.

For environments where physical tampering is a concern, refer to `docs/OPTIONAL_DEVICE_HARDENING.md` for instructions on Flash Encryption and Secure Boot v2.
