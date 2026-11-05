#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Fridge Guard - USB Flash Helper (cargo runner for `cargo run` in firmware/)

Flashes the firmware with:
- the ESP-IDF-built bootloader, which has OTA rollback enabled (espflash's bundled bootloader does not),
- the dual-OTA partition table (partitions.csv),
- an erased otadata partition, so the freshly flashed image (ota_0) boots even if an earlier
  OTA update had switched the device to ota_1.
The NVS partition (Wi-Fi / LINE settings) is kept.

Usage: uv run tools/flash.py <firmware ELF> [extra espflash args, e.g. --port COM3]
"""

import os
import shutil
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PARTITION_TABLE = os.path.join(REPO_ROOT, "partitions.csv")


def main() -> int:
    if len(sys.argv) < 2:
        print("Usage: flash.py <firmware ELF> [extra espflash args...]", file=sys.stderr)
        return 2

    elf = os.path.abspath(sys.argv[1])
    # esp-idf-sys copies the bootloader it built next to the firmware ELF
    bootloader = os.path.join(os.path.dirname(elf), "bootloader.bin")
    if not os.path.isfile(bootloader):
        print(f"Error: '{bootloader}' not found. Build the firmware first (cd firmware && cargo build --release).", file=sys.stderr)
        return 1
    if shutil.which("espflash") is None:
        print("Error: espflash not found. Install it with: cargo install espflash --locked", file=sys.stderr)
        return 1

    cmd = [
        "espflash", "flash",
        "--bootloader", bootloader,
        "--partition-table", PARTITION_TABLE,
        "--erase-parts", "otadata",
        "--monitor",
        *sys.argv[2:],
        elf,
    ]
    print("Running: " + " ".join(cmd))
    return subprocess.call(cmd)


if __name__ == "__main__":
    sys.exit(main())
