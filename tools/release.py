#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "cryptography>=42",
# ]
# ///
"""
Fridge Guard - Local Release Orchestration Tool
Builds, validates, signs, hashes, size-gates, secret-scans, and generates OTA release metadata.

CRITICAL SECURITY RULES:
- Never print or leak private signing keys.
- Private signing keys MUST be located outside the git repository tree.
- Run secret scans across every publishable repository file and the release image.
- Refuse release if secrets, key files or local secret files would be published.
"""

import argparse
import getpass
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import (
    decode_dss_signature,
    encode_dss_signature,
)

TARGET_HARDWARE_ID = "m5stack-nanoc6-v1"

# Must match OTA_SIGNING_DOMAIN / OtaManifest::signing_message() in crates/fridge_guard_core/src/manifest.rs
OTA_SIGNING_DOMAIN = "fridge-guard-ota-v1"
# Public verification key embedded into the firmware via include_str! (committed to Git)
OTA_PUBLIC_KEY_PATH = os.path.join("firmware", "ota_verification_public.pem")
DEFAULT_PRIVATE_KEY_PATH = os.path.join("~", ".config", "fridge-guard", "keys", "ota_signing_private.pem")
# `version` here is FIRMWARE_VERSION in the firmware (env!("CARGO_PKG_VERSION"))
FIRMWARE_CARGO_TOML = os.path.join("firmware", "Cargo.toml")
# Optional non-interactive passphrase source (empty string = unencrypted key on --generate-key)
PASSPHRASE_ENV = "OTA_SIGNING_KEY_PASSPHRASE"
# Order of the NIST P-256 group, used for low-S normalization
P256_ORDER = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551

# Patterns that should NEVER appear in files that are (or would be) published
SUSPICIOUS_PATTERNS = [
    ("Private key block", re.compile(r"-----BEGIN\s+(?:[A-Z0-9]+\s+)*PRIVATE\s+KEY-----")),
    ("Bearer token", re.compile(r"Bearer\s+([A-Za-z0-9_\-.+/=]{40,})")),
    ("LINE user ID", re.compile(r"\b(U[0-9a-f]{32})\b")),
    ("Long token-like string", re.compile(r"(?<![A-Za-z0-9+/])([A-Za-z0-9+/]{100,}={0,2})")),
    (
        "Credential assignment",
        re.compile(
            r"(?i)\b(?:wifi_password|password|passwd|channel_secret|channel_access_token|"
            r"line_channel_token|access_token|auth_token|token|api_key|secret)\b[\"']?\s*[:=]\s*[\"']([^\"'\s]{8,})[\"']"
        ),
    ),
]

# Obvious placeholders used in docs, examples and tests
PLACEHOLDER_MARKERS = ("sample", "example", "dummy", "placeholder", "do_not_commit", "your_")
KNOWN_DUMMY_VALUES = {
    "U0123456789abcdef0123456789abcdef",
    "U1234567890abcdef1234567890abcdef",
}
# Put this marker on a line to acknowledge an intentional test fixture
ALLOW_MARKER = "secret-scan:allow"

# Files that must never be published, whatever their content
SECRET_FILE_NAMES = {".env", ".line_user_id", "secrets.json", "config.local.json"}
KEY_FILE_SUFFIXES = (".pem", ".key", ".p12", ".pfx", ".crt", ".cert")

IGNORED_SCAN_DIRS = {
    ".git",
    "target",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".local_provisioning",
}

def is_placeholder(value: str) -> bool:
    lowered = value.lower()
    return value in KNOWN_DUMMY_VALUES or any(m in lowered for m in PLACEHOLDER_MARKERS)

def scan_text_for_secrets(filepath: str, text: str) -> list:
    findings = []
    for line_no, line in enumerate(text.splitlines(), 1):
        if ALLOW_MARKER in line:
            continue
        for name, pat in SUSPICIOUS_PATTERNS:
            for m in pat.finditer(line):
                value = m.group(1) if m.groups() else m.group(0)
                # Hex-only strings are hashes and signatures (e.g. in release/manifest.json)
                if name == "Long token-like string" and re.fullmatch(r"[0-9a-fA-F]+", value):
                    continue
                if not is_placeholder(value):
                    findings.append((filepath, line_no, f"{name} detected"))
    return findings

def scan_file_for_secrets(filepath: str, include_binary: bool = False) -> list:
    try:
        with open(filepath, "rb") as f:
            data = f.read()
    except OSError as e:
        return [(filepath, 0, f"Could not read file: {e}")]
    if b"\0" in data[:8192]:
        if not include_binary:
            return []
        # Firmware images: look for embedded key material or identifiers in the raw bytes
        return scan_text_for_secrets(filepath, data.decode("latin-1"))
    return scan_text_for_secrets(filepath, data.decode("utf-8", errors="ignore"))

def is_allowed_public_key_file(filepath: str, root_dir: str) -> bool:
    """The embedded OTA verification key is the only key file allowed in the repository."""
    rel = os.path.normcase(os.path.normpath(os.path.relpath(filepath, root_dir)))
    if rel != os.path.normcase(os.path.normpath(OTA_PUBLIC_KEY_PATH)):
        return False
    try:
        with open(filepath, "r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
    except OSError:
        return False
    return "-----BEGIN PUBLIC KEY-----" in content and "PRIVATE" not in content

def list_publishable_files(root_dir: str) -> list:
    """Files Git would publish: tracked plus untracked-but-not-ignored. Falls back to a directory walk."""
    try:
        res = subprocess.run(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            cwd=root_dir, capture_output=True, check=True,
        )
        paths = [os.path.join(root_dir, p) for p in res.stdout.decode("utf-8").split("\0") if p]
        return [p for p in paths if os.path.isfile(p)]
    except (OSError, subprocess.CalledProcessError):
        print("[WARNING] git is unavailable; scanning every file under the directory instead.")
        files = []
        for dirpath, dirnames, filenames in os.walk(root_dir):
            dirnames[:] = [d for d in dirnames if d not in IGNORED_SCAN_DIRS]
            files.extend(os.path.join(dirpath, fn) for fn in filenames)
        return files

def check_file(path: str, root_dir: str, include_binary: bool = False) -> list:
    fn = os.path.basename(path)
    if fn.lower().endswith(KEY_FILE_SUFFIXES):
        if is_allowed_public_key_file(path, root_dir):
            return []
        return [(path, 1, "Key or certificate file would be published")]
    if fn in SECRET_FILE_NAMES or (fn.startswith(".env.") and fn != ".env.example"):
        return [(path, 1, "Local secret file would be published")]
    return scan_file_for_secrets(path, include_binary)

def run_secret_scan(root_dir: str, extra_files: tuple = ()) -> bool:
    print("\n--- Running Secret Scan over publishable files ---")
    all_findings = []
    for path in list_publishable_files(root_dir):
        all_findings.extend(check_file(path, root_dir))
    for path in extra_files:
        all_findings.extend(check_file(path, root_dir, include_binary=True))

    if all_findings:
        print("[FAIL] Secret scan found suspicious content:")
        for path, line, msg in all_findings:
            print(f"  {os.path.relpath(path, root_dir)}:{line} -> {msg}")
        print(f"If a finding is an intentional dummy value, add '{ALLOW_MARKER}' to that line.")
        return False

    print("[PASS] Secret scan clean. No secrets, key files or local secret files would be published.")
    return True

def compute_sha256(filepath: str) -> str:
    hasher = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            hasher.update(chunk)
    return hasher.hexdigest()

def check_git_clean(allow_dirty: bool = False) -> bool:
    try:
        res = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, check=True)
        if res.stdout.strip() and not allow_dirty:
            print("[FAIL] Git worktree is dirty. Please commit or stash changes, or use --allow-dirty.")
            return False
        return True
    except Exception as e:
        if not allow_dirty:
            print(f"[WARNING] Could not check git status: {e}")
            return False
        return True

def validate_semver(v: str) -> bool:
    clean = v.lstrip("v")
    return bool(re.match(r"^\d+\.\d+\.\d+$", clean))

def read_firmware_version() -> str | None:
    try:
        with open(FIRMWARE_CARGO_TOML, "rb") as f:
            return tomllib.load(f).get("package", {}).get("version")
    except (OSError, tomllib.TOMLDecodeError):
        return None

def is_inside_repo(path: str) -> bool:
    abs_path = os.path.normcase(os.path.abspath(os.path.expanduser(path)))
    repo_root = os.path.normcase(os.path.abspath("."))
    try:
        return os.path.commonpath([abs_path, repo_root]) == repo_root
    except ValueError:
        # Different drives on Windows
        return False

def generate_key_pair(private_path: str, public_path: str) -> None:
    private_path = os.path.expanduser(private_path)
    if is_inside_repo(private_path):
        print(f"CRITICAL SECURITY VIOLATION: Private key path '{private_path}' is inside repository directory!", file=sys.stderr)
        sys.exit(1)
    for path in (private_path, public_path):
        if os.path.exists(path):
            print(f"Error: '{path}' already exists. Refusing to overwrite an existing OTA key.", file=sys.stderr)
            print("Devices only accept updates signed by the key embedded in their current firmware.", file=sys.stderr)
            sys.exit(1)

    passphrase = os.getenv(PASSPHRASE_ENV)
    if passphrase is None:
        passphrase = getpass.getpass("Passphrase for the private key (empty = unencrypted): ")
        if passphrase and passphrase != getpass.getpass("Repeat passphrase: "):
            print("Error: Passphrases do not match.", file=sys.stderr)
            sys.exit(1)
    encryption = (
        serialization.BestAvailableEncryption(passphrase.encode("utf-8"))
        if passphrase
        else serialization.NoEncryption()
    )

    key = ec.generate_private_key(ec.SECP256R1())
    os.makedirs(os.path.dirname(os.path.abspath(private_path)), exist_ok=True)
    fd = os.open(private_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as f:
        f.write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, encryption))
    with open(public_path, "wb") as f:
        f.write(key.public_key().public_bytes(
            serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo
        ))

    print(f"[SUCCESS] Private signing key: {private_path}")
    print(f"[SUCCESS] Public verification key (commit this): {public_path}")
    print("Back up the private key offline. If it is lost, devices can no longer be updated over the air.")

def load_signing_key(path: str) -> ec.EllipticCurvePrivateKey:
    with open(os.path.expanduser(path), "rb") as f:
        data = f.read()
    try:
        key = serialization.load_pem_private_key(data, password=None)
    except TypeError:
        passphrase = os.getenv(PASSPHRASE_ENV) or getpass.getpass("Passphrase for OTA signing key: ")
        try:
            key = serialization.load_pem_private_key(data, password=passphrase.encode("utf-8"))
        except ValueError:
            print("Error: Could not decrypt OTA signing key (wrong passphrase?).", file=sys.stderr)
            sys.exit(1)
    if not isinstance(key, ec.EllipticCurvePrivateKey) or not isinstance(key.curve, ec.SECP256R1):
        print("Error: OTA signing key must be an ECDSA P-256 (prime256v1) private key.", file=sys.stderr)
        sys.exit(1)
    return key

def check_key_matches_firmware(key: ec.EllipticCurvePrivateKey, public_path: str) -> bool:
    try:
        with open(public_path, "rb") as f:
            embedded = serialization.load_pem_public_key(f.read())
    except (OSError, ValueError) as e:
        print(f"[FAIL] Could not load embedded verification key '{public_path}': {e}", file=sys.stderr)
        return False
    return embedded.public_numbers() == key.public_key().public_numbers()

def ota_signing_message(hardware: str, version: str, sha256_hex: str, min_firmware_version: str = "") -> bytes:
    """Must match OtaManifest::signing_message() in crates/fridge_guard_core/src/manifest.rs"""
    return (
        f"{OTA_SIGNING_DOMAIN}\nhardware={hardware}\nversion={version}\nsha256={sha256_hex.lower()}"
        f"\nmin_firmware_version={min_firmware_version}"
    ).encode("utf-8")

def sign_ota_message(key: ec.EllipticCurvePrivateKey, message: bytes) -> str:
    """Return a hex-encoded raw r || s (64 bytes) ECDSA P-256 / SHA-256 signature, low-S normalized."""
    r, s = decode_dss_signature(key.sign(message, ec.ECDSA(hashes.SHA256())))
    if s > P256_ORDER // 2:
        s = P256_ORDER - s
    # Self-check before publishing anything
    key.public_key().verify(encode_dss_signature(r, s), message, ec.ECDSA(hashes.SHA256()))
    return (r.to_bytes(32, "big") + s.to_bytes(32, "big")).hex()

def verify_manifest(manifest_path: str, public_path: str) -> bool:
    """Check a published manifest the way the firmware does: schema, then signature against the embedded key."""
    print(f"\n--- Verifying OTA manifest {manifest_path} ---")
    try:
        with open(manifest_path, "r", encoding="utf-8") as f:
            m = json.load(f)
        with open(public_path, "rb") as f:
            public_key = serialization.load_pem_public_key(f.read())
    except (OSError, ValueError) as e:
        print(f"[FAIL] {e}")
        return False

    problems = []
    if m.get("schema_version") != 1:
        problems.append("schema_version must be 1")
    if m.get("hardware") != TARGET_HARDWARE_ID:
        problems.append(f"hardware must be '{TARGET_HARDWARE_ID}'")
    if not validate_semver(str(m.get("version", ""))) or str(m.get("version", "")).startswith("v"):
        problems.append("version must be X.Y.Z")
    if not str(m.get("asset_url", "")).startswith("https://"):
        problems.append("asset_url must be HTTPS")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", str(m.get("sha256", ""))):
        problems.append("sha256 must be 64 hex characters")
    if not re.fullmatch(r"[0-9a-fA-F]{128}", str(m.get("signature", ""))):
        problems.append("signature must be 128 hex characters (raw r || s)")
    min_fw = m.get("min_firmware_version")
    if min_fw is not None and not re.fullmatch(r"\d+\.\d+\.\d+", str(min_fw)):
        problems.append("min_firmware_version must be X.Y.Z")
    if problems:
        for p in problems:
            print(f"[FAIL] {p}")
        return False

    sig = bytes.fromhex(m["signature"])
    der = encode_dss_signature(int.from_bytes(sig[:32], "big"), int.from_bytes(sig[32:], "big"))
    try:
        message = ota_signing_message(m["hardware"], m["version"], m["sha256"], min_fw or "")
        public_key.verify(der, message, ec.ECDSA(hashes.SHA256()))
    except InvalidSignature:
        print(f"[FAIL] Signature does not verify against {public_path}; devices would reject this manifest.")
        return False

    print(f"[PASS] Manifest v{m['version']} is well-formed and signed by the embedded release key.")
    return True

def main():
    parser = argparse.ArgumentParser(description="Fridge Guard Release Tool")
    parser.add_argument("--version", help="Release version (e.g. 1.0.0 or v1.0.0)")
    parser.add_argument("--binary", help="Path to prebuilt firmware .bin file")
    parser.add_argument("--key-path", help=f"Path to local private signing key (outside git repo, default: {DEFAULT_PRIVATE_KEY_PATH})")
    parser.add_argument("--public-key-path", default=OTA_PUBLIC_KEY_PATH, help=f"Public verification key embedded in firmware (default: {OTA_PUBLIC_KEY_PATH})")
    parser.add_argument("--generate-key", action="store_true", help="Generate a new OTA signing key pair and exit")
    parser.add_argument("--output-dir", default="release", help="Output directory for release assets (default: release; --dry-run uses a temporary directory)")
    parser.add_argument("--min-firmware-version", help="Oldest firmware (X.Y.Z) allowed to install this release (signed into the manifest)")
    parser.add_argument("--repo", default="771-8bit/fridge_guard", help="GitHub owner/repo (default: 771-8bit/fridge_guard)")
    parser.add_argument("--dry-run", action="store_true", help="Simulate release without modifying production files")
    parser.add_argument("--allow-dirty", action="store_true", help="Allow dirty git worktree (development/testing only)")
    parser.add_argument("--scan-only", action="store_true", help="Run secret scan only and exit")
    parser.add_argument("--verify-manifest", metavar="PATH", help="Verify a manifest's schema and signature against the embedded public key and exit")
    parser.add_argument("--gh-release", action="store_true", help="Publish release using GitHub CLI (gh release create)")
    args = parser.parse_args()

    key_path = args.key_path or os.getenv("OTA_SIGNING_PRIVATE_KEY") or DEFAULT_PRIVATE_KEY_PATH

    if args.generate_key:
        generate_key_pair(key_path, args.public_key_path)
        sys.exit(0)

    if args.verify_manifest:
        sys.exit(0 if verify_manifest(args.verify_manifest, args.public_key_path) else 1)

    # Secret scan
    if not run_secret_scan("."):
        print("\nERROR: Secret scan failed. Aborting release.", file=sys.stderr)
        sys.exit(1)

    if args.scan_only:
        sys.exit(0)

    if not args.version:
        print("Error: --version is required for release operations.", file=sys.stderr)
        sys.exit(1)

    # Version check
    semver = args.version.lstrip("v")
    if not validate_semver(semver):
        print(f"Error: Invalid semantic version format '{args.version}'. Expected X.Y.Z.", file=sys.stderr)
        sys.exit(1)
    min_fw = args.min_firmware_version.lstrip("v") if args.min_firmware_version else ""
    if min_fw and not validate_semver(min_fw):
        print(f"Error: Invalid --min-firmware-version '{args.min_firmware_version}'. Expected X.Y.Z.", file=sys.stderr)
        sys.exit(1)

    # The release version must equal the version compiled into the binary,
    # otherwise devices would re-install the same image on every OTA check.
    firmware_version = read_firmware_version()
    if firmware_version != semver:
        print(f"Error: --version {semver} does not match version \"{firmware_version}\" in {FIRMWARE_CARGO_TOML}.", file=sys.stderr)
        print(f"Bump the version in {FIRMWARE_CARGO_TOML}, rebuild the firmware, then release.", file=sys.stderr)
        sys.exit(1)

    # Git worktree check
    if not check_git_clean(args.allow_dirty):
        sys.exit(1)

    # Key path validation: Key MUST NOT be inside repository!
    if is_inside_repo(key_path):
        print(f"CRITICAL SECURITY VIOLATION: Private key '{key_path}' is inside repository directory!", file=sys.stderr)
        print("Move private signing keys outside the repository (e.g. ~/.config/fridge-guard/keys/).", file=sys.stderr)
        sys.exit(1)
    if not os.path.exists(os.path.expanduser(key_path)):
        print(f"Error: OTA signing key '{key_path}' not found. Create one with --generate-key.", file=sys.stderr)
        sys.exit(1)
    signing_key = load_signing_key(key_path)
    if not check_key_matches_firmware(signing_key, args.public_key_path):
        print(f"Error: Signing key does not match the verification key embedded in firmware ({args.public_key_path}).", file=sys.stderr)
        print("Devices would reject this release. Aborting.", file=sys.stderr)
        sys.exit(1)

    # Output directory (a dry run never touches the real release directory)
    if args.dry_run:
        args.output_dir = tempfile.mkdtemp(prefix="fridge-guard-dry-run-")
        print(f"[DRY-RUN] Writing release candidate files to {args.output_dir}")
    os.makedirs(args.output_dir, exist_ok=True)
    release_bin_name = f"fridge-guard-v{semver}.bin"
    output_bin_path = os.path.join(args.output_dir, release_bin_name)

    # Handle binary
    if args.binary:
        if not os.path.exists(args.binary):
            print(f"Error: Binary '{args.binary}' not found.", file=sys.stderr)
            sys.exit(1)
        shutil.copy2(args.binary, output_bin_path)
    else:
        # Create dummy/signed placeholder binary if in dry-run
        if args.dry_run:
            print(f"[DRY-RUN] Creating development release candidate binary: {output_bin_path}")
            with open(output_bin_path, "wb") as f:
                f.write(f"M5STACK_NANOC6_FIRMWARE_V{semver}\n".encode("utf-8"))
                f.write(b"\x00" * 1024)
        else:
            print("Error: No --binary specified and not in --dry-run mode.", file=sys.stderr)
            sys.exit(1)

    # Size gate check
    sys.path.insert(0, os.path.dirname(__file__))
    import size_gate
    if not size_gate.check_size_gate(output_bin_path, "partitions.csv"):
        print("\nERROR: Firmware binary failed OTA partition size gate. Aborting release.", file=sys.stderr)
        sys.exit(1)

    # The release image must not carry key material or identifiers either
    if not run_secret_scan(".", extra_files=(output_bin_path,)):
        print("\nERROR: Secret scan of the release image failed. Aborting release.", file=sys.stderr)
        sys.exit(1)

    # Compute SHA-256 and sign (hardware, version, sha256, min_firmware_version)
    file_sha256 = compute_sha256(output_bin_path)
    signature = sign_ota_message(signing_key, ota_signing_message(TARGET_HARDWARE_ID, semver, file_sha256, min_fw))

    # Construct manifest
    asset_url = f"https://github.com/{args.repo}/releases/download/v{semver}/{release_bin_name}"
    manifest = {
        "schema_version": 1,
        "version": semver,
        "asset_url": asset_url,
        "sha256": file_sha256,
        "hardware": TARGET_HARDWARE_ID,
        "timestamp": int(time.time()),
        "signature": signature,
        "release_notes_url": f"https://github.com/{args.repo}/releases/tag/v{semver}"
    }
    if min_fw:
        manifest["min_firmware_version"] = min_fw

    manifest_path = os.path.join(args.output_dir, "manifest.json")
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    if args.dry_run:
        print("\n[DRY-RUN] Manifest generated:")
        print(json.dumps(manifest, indent=2))
    else:
        print(f"\n[SUCCESS] Generated release manifest: {manifest_path}")

    print("\n==================================================")
    print(f" Release v{semver} Prepared Successfully!")
    print(f"   Binary:  {output_bin_path}")
    print(f"   SHA-256: {file_sha256}")
    print(f"   URL:     {asset_url}")
    print("==================================================")

    # Optional GitHub CLI release
    if args.gh_release and not args.dry_run:
        print("\nPublishing via GitHub CLI...")
        gh_cmd = [
            "gh", "release", "create", f"v{semver}",
            output_bin_path,
            "--title", f"Release v{semver}",
            "--notes", f"Automated release for {TARGET_HARDWARE_ID}."
        ]
        subprocess.run(gh_cmd, check=True)
        print("GitHub Release created and asset uploaded!")

if __name__ == "__main__":
    main()
