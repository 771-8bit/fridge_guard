use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TARGET_HARDWARE_ID: &str = "m5stack-nanoc6-v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemVer {
    pub fn parse(s: &str) -> Option<Self> {
        let clean = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let mut parts = clean.split('.');

        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;

        if parts.next().is_some() {
            return None; // Do not allow trailing segments for now
        }

        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtaManifest {
    pub schema_version: u32,
    pub version: String,
    pub asset_url: String,
    pub sha256: String,
    pub hardware: String,
    pub timestamp: u64,
    /// ECDSA P-256 / SHA-256 signature over `signing_message()`, hex-encoded raw `r || s` (64 bytes).
    pub signature: String,
    #[serde(default)]
    pub release_notes_url: Option<String>,
    #[serde(default)]
    pub min_firmware_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestValidationError {
    InvalidJson(String),
    UnsupportedSchemaVersion(u32),
    InvalidSemVer(String),
    IncompatibleHardware { expected: String, found: String },
    EqualOrDowngradeVersion { current: String, candidate: String },
    MinimumVersionNotMet { current: String, required: String },
    InvalidSha256Format(String),
    InsecureAssetUrl(String),
    InvalidPublicKey(String),
    InvalidSignatureFormat(String),
    SignatureVerificationFailed,
}

/// Domain-separation header of the signed OTA message. Must match `tools/release.py`.
pub const OTA_SIGNING_DOMAIN: &str = "fridge-guard-ota-v1";

/// Parse the PEM-encoded (SubjectPublicKeyInfo) P-256 OTA verification key embedded in firmware.
pub fn parse_verifying_key_pem(pem: &str) -> Result<VerifyingKey, ManifestValidationError> {
    VerifyingKey::from_public_key_pem(pem.trim())
        .map_err(|e| ManifestValidationError::InvalidPublicKey(e.to_string()))
}

impl OtaManifest {
    /// Canonical message covered by the release signature.
    ///
    /// Binds the image hash to its version, hardware target and minimum-version requirement,
    /// so a validly signed older image cannot be replayed under a newer version number and the
    /// requirement cannot be stripped. `asset_url` is not signed: the signed hash pins the image.
    pub fn signing_message(&self) -> String {
        format!(
            "{}\nhardware={}\nversion={}\nsha256={}\nmin_firmware_version={}",
            OTA_SIGNING_DOMAIN,
            self.hardware,
            self.version,
            self.sha256.to_ascii_lowercase(),
            self.min_firmware_version.as_deref().unwrap_or("")
        )
    }

    /// Verify the release signature against the trusted OTA verification key.
    pub fn verify_signature(&self, key: &VerifyingKey) -> Result<(), ManifestValidationError> {
        let sig_bytes = hex::decode(&self.signature)
            .map_err(|_| ManifestValidationError::InvalidSignatureFormat(self.signature.clone()))?;
        let signature = Signature::from_slice(&sig_bytes)
            .map_err(|_| ManifestValidationError::InvalidSignatureFormat(self.signature.clone()))?;
        key.verify(self.signing_message().as_bytes(), &signature)
            .map_err(|_| ManifestValidationError::SignatureVerificationFailed)
    }

    /// Parse and validate an OTA manifest against the running firmware version and hardware ID.
    ///
    /// A manifest is only returned if its signature verifies against `trusted_key`.
    pub fn parse_and_validate(
        json_str: &str,
        current_version_str: &str,
        device_hardware_id: &str,
        trusted_key: &VerifyingKey,
    ) -> Result<Self, ManifestValidationError> {
        let manifest: OtaManifest = serde_json::from_str(json_str)
            .map_err(|e| ManifestValidationError::InvalidJson(e.to_string()))?;

        if manifest.schema_version != 1 {
            return Err(ManifestValidationError::UnsupportedSchemaVersion(
                manifest.schema_version,
            ));
        }

        // Validate hardware compatibility
        if manifest.hardware != device_hardware_id {
            return Err(ManifestValidationError::IncompatibleHardware {
                expected: device_hardware_id.to_string(),
                found: manifest.hardware.clone(),
            });
        }

        // Validate asset URL (must be HTTPS)
        if !manifest.asset_url.starts_with("https://") {
            return Err(ManifestValidationError::InsecureAssetUrl(
                manifest.asset_url.clone(),
            ));
        }

        // Validate SHA-256 format (must be 64 hex characters)
        if manifest.sha256.len() != 64 || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ManifestValidationError::InvalidSha256Format(
                manifest.sha256.clone(),
            ));
        }

        // Parse SemVers
        let candidate_semver = SemVer::parse(&manifest.version)
            .ok_or_else(|| ManifestValidationError::InvalidSemVer(manifest.version.clone()))?;
        let current_semver = SemVer::parse(current_version_str).ok_or_else(|| {
            ManifestValidationError::InvalidSemVer(current_version_str.to_string())
        })?;

        // Strict update check: must be strictly greater than current version
        if candidate_semver <= current_semver {
            return Err(ManifestValidationError::EqualOrDowngradeVersion {
                current: current_version_str.to_string(),
                candidate: manifest.version.clone(),
            });
        }

        // Check optional minimum firmware version requirement
        if let Some(ref min_ver_str) = manifest.min_firmware_version {
            let min_semver = SemVer::parse(min_ver_str)
                .ok_or_else(|| ManifestValidationError::InvalidSemVer(min_ver_str.clone()))?;
            if current_semver < min_semver {
                return Err(ManifestValidationError::MinimumVersionNotMet {
                    current: current_version_str.to_string(),
                    required: min_ver_str.clone(),
                });
            }
        }

        // Authenticity: only a manifest signed by the release key may trigger an update
        manifest.verify_signature(trusted_key)?;

        Ok(manifest)
    }

    /// Verify binary payload SHA-256 against manifest hash
    pub fn verify_payload_sha256(&self, payload: &[u8]) -> bool {
        let mut verifier = OtaStreamVerifier::new(&self.sha256);
        verifier.update(payload);
        verifier.finalize_and_verify().0
    }
}

/// Streaming SHA-256 verifier for chunked OTA binary downloads
pub struct OtaStreamVerifier {
    hasher: Sha256,
    expected_hex: String,
}

impl OtaStreamVerifier {
    pub fn new(expected_hex: &str) -> Self {
        Self {
            hasher: Sha256::new(),
            expected_hex: expected_hex.to_string(),
        }
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
    }

    /// Finalize hash and verify against expected hash.
    /// Returns (matches, calculated_hex).
    pub fn finalize_and_verify(self) -> (bool, String) {
        let digest = self.hasher.finalize();
        let calculated = hex::encode(digest);
        let matches = calculated.eq_ignore_ascii_case(&self.expected_hex);
        (matches, calculated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::SigningKey;

    const UNSIGNED_MANIFEST_JSON: &str = r#"{
        "schema_version": 1,
        "version": "1.1.0",
        "asset_url": "https://github.com/771-8bit/fridge_guard/releases/download/v1.1.0/fridge_guard.bin",
        "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "hardware": "m5stack-nanoc6-v1",
        "timestamp": 1729900000,
        "signature": ""
    }"#;

    fn test_signing_key() -> SigningKey {
        SigningKey::from_slice(&[0x11u8; 32]).unwrap()
    }

    fn trusted_key() -> VerifyingKey {
        *test_signing_key().verifying_key()
    }

    fn sign_manifest(manifest: &mut OtaManifest, key: &SigningKey) {
        let sig: Signature = key.sign(manifest.signing_message().as_bytes());
        manifest.signature = hex::encode(sig.to_bytes());
    }

    fn signed_manifest_json(unsigned_json: &str, key: &SigningKey) -> String {
        let mut manifest: OtaManifest = serde_json::from_str(unsigned_json).unwrap();
        sign_manifest(&mut manifest, key);
        serde_json::to_string(&manifest).unwrap()
    }

    fn valid_manifest_json() -> String {
        signed_manifest_json(UNSIGNED_MANIFEST_JSON, &test_signing_key())
    }

    #[test]
    fn test_valid_manifest_upgrade() {
        let res = OtaManifest::parse_and_validate(
            &valid_manifest_json(),
            "1.0.0",
            TARGET_HARDWARE_ID,
            &trusted_key(),
        );
        assert!(res.is_ok());
        let manifest = res.unwrap();
        assert_eq!(manifest.version, "1.1.0");
    }

    #[test]
    fn test_reject_unsigned_manifest() {
        let res = OtaManifest::parse_and_validate(
            UNSIGNED_MANIFEST_JSON,
            "1.0.0",
            TARGET_HARDWARE_ID,
            &trusted_key(),
        );
        assert!(matches!(
            res,
            Err(ManifestValidationError::InvalidSignatureFormat(_))
        ));

        // Missing signature field entirely
        let no_field = UNSIGNED_MANIFEST_JSON.replace(",\n        \"signature\": \"\"", "");
        let res =
            OtaManifest::parse_and_validate(&no_field, "1.0.0", TARGET_HARDWARE_ID, &trusted_key());
        assert!(matches!(res, Err(ManifestValidationError::InvalidJson(_))));
    }

    #[test]
    fn test_reject_signature_from_untrusted_key() {
        let attacker_key = SigningKey::from_slice(&[0x22u8; 32]).unwrap();
        let json = signed_manifest_json(UNSIGNED_MANIFEST_JSON, &attacker_key);
        let res =
            OtaManifest::parse_and_validate(&json, "1.0.0", TARGET_HARDWARE_ID, &trusted_key());
        assert_eq!(
            res,
            Err(ManifestValidationError::SignatureVerificationFailed)
        );
    }

    #[test]
    fn test_reject_tampered_signed_fields() {
        let signed: OtaManifest = serde_json::from_str(&valid_manifest_json()).unwrap();

        // Swapped image hash
        let mut tampered = signed.clone();
        tampered.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        assert_eq!(
            tampered.verify_signature(&trusted_key()),
            Err(ManifestValidationError::SignatureVerificationFailed)
        );

        // Old signed image replayed under a bumped version number
        let mut tampered = signed.clone();
        tampered.version = "9.9.9".into();
        let json = serde_json::to_string(&tampered).unwrap();
        assert_eq!(
            OtaManifest::parse_and_validate(&json, "1.0.0", TARGET_HARDWARE_ID, &trusted_key()),
            Err(ManifestValidationError::SignatureVerificationFailed)
        );

        // Signed minimum-version requirement stripped
        let mut with_min = signed.clone();
        with_min.min_firmware_version = Some("1.0.0".into());
        sign_manifest(&mut with_min, &test_signing_key());
        let mut stripped = with_min.clone();
        stripped.min_firmware_version = None;
        assert_eq!(
            stripped.verify_signature(&trusted_key()),
            Err(ManifestValidationError::SignatureVerificationFailed)
        );
        assert!(with_min.verify_signature(&trusted_key()).is_ok());
    }

    /// Vector produced by `tools/release.py` (Python `cryptography`) with a throwaway key,
    /// guarding the cross-language signing message and signature encoding.
    #[test]
    fn test_verify_release_tool_signature_interop() {
        const RELEASE_TOOL_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE/Xuy1QJwHFWH85wIxjRf93oxJv2j
Nnr7yS2Pb2SEXVkMWSabCDNQCAhQTvMySlmS6ajfBwT2TFBIH7eaDQsXPA==
-----END PUBLIC KEY-----
";
        const RELEASE_TOOL_MANIFEST_JSON: &str = r#"{
            "schema_version": 1,
            "version": "0.1.0",
            "asset_url": "https://github.com/771-8bit/fridge_guard/releases/download/v0.1.0/fridge-guard-v0.1.0.bin",
            "sha256": "94b95289a037c8e825ee9aacca320fa68df095d497bc132c3c0e20f2f6025694",
            "hardware": "m5stack-nanoc6-v1",
            "timestamp": 1793097125,
            "signature": "a9abde19d3ed707eb9a69a78fdfc9976d9b3728f8c7c92121ec469148bf9a20a11f8c9e0950a76e1da146a85a9f2368e5de8eff2d8349e98c3cc481bc843b715",
            "release_notes_url": "https://github.com/771-8bit/fridge_guard/releases/tag/v0.1.0",
            "min_firmware_version": "0.0.5"
        }"#;

        let key = parse_verifying_key_pem(RELEASE_TOOL_PUBLIC_KEY_PEM).unwrap();
        let res = OtaManifest::parse_and_validate(
            RELEASE_TOOL_MANIFEST_JSON,
            "0.0.9",
            TARGET_HARDWARE_ID,
            &key,
        );
        assert_eq!(res.unwrap().version, "0.1.0");

        // Same manifest must be rejected by any other trusted key
        let res = OtaManifest::parse_and_validate(
            RELEASE_TOOL_MANIFEST_JSON,
            "0.0.9",
            TARGET_HARDWARE_ID,
            &trusted_key(),
        );
        assert_eq!(
            res,
            Err(ManifestValidationError::SignatureVerificationFailed)
        );
    }

    #[test]
    fn test_parse_verifying_key_pem() {
        assert!(matches!(
            parse_verifying_key_pem("not a key"),
            Err(ManifestValidationError::InvalidPublicKey(_))
        ));
    }

    #[test]
    fn test_reject_equal_or_downgrade() {
        // Equal version
        let res = OtaManifest::parse_and_validate(
            &valid_manifest_json(),
            "1.1.0",
            TARGET_HARDWARE_ID,
            &trusted_key(),
        );
        assert_eq!(
            res,
            Err(ManifestValidationError::EqualOrDowngradeVersion {
                current: "1.1.0".to_string(),
                candidate: "1.1.0".to_string()
            })
        );

        // Downgrade
        let res2 = OtaManifest::parse_and_validate(
            &valid_manifest_json(),
            "2.0.0",
            TARGET_HARDWARE_ID,
            &trusted_key(),
        );
        assert_eq!(
            res2,
            Err(ManifestValidationError::EqualOrDowngradeVersion {
                current: "2.0.0".to_string(),
                candidate: "1.1.0".to_string()
            })
        );
    }

    #[test]
    fn test_reject_incompatible_hardware() {
        let res = OtaManifest::parse_and_validate(
            &valid_manifest_json(),
            "1.0.0",
            "esp32c3-devkit",
            &trusted_key(),
        );
        assert_eq!(
            res,
            Err(ManifestValidationError::IncompatibleHardware {
                expected: "esp32c3-devkit".to_string(),
                found: "m5stack-nanoc6-v1".to_string()
            })
        );
    }

    #[test]
    fn test_reject_insecure_url() {
        let json = valid_manifest_json().replace("https://", "http://");
        let res =
            OtaManifest::parse_and_validate(&json, "1.0.0", TARGET_HARDWARE_ID, &trusted_key());
        assert!(matches!(
            res,
            Err(ManifestValidationError::InsecureAssetUrl(_))
        ));
    }

    #[test]
    fn test_verify_payload_sha256() {
        let manifest = OtaManifest {
            schema_version: 1,
            version: "1.1.0".to_string(),
            asset_url: "https://example.com/bin".to_string(),
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string(),
            hardware: "m5stack-nanoc6-v1".to_string(),
            timestamp: 0,
            signature: String::new(),
            release_notes_url: None,
            min_firmware_version: None,
        };

        // "abc" sha256 is ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert!(manifest.verify_payload_sha256(b"abc"));
        assert!(!manifest.verify_payload_sha256(b"wrong data"));
    }

    #[test]
    fn test_ota_stream_verifier_chunked() {
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut verifier = OtaStreamVerifier::new(expected);
        verifier.update(b"a");
        verifier.update(b"b");
        verifier.update(b"c");
        let (matched, hex_val) = verifier.finalize_and_verify();
        assert!(matched);
        assert_eq!(hex_val, expected);
    }
}
