use std::process::Command;

use devnull::signin::{Attempt, Callback, Inbox, REDIRECT_URI};

fn attempt() -> Attempt {
    Attempt {
        verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned(),
        state: "0123456789abcdef0123456789abcdef".to_owned(),
    }
}

fn query(url: &str) -> Vec<(String, String)> {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap();
            let mut decoded = Vec::new();
            let mut bytes = value.bytes();
            while let Some(byte) = bytes.next() {
                if byte == b'%' {
                    let hex: String = [bytes.next().unwrap(), bytes.next().unwrap()]
                        .map(char::from)
                        .iter()
                        .collect();
                    decoded.push(u8::from_str_radix(&hex, 16).unwrap());
                } else {
                    decoded.push(byte);
                }
            }
            (name.to_owned(), String::from_utf8(decoded).unwrap())
        })
        .collect()
}

#[test]
fn the_challenge_is_the_rfc_7636_s256_of_the_verifier() {
    assert_eq!(
        attempt().challenge(),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn a_random_attempt_meets_the_server_rules() {
    let attempt = Attempt::random();
    let unreserved = |b: u8| b.is_ascii_alphanumeric() || b"._~-".contains(&b);

    assert!((43..=128).contains(&attempt.verifier.len()));
    assert!(attempt.verifier.bytes().all(unreserved));
    assert_eq!(attempt.challenge().len(), 43);
    assert_eq!(attempt.state.len(), 32);
    assert_ne!(attempt.state, Attempt::random().state);
}

#[test]
fn the_authorize_url_carries_every_parameter_intact() {
    let fingerprint = "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s";
    let url = attempt().authorize_url(
        "https://dev-null.sh",
        fingerprint,
        "devnull-tui on my box & co",
    );

    assert!(url.starts_with("https://dev-null.sh/mobile/authorize?"));
    assert_eq!(
        query(&url),
        [
            ("fingerprint", fingerprint),
            ("device_name", "devnull-tui on my box & co"),
            (
                "code_challenge",
                "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
            ),
            ("code_challenge_method", "S256"),
            ("state", "0123456789abcdef0123456789abcdef"),
            ("redirect_uri", REDIRECT_URI),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
    );
}

#[test]
fn an_approval_gives_back_the_code() {
    let url = format!("{REDIRECT_URI}?code=a%2Bb~c&state=0123456789abcdef0123456789abcdef");
    assert_eq!(
        attempt().callback(&url),
        Some(Callback::Approved("a+b~c".to_owned()))
    );
}

#[test]
fn a_cancel_is_a_denial() {
    let url = format!("{REDIRECT_URI}?error=access_denied&state=0123456789abcdef0123456789abcdef");
    assert_eq!(attempt().callback(&url), Some(Callback::Denied));
}

#[test]
fn a_redirect_from_another_attempt_is_ignored() {
    for url in [
        format!("{REDIRECT_URI}?code=abc&state=ffffffffffffffffffffffffffffffff"),
        format!("{REDIRECT_URI}?code=abc"),
        "devnull://elsewhere?code=abc&state=0123456789abcdef0123456789abcdef".to_owned(),
        format!("{REDIRECT_URI}?code=a%zz&state=0123456789abcdef0123456789abcdef"),
    ] {
        assert_eq!(attempt().callback(&url), None, "{url}");
    }
}

#[test]
fn the_redirect_handler_hands_the_link_to_the_waiting_app() {
    let runtime = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    // SAFETY: no other test in this binary reads the environment.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &runtime) };
    let inbox = Inbox::bind().unwrap();
    let link = format!("{REDIRECT_URI}?code=abc&state=0123456789abcdef0123456789abcdef");
    assert_eq!(inbox.poll().unwrap(), None);

    let handler = Command::new(env!("CARGO_BIN_EXE_devnull"))
        .args(["--redirect", &link])
        .env("XDG_RUNTIME_DIR", &runtime)
        .status()
        .unwrap();

    assert!(handler.success());
    assert_eq!(inbox.poll().unwrap(), Some(link));
    assert_eq!(inbox.poll().unwrap(), None);
}
