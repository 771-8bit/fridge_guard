# Remote OTA Firmware Release & Rollback Guide

Fridge Guard features pull-based, cryptographic-signature-verified, dual-slot (A/B) over-the-air firmware updates hosted on public GitHub Releases.

---

## 1. OTA Update Architecture

```text
[ Developer Machine ]
        |
        | 1. cargo build (Release)
        | 2. tools/size_gate.py (Ensure image <= 1.92 MB OTA slot)
        | 3. tools/release.py (Compute SHA-256, ECDSA-sign manifest, run secret scan)
        v
[ Public GitHub Repository ]
  ├── release/manifest.json               (Stable public HTTPS URL)
  └── Releases: vX.Y.Z
        └── fridge-guard-vX.Y.Z.bin       (Immutable signed release binary)
        ^
        | Outbound HTTPS GET (Verified with Mozilla root CA bundle)
        |
[ M5Stack NanoC6 ]
  ├── 1. Fetches manifest.json 60 s after boot, then every 24 h, and on the GPIO9 button
  ├── 2. Compares SemVer: Reject equal or older firmware!
  ├── 3. Verifies hardware target identifier ('m5stack-nanoc6-v1')
  ├── 4. Verifies ECDSA P-256 manifest signature with the embedded public key
  ├── 5. Skips a version that was already rolled back once on this device
  ├── 6. Streams binary into inactive OTA partition (ota_0 / ota_1)
  ├── 7. Verifies streamed image SHA-256 against the signed hash
  ├── 8. Switches boot partition in otadata, saves activity/report state, and reboots
  ├── 9. Post-update self-test (new image is in PENDING_VERIFY state):
  │        ├── Manifest fetched over HTTPS within 10 min -> esp_ota_mark_app_valid_cancel_rollback() [Commit Update]
  │        └── Panic / watchdog reset / power loss / no manifest within 10 min -> Bootloader rollback to previous slot
```

Rollback requires the ESP-IDF-built bootloader (`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y`).
`cargo run --release` in `firmware/` flashes it via `tools/flash.py`; plain `espflash flash` would use espflash's
bundled bootloader, which has rollback disabled.

---

## 2. Generating the Local OTA Signing Key

Every manifest is signed with an ECDSA P-256 private key. The matching public key is committed as
`firmware/ota_verification_public.pem` and compiled into the firmware (`include_str!` in `firmware/src/ota.rs`).
The firmware build fails if this file is missing.

The OTA private signing key **MUST NEVER** be placed in Git or CI. Generate the pair once:

```bash
uv run tools/release.py --generate-key
```

- Private key: `~/.config/fridge-guard/keys/ota_signing_private.pem` (override with `--key-path` or `OTA_SIGNING_PRIVATE_KEY`). You are asked for an optional passphrase.
- Public key: `firmware/ota_verification_public.pem` — **commit this file**, then build and USB-flash the firmware.

The tool refuses to overwrite existing keys. **Back up the private key offline** (e.g. password manager or encrypted USB drive):
devices only accept updates signed by the key embedded in the firmware they are currently running.
If the key is lost, every device has to be reflashed over USB with firmware containing a new public key.

<details>
<summary>Equivalent OpenSSL commands</summary>

```bash
mkdir -p ~/.config/fridge-guard/keys
openssl ecparam -name prime256v1 -genkey -noout -out ~/.config/fridge-guard/keys/ota_signing_private.pem
openssl ec -in ~/.config/fridge-guard/keys/ota_signing_private.pem -pubout -out firmware/ota_verification_public.pem
```
</details>

---

## 3. Creating a Release with `tools/release.py`

When releasing a new firmware version (e.g. `v0.2.0`):

1. **Build the release binary** (from `firmware/`, see [BUILD.md](BUILD.md)):
   ```bash
   cd firmware
   cargo build --release
   espflash save-image --chip esp32c6 ../target/riscv32imac-esp-espidf/release/fridge_guard_firmware ../target/fridge_guard_firmware.bin
   cd ..
   ```
   `version` in `firmware/Cargo.toml` (the firmware's `FIRMWARE_VERSION`) must already be bumped to the release version.
2. **Execute the release tool**:
   ```bash
   uv run tools/release.py \
     --version 0.2.0 \
     --binary target/fridge_guard_firmware.bin \
     --repo 771-8bit/fridge_guard \
     --gh-release
   ```
   Add `--min-firmware-version X.Y.Z` if devices older than that must first install an intermediate release.
   `--dry-run` signs a placeholder image into a temporary directory and leaves `release/` untouched.

### What `tools/release.py` performs automatically:
1. **Secret Scan**: Inspects every file Git would publish (tracked and untracked-but-not-ignored) for private keys, bearer tokens, LINE user IDs, long token-like strings and credential assignments, and refuses key files and local secret files (`.env`, `.line_user_id`, ...). `firmware/ota_verification_public.pem` is the only key file allowed. The release image is scanned as well. Intentional test dummies are marked with `secret-scan:allow`.
2. **SemVer Validation**: Verifies version string conforms to SemVer format and equals `version` in `firmware/Cargo.toml`.
3. **Key Checks**: Refuses a private key inside the repository and verifies it matches the public key embedded in the firmware.
4. **Hardware Size Gate**: Parses `partitions.csv` and guarantees the binary strictly fits within the 1.92 MB OTA partition.
5. **Hash & Signature**: Calculates the SHA-256 digest and signs the canonical message (see below).
6. **Manifest Generation**: Updates `release/manifest.json`.
7. **GitHub Release**: Uses the GitHub CLI (`gh release create`) to publish the release asset.

After the GitHub Release exists, commit and push `release/manifest.json` to `main`. Devices pick up the update on their next check.
CI verifies the committed manifest's signature against `firmware/ota_verification_public.pem`
(`uv run tools/release.py --verify-manifest release/manifest.json`), so a manifest devices would reject never reaches `main` unnoticed.
Until the first release, `release/manifest.json` does not exist and devices simply find no update.

For non-interactive use, the key passphrase can be supplied via `OTA_SIGNING_KEY_PASSPHRASE`.

---

## 4. Manifest JSON Schema

The device validates `release/manifest.json` against the following schema:

```json
{
  "schema_version": 1,
  "version": "0.2.0",
  "asset_url": "https://github.com/771-8bit/fridge_guard/releases/download/v0.2.0/fridge-guard-v0.2.0.bin",
  "sha256": "64_hex_characters_sha256_checksum",
  "hardware": "m5stack-nanoc6-v1",
  "timestamp": 1729900000,
  "signature": "128_hex_characters_raw_r_s_ecdsa_p256_signature",
  "release_notes_url": "https://github.com/771-8bit/fridge_guard/releases/tag/v0.2.0",
  "min_firmware_version": "0.1.0"
}
```

`min_firmware_version` is optional. The manifest response must not exceed 8 KB.

---

### Signature

`signature` is an ECDSA P-256 / SHA-256 signature, encoded as hex of the raw 64-byte `r || s` (low-S), over this exact UTF-8 message (no trailing newline):

```text
fridge-guard-ota-v1
hardware=<hardware>
version=<version>
sha256=<lowercase sha256>
min_firmware_version=<min_firmware_version, or empty if absent>
```

The firmware rejects the manifest before downloading anything if the signature does not verify, and then checks the
streamed image against the signed `sha256`. Binding `version` and `hardware` prevents replaying a genuinely signed
older image under a newer version number, and binding `min_firmware_version` prevents stripping that requirement.
`asset_url`, `timestamp`, and `release_notes_url` are not signed:
the download location may change, but the content cannot.

---

## 5. Rollback Behavior & Fail-Safe Protection

- **Unconfirmed Until Proven**: A newly installed image stays in `PENDING_VERIFY` until it has booted, run the sensing loop, and fetched the OTA manifest over HTTPS. Any reboot before that (panic, watchdog reset, power loss) makes the ESP-IDF bootloader mark the slot invalid and boot the previous image.
- **Network Self-Test**: An image that cannot reach the manifest server within 10 minutes rolls itself back (`esp_ota_mark_app_invalid_rollback_and_reboot`). A release that breaks Wi-Fi, TLS, or HTTP therefore cannot strand devices beyond the reach of future OTA fixes. The download that preceded the update proves the network was working moments earlier, so a coincidental outage is unlikely; if one happens, the device just keeps running the previous version.
- **No Retry Loops**: The previous image skips a manifest whose version equals the image that was just rolled back (read from the invalid partition's app descriptor), so a broken release is not downloaded again and again. Publishing a newer version clears this.
- **USB Reflash After OTA**: `tools/flash.py` erases `otadata`, so the image flashed over USB (into `ota_0`) boots even if an earlier OTA had switched the device to `ota_1`. NVS settings are kept.
