mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::openssh_accepts;
use devnull::api::{Call, Client, Error, Reply};
use devnull::device::DeviceKey;
use devnull::model::Log;

struct Seen {
    head: String,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Answers one request with a canned response and hands back what the client sent.
fn serve(status: &str, body: &str) -> (String, JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let mut seen = Seen {
            head,
            body: String::new(),
        };
        let length = seen
            .header("Content-Length")
            .map_or(0, |length| length.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        seen.body = String::from_utf8(body).unwrap();
        stream.write_all(response.as_bytes()).unwrap();
        seen
    });
    (base, server)
}

fn signature_holds(key: &DeviceKey, seen: &Seen, method: &str, path: &str, dir: &str) -> bool {
    let message = format!(
        "{}\n{}\n{method}\n{path}\n{}",
        seen.header("X-Devnull-Timestamp").unwrap(),
        seen.header("X-Devnull-Nonce").unwrap(),
        seen.body
    );
    openssh_accepts(
        key,
        seen.header("X-SSH-Signature").unwrap(),
        &message,
        "devnull-api",
        dir,
    )
}

#[test]
fn redeeming_posts_the_code_signed_and_reads_the_user() {
    let (base, server) = serve(
        "200 OK",
        r#"{"data":{"id":7,"username":"ada","name":"Ada","bio":null},"unread_notifications_count":2}"#,
    );
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let account = client.redeem("the-code", "the-verifier").unwrap();
    let seen = server.join().unwrap();

    assert_eq!(account.me.username, "ada");
    assert_eq!(account.me.name.as_deref(), Some("Ada"));
    assert_eq!(account.unread, 2);
    assert!(seen.head.starts_with("POST /api/v1/devices HTTP/1.1\r\n"));
    assert_eq!(seen.header("Content-Type"), Some("application/json"));
    assert_eq!(seen.header("Accept"), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&seen.body).unwrap(),
        serde_json::json!({ "code": "the-code", "code_verifier": "the-verifier" })
    );
    assert!(signature_holds(
        client.key(),
        &seen,
        "POST",
        "/api/v1/devices",
        "api-redeem"
    ));
}

#[test]
fn me_is_a_signed_get_without_a_body() {
    let (base, server) = serve("200 OK", r#"{"data":{"username":"grace","name":null}}"#);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let account = client.me().unwrap();
    let seen = server.join().unwrap();

    assert_eq!(account.me.username, "grace");
    assert_eq!(account.unread, 0);
    assert!(seen.head.starts_with("GET /api/v1/me HTTP/1.1\r\n"));
    assert_eq!(seen.body, "");
    assert!(signature_holds(
        client.key(),
        &seen,
        "GET",
        "/api/v1/me",
        "api-me"
    ));
}

#[test]
fn an_unapproved_device_is_told_to_sign_in() {
    let (base, _server) = serve(
        "401 Unauthorized",
        r#"{"message":"This key isn't approved for the /dev/null app. Sign in again in the app.","code":"device_not_approved"}"#,
    );
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .me()
        .unwrap_err();

    assert!(error.needs_sign_in());
    assert!(error.to_string().starts_with("This key isn't approved"));
}

#[test]
fn a_spent_code_is_a_refusal_not_a_sign_in_prompt() {
    let (base, _server) = serve(
        "400 Bad Request",
        r#"{"message":"This sign-in link expired.","code":"invalid_grant"}"#,
    );
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .redeem("old", "verifier")
        .unwrap_err();

    assert!(!error.needs_sign_in());
    assert!(
        matches!(error, Error::Api { status: 400, code: Some(ref code), .. } if code == "invalid_grant")
    );
}

#[test]
fn a_log_page_is_read_from_its_route() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/log-trending.json"
    ))
    .unwrap();
    let (base, server) = serve("200 OK", &fixture);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let reply = client.call(&Call::Page(Log::Trending)).unwrap();
    let seen = server.join().unwrap();

    let Reply::Page(page) = reply else {
        panic!("a page call answers with a page");
    };
    assert_eq!(page.log, Log::Trending);
    assert!(page.is_ranked);
    assert!(
        seen.head
            .starts_with("GET /api/v1/log/trending HTTP/1.1\r\n")
    );
}

#[test]
fn a_throttle_says_how_long_to_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
            line.clear();
        }
        let body = r#"{"message":"Too Many Attempts.","code":"rate_limited"}"#;
        write!(
            stream,
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .me()
        .unwrap_err();

    assert!(matches!(
        error,
        Error::Api { status: 429, retry_after: Some(wait), .. } if wait == Duration::from_secs(7)
    ));
}

#[test]
fn a_page_that_is_not_the_api_is_unexpected() {
    let (base, _server) = serve("502 Bad Gateway", "<html>bad gateway</html>");
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .me()
        .unwrap_err();

    assert!(matches!(error, Error::Unexpected(502)));
}
