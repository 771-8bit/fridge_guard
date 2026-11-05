# Rust on ESP-IDF Toolchain & Reproducibility Guide

## 1. Selected Stack & Versions

This project targets the **M5Stack NanoC6** containing the **ESP32-C6FH4** (RISC-V 32-bit core with IMC extensions and atomics, 4 MB embedded flash).

| Component | Selected Version / Pin | Purpose |
|---|---|---|
| **Rust Toolchain** | `nightly-2025-06-01` (pinned in `rust-toolchain.toml`) with `rust-src` | Required for building `core`/`alloc`/`std` for tier-3 target `riscv32imac-esp-espidf`. Newer nightlies can break `std` for this target (e.g. the 2026-10-25 nightly fails in `std::os::espidf::fs`), so bump the pin deliberately and rebuild the firmware |
| **Rust Target** | `riscv32imac-esp-espidf` | ESP-IDF target for ESP32-C6 |
| **ESP-IDF Version** | `v5.2.2` | Stable Espressif framework release with full C6 support, dual OTA rollback, and mbedtls certificate bundle |
| **esp-idf-svc** | `0.51` | High-level safe wrappers for Wi-Fi, HTTP client, NVS, and SNTP |
| **esp-idf-hal** | `0.45` | Safe GPIO and peripheral abstraction |
| **esp-idf-sys** | `0.36` | Low-level C bindings for ESP-IDF native functions |
| **embedded-svc** | `0.28` | Standard embedded traits for network services |

---

## 2. Unsafe Code Isolation & Justification

The core domain logic (`crates/fridge_guard_core`) contains **zero `unsafe` blocks**.

In the firmware layer (`firmware/src/`), `unsafe` is limited to narrow ESP-IDF OTA C API calls in `firmware/src/ota.rs`:
1. `esp_ota_get_next_update_partition`: Retrieves the inactive OTA partition (`ota_0` or `ota_1`).
2. `esp_ota_begin` / `esp_ota_write` / `esp_ota_end` / `esp_ota_abort`: Streams downloaded firmware chunks into the inactive slot; an RAII guard aborts unfinished writes.
3. `esp_ota_set_boot_partition`: Updates `otadata` to boot into the newly flashed slot upon next restart.
4. `esp_ota_get_running_partition` / `esp_ota_get_state_partition`: Detects a freshly installed image awaiting its self-test.
5. `esp_ota_mark_app_valid_cancel_rollback` / `esp_ota_mark_app_invalid_rollback_and_reboot`: Confirms or rejects the new image after the post-update self-test.
6. `esp_ota_get_last_invalid_partition` / `esp_ota_get_partition_description`: Reads the version that was rolled back, so it is not reinstalled.
7. `esp_restart`: Reboots the microcontroller.

All raw pointers and C return codes are wrapped behind safe Rust APIs.

---

## 3. Why Rust-on-ESP-IDF instead of Pure `no_std`

1. **Production TLS & Certificate Verification**: ESP-IDF provides a mature mbedtls implementation bundled with Mozilla root CAs (`CONFIG_MBEDTLS_CERTIFICATE_BUNDLE=y`). This ensures production-grade HTTPS certificate verification against both LINE (`api.line.me`) and GitHub (`github.com`) without manual root CA hardcoding or insecure bypasses.
2. **Dual-Slot OTA & Automatic Rollback**: ESP-IDF's `otadata` partition driver handles A/B slot swapping, first-boot verification, and hardware watchdog rollbacks automatically.
3. **Robust Wi-Fi & SNTP**: Reuses Espressif's battle-tested 802.11 stack and lwIP SNTP client for stable long-term operation.
