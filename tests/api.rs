mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::openssh_accepts;
use devnull::api::{Call, Client, Error, Reply};
use devnull::device::DeviceKey;
use devnull::draft::Target;
use devnull::model::{CommitId, Filter, Log, NotificationId};

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
fn an_older_page_passes_its_cursor_back_in_the_signed_path() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/log-latest.json"
    ))
    .unwrap();
    let (base, server) = serve("200 OK", &fixture);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    client
        .call(&Call::Older {
            log: Log::Latest,
            cursor: "eyJpZCI6MX0+/=".to_owned(),
        })
        .unwrap();
    let seen = server.join().unwrap();

    assert!(
        seen.head
            .starts_with("GET /api/v1/log/latest?cursor=eyJpZCI6MX0%2B%2F%3D HTTP/1.1\r\n"),
        "{}",
        seen.head
    );
    assert!(signature_holds(
        client.key(),
        &seen,
        "GET",
        "/api/v1/log/latest?cursor=eyJpZCI6MX0%2B%2F%3D",
        "api-older"
    ));
}

#[test]
fn a_check_for_new_commits_names_the_newest_listed_and_reads_the_count() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/log-following.json"
    ))
    .unwrap()
    .replace(
        r#""is_ranked": false"#,
        r#""is_ranked": false, "new_commits_count": 2"#,
    );
    let (base, server) = serve("200 OK", &fixture);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let reply = client
        .call(&Call::Since {
            log: Log::Following,
            since: CommitId("9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61".to_owned()),
        })
        .unwrap();
    let seen = server.join().unwrap();

    let Reply::Page(page) = reply else {
        panic!("a page");
    };
    assert_eq!(page.new_commits_count, Some(2));
    let path = "/api/v1/log/following?since=9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61";
    assert!(
        seen.head.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
        "{}",
        seen.head
    );
    assert!(signature_holds(
        client.key(),
        &seen,
        "GET",
        path,
        "api-since"
    ));
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

#[test]
fn json_of_the_wrong_shape_names_the_field() {
    let (base, _server) = serve("200 OK", r#"{"data":{"name":"Ada"}}"#);
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .me()
        .unwrap_err();

    assert!(
        matches!(&error, Error::WrongShape(detail) if detail.contains("username")),
        "{error:?}"
    );
}

#[test]
fn a_422_names_the_first_field_message_without_the_count() {
    let (base, _server) = serve(
        "422 Unprocessable Content",
        r#"{"message":"The body field must not be greater than 4000 characters. (and 1 more error)","errors":{"body":["The body field must not be greater than 4000 characters."],"parent_id":["The selected parent id is invalid."]}}"#,
    );
    let post = Call::Post {
        target: Target::New,
        body: "x".to_owned(),
    };
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .call(&post)
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "The body field must not be greater than 4000 characters."
    );
    assert!(!error.may_have_landed());
}

#[test]
fn a_dropped_connection_may_have_landed_and_a_refused_one_did_not() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
    });
    let post = Call::Post {
        target: Target::New,
        body: "x".to_owned(),
    };
    let dropped = Client::new(base, DeviceKey::generate().unwrap())
        .call(&post)
        .unwrap_err();
    server.join().unwrap();
    assert!(dropped.may_have_landed(), "{dropped:?}");

    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let refused = Client::new(base, DeviceKey::generate().unwrap())
        .call(&post)
        .unwrap_err();
    assert!(!refused.may_have_landed(), "{refused:?}");
}

#[test]
fn notifications_always_name_their_filter_and_pass_the_cursor_in_the_signed_path() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications.json"
    ))
    .unwrap();
    let (base, server) = serve("200 OK", &fixture);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let reply = client
        .call(&Call::Notifications {
            filter: Filter::Stars,
            cursor: Some("eyJpZCI6MX0+/=".to_owned()),
        })
        .unwrap();
    let seen = server.join().unwrap();

    let Reply::Notifications(page) = reply else {
        panic!("a notifications call answers with a notifications page");
    };
    assert_eq!(page.filter, Filter::Stars);
    let path = "/api/v1/notifications?type=stars&cursor=eyJpZCI6MX0%2B%2F%3D";
    assert!(
        seen.head.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
        "{}",
        seen.head
    );
    assert!(signature_holds(
        client.key(),
        &seen,
        "GET",
        path,
        "api-notifications"
    ));
}

#[test]
fn a_group_is_read_from_the_route_of_its_notification() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications-group.json"
    ))
    .unwrap();
    let (base, server) = serve("200 OK", &fixture);
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let reply = client
        .call(&Call::Group {
            id: NotificationId("9e000000-0000-4000-8000-000000000002".to_owned()),
            filter: Filter::All,
            cursor: None,
        })
        .unwrap();
    let seen = server.join().unwrap();

    assert!(matches!(reply, Reply::Group(page) if page.events.len() == 5));
    assert!(
        seen.head.starts_with(
            "GET /api/v1/notifications/9e000000-0000-4000-8000-000000000002/group?type=all HTTP/1.1\r\n"
        ),
        "{}",
        seen.head
    );
}
