const OTA_PUBLIC_KEY_PATH: &str = "ota_verification_public.pem";

fn main() {
    println!("cargo:rerun-if-changed={OTA_PUBLIC_KEY_PATH}");
    check_ota_public_key();

    embuild::espidf::sysenv::output();
}

/// Fail the build early with a clear message instead of shipping firmware that can never verify OTA updates.
fn check_ota_public_key() {
    let pem = std::fs::read_to_string(OTA_PUBLIC_KEY_PATH).unwrap_or_else(|_| {
        panic!(
            "firmware/{OTA_PUBLIC_KEY_PATH} not found. Generate the OTA signing key pair with \
             `uv run tools/release.py --generate-key` (see docs/OTA_RELEASE.md)."
        )
    });
    if pem.contains("PRIVATE KEY") {
        panic!("firmware/{OTA_PUBLIC_KEY_PATH} contains a PRIVATE key. Remove it immediately and regenerate the key pair.");
    }
    if !pem.contains("-----BEGIN PUBLIC KEY-----") {
        panic!("firmware/{OTA_PUBLIC_KEY_PATH} is not a PEM SubjectPublicKeyInfo (\"BEGIN PUBLIC KEY\") file.");
    }
}
