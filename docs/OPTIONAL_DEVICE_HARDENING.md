# Optional Hardware Security Hardening Guide (eFuses, Secure Boot v2 & Flash Encryption)

> [!CAUTION]
> **IRREVERSIBLE HARDWARE ACTIONS WARNING**:
> Burning eFuses permanently modifies physical one-time-programmable silicon fuses inside the ESP32-C6 SoC.
> **ONCE BURNED, EFUSES CANNOT BE CLEARED OR RESET.**
> Flashing invalid keys or corrupted bootloaders with Secure Boot or Flash Encryption permanently locked can permanently brick the device.
>
> **NEVER execute these operations on your only hardware.** Test all procedures on a sacrificial development unit first.

---

## 1. Overview of Hardware Security Features

| Feature | Protection Provided | Irreversible eFuse State |
|---|---|:---:|
| **Signed OTA Updates (Baseline)** | Cryptographically rejects unsigned firmware images via HTTPS | **No** (Software-enforced) |
| **Secure Boot v2** | Cryptographically verifies bootloader and application signatures on hardware reset | **YES** (`SECURE_BOOT_EN`, `BLOCK_KEY0-5`) |
| **Flash Encryption (Release Mode)** | Encrypts external SPI flash at rest with AES-XTS-128; prevents physical flash dumping | **YES** (`FLASH_CRYPT_CNT`, `FLASH_CRYPT_CONFIG`) |
| **NVS Encryption** | Encrypts key-value pairs stored in the NVS partition | **YES** (Requires Flash Encryption) |

---

## 2. Preflight Checklist (MANDATORY BEFORE BURNING EFUSES)

Before proceeding with any eFuse operation:

- [ ] You have verified that normal USB provisioning and signed OTA updates work on the target board.
- [ ] You have generated and backed up all private signing and encryption keys in multiple secure offline locations.
- [ ] You have confirmed that the power supply to the board is stable and PSE-certified. A brownout during eFuse burning can result in an unrecoverable brick.
- [ ] You have tested the exact binary build on a sacrificial board.

---

## 3. Step-by-Step Optional Hardening Procedure

### Step 3.1: Enable Flash Encryption in Development Mode (Test First)

1. Enable Flash Encryption in development mode in `sdkconfig`:
   ```ini
   CONFIG_SECURE_FLASH_ENC_ENABLED=y
   CONFIG_SECURE_FLASH_ENCRYPTION_MODE_DEVELOPMENT=y
   ```
2. Build and flash the firmware via `espflash`:
   ```bash
   cd firmware
   cargo run --release
   ```
3. On first boot, the ESP32-C6 will generate an internal AES key in an eFuse block and encrypt the flash contents.
4. Verify that the device boots, connects to Wi-Fi, and detects door transitions normally.

### Step 3.2: Enable Secure Boot v2

1. Generate a dedicated Secure Boot v2 signing key:
   ```bash
   espsecure.py generate_signing_key --version 2 secure_boot_signing_key.pem
   ```
2. Enable Secure Boot v2 in `sdkconfig`:
   ```ini
   CONFIG_SECURE_BOOT=y
   CONFIG_SECURE_BOOT_V2_ENABLED=y
   ```
3. Build the signed bootloader and application.
4. Flash the bootloader digest to eFuses following Espressif's official guide:
   https://docs.espressif.com/projects/esp-idf/en/latest/esp32c6/security/secure-boot-v2.html

### Step 3.3: Transition to Release Mode (Permanent Lock)

Once and only once you have confirmed all functionality:
1. Set Flash Encryption mode to Release:
   ```ini
   CONFIG_SECURE_FLASH_ENCRYPTION_MODE_RELEASE=y
   ```
2. Burn the final protection eFuses using `espefuse.py`:
   ```bash
   espefuse.py --port COM3 burn_efuse FLASH_CRYPT_CNT
   ```
3. In release mode, JTAG debugging and physical ROM UART download mode are permanently restricted. All subsequent updates MUST be delivered via verified HTTPS OTA.
