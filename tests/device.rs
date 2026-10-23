mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::{openssh_accepts, scratch};
use ttyl::device::{DeviceKey, Nonce};

#[test]
fn openssh_verifies_the_request_the_server_rebuilds() {
    let key = DeviceKey::generate().unwrap();
    let body = r#"{"code":"abc","code_verifier":"xyz"}"#;
    let signed = key.sign(
        "POST",
        "/api/v1/devices?x=1",
        body,
        1_790_000_000,
        Nonce::random(),
    );
    let message = format!(
        "1790000000\n{}\nPOST\n/api/v1/devices?x=1\n{body}",
        signed.nonce
    );

    assert!(openssh_accepts(
        &key,
        &signed.signature,
        &message,
        "devnull-api",
        "verify-ok"
    ));
    assert!(!openssh_accepts(
        &key,
        &signed.signature,
        &message.replace("abc", "abd"),
        "devnull-api",
        "verify-tampered"
    ));
    assert!(!openssh_accepts(
        &key,
        &signed.signature,
        &message,
        "file",
        "verify-namespace"
    ));
}

#[test]
fn a_get_signs_an_empty_body() {
    let key = DeviceKey::generate().unwrap();
    let signed = key.sign("GET", "/api/v1/me", "", 1_790_000_000, Nonce::random());
    let message = format!("1790000000\n{}\nGET\n/api/v1/me\n", signed.nonce);

    assert!(openssh_accepts(
        &key,
        &signed.signature,
        &message,
        "devnull-api",
        "verify-get"
    ));
}

#[test]
fn headers_are_single_line_values_the_server_accepts() {
    let key = DeviceKey::generate().unwrap();
    let signed = key.sign("GET", "/api/v1/me", "", 1_790_000_000, Nonce::random());
    let headers = signed.headers();

    assert_eq!(headers[0], ("X-Devnull-Timestamp", "1790000000"));
    assert_eq!(headers[1].0, "X-Devnull-Nonce");
    assert_eq!(headers[1].1.len(), 32);
    assert!(
        headers[1]
            .1
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_eq!(headers[2].0, "X-SSH-Signature");
    assert!(!headers[2].1.contains(char::is_whitespace));
}

#[test]
fn nonces_differ() {
    assert_ne!(Nonce::random().to_string(), Nonce::random().to_string());
}

#[test]
fn the_fingerprint_matches_openssh() {
    let key = DeviceKey::generate().unwrap();
    let dir = scratch("fingerprint");
    fs::write(dir.join("key.pub"), key.public_key()).unwrap();
    let output = Command::new("ssh-keygen")
        .args(["-l", "-E", "sha256", "-f"])
        .arg(dir.join("key.pub"))
        .output()
        .unwrap();
    let listed = String::from_utf8(output.stdout).unwrap();
    let fingerprint = key.fingerprint();

    assert_eq!(listed.split_whitespace().nth(1), Some(fingerprint.as_str()));
    let digest = fingerprint.strip_prefix("SHA256:").unwrap();
    assert_eq!(digest.len(), 43);
    assert!(
        digest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    );
}

#[test]
fn a_saved_key_is_private_and_loads_back() {
    let key = DeviceKey::generate().unwrap();
    let path = scratch("save").join("device_ed25519");
    key.save(&path).unwrap();

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        DeviceKey::load(&path).unwrap().fingerprint(),
        key.fingerprint()
    );
}

#[test]
fn the_first_run_makes_a_private_key_and_later_runs_reuse_it() {
    let path = scratch("open").join("ttyl/device_ed25519");
    let first = DeviceKey::open(&path).unwrap();

    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        DeviceKey::open(&path).unwrap().fingerprint(),
        first.fingerprint()
    );
}
