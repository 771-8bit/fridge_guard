# Build & Development Guide

This document describes how to build, test, and flash the Fridge Guard firmware for the **M5Stack NanoC6 (ESP32-C6)** reference hardware.

---

## 1. Prerequisites

### Host System Requirements
- **Rust Toolchain**: Pinned nightly with `rust-src` (configured via `rust-toolchain.toml`; rustup installs it automatically).
- **Python**: Python 3.12 for the ESP-IDF build (installed automatically into `~/.espressif/python_env`), plus `uv` for the tools in `tools/`.
- **Git**: Required by the ESP-IDF build to download ESP-IDF.
- **USB Cable**: USB Type-C cable with data lines for connection to M5Stack NanoC6.
- **OTA Verification Key**: `firmware/ota_verification_public.pem` must exist; the firmware build fails without it. Maintainers create it once with `uv run tools/release.py --generate-key` (see [OTA_RELEASE.md](OTA_RELEASE.md)).

---

## 2. Running Host-Side Unit & Acceptance Tests

All domain logic, state machines, debouncing algorithms, report formatters, and protocol parsers are packaged in `crates/fridge_guard_core` and can be tested on any host OS (Windows, Linux, macOS) without ESP32 hardware or cross-compilers:

```bash
# Run all unit and acceptance tests
cargo test -p fridge_guard_core

# Check code formatting
cargo fmt --check

# Run Clippy static analysis
cargo clippy -p fridge_guard_core -- -D warnings
```

---

## 3. Building Firmware for ESP32-C6 (M5Stack NanoC6)

The firmware targets the `riscv32imac-esp-espidf` Rust tier-3 target on ESP-IDF v5.2.2.
Target, `build-std`, and ESP-IDF settings live in `firmware/.cargo/config.toml`, so **run firmware commands from the `firmware/` directory**.

### Step 3.1: Install ESP-RS Tooling

```bash
cargo install ldproxy
cargo install espflash --locked
```

### Step 3.2: Compile the Release Firmware

> [!IMPORTANT]
> **Windows Users**: `esp-idf-sys` and CMake strictly check build directory path lengths. If your repo is cloned under a typical user path (e.g. `C:\Users\<user>\Documents\...`), CMake will abort immediately with `Error: Too long output directory: ... Shorten your project path down to no more than 10 characters`.
> Before running `cargo build` on Windows (PowerShell), always set a short target directory:
> ```powershell
> $env:CARGO_TARGET_DIR = "C:\fgt"
> ```

```bash
cd firmware
cargo build --release
```

The first build downloads ESP-IDF and its toolchain into `~/.espressif` and takes a while (10–30 minutes). Later builds are incremental.

The compiled ELF is placed at `target/riscv32imac-esp-espidf/release/fridge_guard_firmware` (or `$env:CARGO_TARGET_DIR/riscv32imac-esp-espidf/release/fridge_guard_firmware` if configured).

### Windows Notes

- **libclang**: bindgen needs `libclang.dll`. Install Espressif's clang into `~/.espressif` from PowerShell (not Git Bash) and point the build at it:
  ```powershell
  $env:IDF_TOOLS_PATH = "$HOME\.espressif"
  py -3.12 "$HOME\.espressif\esp-idf\v5.2.2\tools\idf_tools.py" install esp-clang
  # Use the installed version directory:
  $clang = "$HOME\.espressif\tools\esp-clang\16.0.1-fe4f10a809\esp-clang\bin"
  $env:LIBCLANG_PATH = $clang
  $env:PATH = "$clang;$env:PATH"   # its sibling DLLs must be on PATH
  ```
  (`~/.espressif/esp-idf` exists after the first build attempt.)
- **Long paths / Too long output directory**: As noted above in Step 3.2, set a short output path to satisfy CMake path limits:
  ```powershell
  $env:CARGO_TARGET_DIR = "C:\fgt"
  ```
- **Python**: Put Python 3.12 first on `PATH` for the ESP-IDF build (3.12 is the tested version).

---

## 4. Flashing to Physical M5Stack NanoC6

Connect the M5Stack NanoC6 via USB-C to your development computer, then run from `firmware/`:

```bash
# Build, flash and open the serial monitor (extra espflash args go after --, e.g. -- --port COM3)
cargo run --release
```

`cargo run` uses `tools/flash.py` (via `uv`) as its runner, which calls `espflash flash` with:
- the ESP-IDF-built `bootloader.bin` (OTA rollback enabled; espflash's bundled bootloader has it disabled),
- the dual-OTA partition table `partitions.csv` (ESP-IDF's own build cannot resolve a relative custom partition path),
- `--erase-parts otadata`, so the new image boots even after an OTA switched the device to `ota_1`. NVS settings are kept.

---

## 5. OTA Slot Size Gate Verification

Before distributing any firmware build, convert the ELF to an application image and verify that it fits within the 1.92 MB OTA partition (run from `firmware/`):

```bash
# If using default target dir:
espflash save-image --chip esp32c6 ../target/riscv32imac-esp-espidf/release/fridge_guard_firmware ../target/fridge_guard_firmware.bin
cd ..
uv run tools/size_gate.py target/fridge_guard_firmware.bin

# If using CARGO_TARGET_DIR="C:\fgt" on Windows:
# espflash save-image --chip esp32c6 C:/fgt/riscv32imac-esp-espidf/release/fridge_guard_firmware C:/fgt/fridge_guard_firmware.bin
# cd ..
# uv run tools/size_gate.py C:/fgt/fridge_guard_firmware.bin
```

If the binary exceeds the partition limit, the gate will fail with exit code 1.
