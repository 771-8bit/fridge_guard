#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Fridge Guard - OTA Slot Image Size Gate
Verifies that the compiled firmware binary fits within the hardware OTA partition slot.

CRITICAL HARDWARE CONSTRAINT:
- Board: M5Stack NanoC6 (4 MB Flash)
- Partition table (partitions.csv) defines dual OTA slots (ota_0, ota_1).
- This gate enforces that the firmware application image (including any signing header overhead)
  strictly fits inside the designated OTA slot.
"""

import argparse
import os
import sys

def parse_partition_table(csv_path: str) -> dict:
    """Parse partitions.csv and return partition slot sizes in bytes."""
    slots = {}
    if not os.path.exists(csv_path):
        raise FileNotFoundError(f"Partition table file not found: {csv_path}")

    with open(csv_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = [p.strip() for p in line.split(",")]
            if len(parts) >= 5:
                name, ptype, subtype, offset, size = parts[0], parts[1], parts[2], parts[3], parts[4]
                size_bytes = int(size, 16) if size.lower().startswith("0x") else int(size)
                slots[name] = size_bytes
    return slots

def check_size_gate(binary_path: str, partition_csv_path: str, max_allowed_override: int = None) -> bool:
    if not os.path.exists(binary_path):
        print(f"Error: Binary file not found at: {binary_path}", file=sys.stderr)
        return False

    actual_size = os.path.getsize(binary_path)

    if max_allowed_override:
        max_size = max_allowed_override
        slot_name = "custom override"
    else:
        slots = parse_partition_table(partition_csv_path)
        ota_0_size = slots.get("ota_0")
        ota_1_size = slots.get("ota_1")
        if not ota_0_size:
            print("Error: Could not locate 'ota_0' partition in partitions.csv", file=sys.stderr)
            return False
        max_size = ota_0_size
        slot_name = f"ota_0 ({ota_0_size} bytes / {ota_0_size // 1024} KB)"

    headroom = max_size - actual_size
    headroom_pct = (headroom / max_size) * 100.0

    print("==================================================")
    print(" Fridge Guard - Firmware Size Gate Verification")
    print("==================================================")
    print(f"  Target Binary:     {binary_path}")
    print(f"  Binary Size:       {actual_size:,} bytes ({actual_size / 1024:.1f} KB)")
    print(f"  OTA Slot Limit:    {max_size:,} bytes ({max_size / 1024:.1f} KB) [{slot_name}]")

    if actual_size <= max_size:
        print(f"  Status:            PASS [Headroom: {headroom:,} bytes ({headroom_pct:.1f}% free)]")
        print("==================================================")
        return True
    else:
        overflow = actual_size - max_size
        print(f"  Status:            FAIL [EXCEEDED by {overflow:,} bytes!]")
        print("==================================================")
        print("CRITICAL: The firmware binary exceeds the 4 MB flash OTA slot capacity!")
        print("Reduce code size, optimize dependencies, or review compiler optimization flags.")
        return False

def main():
    parser = argparse.ArgumentParser(description="Verify firmware binary fits within OTA partition slot")
    parser.add_argument("binary", nargs="?", help="Path to compiled .bin firmware binary")
    parser.add_argument("--partitions", default="partitions.csv", help="Path to partitions.csv (default: partitions.csv)")
    parser.add_argument("--max-size", type=int, help="Override maximum size in bytes directly")
    parser.add_argument("--test-mock-size", type=int, help="Run test with mock size without requiring actual binary")
    args = parser.parse_args()

    if args.test_mock_size:
        slots = parse_partition_table(args.partitions)
        limit = slots.get("ota_0", 1966080)
        mock_size = args.test_mock_size
        passed = mock_size <= limit
        print(f"[TEST] Mock size: {mock_size:,} vs Limit: {limit:,} -> {'PASS' if passed else 'FAIL'}")
        sys.exit(0 if passed else 1)

    if not args.binary:
        parser.print_help()
        sys.exit(1)

    passed = check_size_gate(args.binary, args.partitions, args.max_size)
    sys.exit(0 if passed else 1)

if __name__ == "__main__":
    main()
