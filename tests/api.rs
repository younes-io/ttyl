mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{openssh_accepts, scratch};
use ttyl::api::{Call, Client, Error, Reply};
use ttyl::attach::{self, Attachment, Image, Poll};
use ttyl::cache::Cache;
use ttyl::device::DeviceKey;
use ttyl::draft::Target;
use ttyl::model::{Circle, CommitId, Filter, Log, NotificationId, Reason};

struct Seen {
    head: String,
    body: String,
    raw: Vec<u8>,
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
            raw: Vec::new(),
        };
        let length = seen
            .header("Content-Length")
            .map_or(0, |length| length.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        seen.body = String::from_utf8_lossy(&body).into_owned();
        seen.raw = body;
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
        attachment: Attachment::None,
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
fn a_poll_goes_with_the_body_and_lasts_in_minutes() {
    let (base, server) = serve(
        "422 Unprocessable Content",
        r#"{"message":"no","errors":{"body":["no"]}}"#,
    );
    let post = Call::Post {
        target: Target::New,
        body: "which editor?".to_owned(),
        attachment: Attachment::Poll(Poll {
            options: vec!["vim".to_owned(), "emacs".to_owned()],
            duration: attach::Duration::ThreeDays,
        }),
    };
    Client::new(base, DeviceKey::generate().unwrap())
        .call(&post)
        .unwrap_err();
    let seen = server.join().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(sent["body"], "which editor?");
    assert_eq!(
        sent["poll"],
        serde_json::json!({"options": ["vim", "emacs"], "duration": 4320})
    );
}

/// Each part of a multipart body, as its field name, file name and type, and its bytes.
fn parts(seen: &Seen) -> Vec<(String, Vec<u8>)> {
    let boundary = (seen.header("Content-Type").unwrap())
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let delimiter = format!("--{boundary}");
    let raw = &seen.raw;
    let mut starts: Vec<usize> = (0..raw.len())
        .filter(|&at| raw[at..].starts_with(delimiter.as_bytes()))
        .collect();
    assert!(raw[starts.pop().unwrap()..].starts_with(format!("{delimiter}--\r\n").as_bytes()));
    let ends = starts
        .iter()
        .skip(1)
        .copied()
        .chain([raw.len() - delimiter.len() - 4]);
    (starts.iter().zip(ends))
        .map(|(&start, end)| {
            let part = &raw[start + delimiter.len() + 2..end - 2];
            let split = (0..part.len())
                .find(|&at| part[at..].starts_with(b"\r\n\r\n"))
                .unwrap();
            let head = String::from_utf8(part[..split].to_vec()).unwrap();
            let name = head
                .split("name=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let file = head
                .split("filename=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let mime = head.split("Content-Type: ").nth(1).unwrap();
            (format!("{name} {file} {mime}"), part[split + 4..].to_vec())
        })
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    let mut child = std::process::Command::new("sha256sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let out = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
    out.split_whitespace().next().unwrap().to_owned()
}

#[test]
fn images_go_as_multipart_signed_over_their_hashes() {
    let dir = common::scratch("api-images");
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x02\0\0\0\x03".to_vec();
    png.extend([0xff, 0x00, 0x0d, 0x0a, 0x2d, 0x2d]);
    let webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0\x09\0\0\x04\0\0".to_vec();
    fs::write(dir.join("my screenshot.png"), &png).unwrap();
    fs::write(dir.join("cat.webp"), &webp).unwrap();
    let (base, server) = serve(
        "422 Unprocessable Content",
        r#"{"message":"no","errors":{"body":["no"]}}"#,
    );
    let post = Call::Post {
        target: Target::New,
        body: "look".to_owned(),
        attachment: Attachment::Images(vec![
            Image {
                path: dir.join("my screenshot.png"),
                description: "a terminal".to_owned(),
            },
            Image {
                path: dir.join("cat.webp"),
                description: String::new(),
            },
        ]),
    };
    let client = Client::new(base, DeviceKey::generate().unwrap());
    client.call(&post).unwrap_err();
    let seen = server.join().unwrap();

    let parts = parts(&seen);
    let heads: Vec<&str> = parts.iter().map(|(head, _)| head.as_str()).collect();
    assert_eq!(
        heads,
        [
            "payload payload.json application/json",
            "images[] image-1.png image/png",
            "images[] image-2.webp image/webp",
        ],
        "the payload comes first and no local file name leaves the machine"
    );
    assert_eq!(parts[1].1, png);
    assert_eq!(parts[2].1, webp);
    let payload = String::from_utf8(parts[0].1.clone()).unwrap();
    let sent: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(sent["body"], "look");
    assert_eq!(
        sent["image_descriptions"],
        serde_json::json!(["a terminal", ""])
    );

    let message = format!(
        "{}\n{}\nPOST\n/api/v1/commits\n{},{}\n{payload}",
        seen.header("X-Devnull-Timestamp").unwrap(),
        seen.header("X-Devnull-Nonce").unwrap(),
        sha256(&png),
        sha256(&webp),
    );
    let signature = seen.header("X-SSH-Signature").unwrap();
    assert!(openssh_accepts(
        client.key(),
        signature,
        &message,
        "devnull-api-images",
        "api-images-sig"
    ));
    assert!(!openssh_accepts(
        client.key(),
        signature,
        &message,
        "devnull-api",
        "api-images-plain"
    ));
}

#[test]
fn an_image_gone_from_disk_is_named_and_nothing_is_sent() {
    let post = Call::Post {
        target: Target::New,
        body: String::new(),
        attachment: Attachment::Images(vec![Image {
            path: "/nonexistent/ttyl/shot.png".into(),
            description: String::new(),
        }]),
    };
    let error = Client::new("http://127.0.0.1:9", DeviceKey::generate().unwrap())
        .call(&post)
        .unwrap_err();
    assert!(
        error.to_string().starts_with("can't read shot.png: "),
        "{error}"
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
        attachment: Attachment::None,
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

fn fixture(name: &str) -> serde_json::Value {
    let path = format!("{}/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Makes one call against a server answering 200 with `response`, and hands back the reply and the request.
fn exchange(call: &Call, response: &serde_json::Value) -> (Reply, Seen) {
    let (base, server) = serve("200 OK", &response.to_string());
    let reply = Client::new(base, DeviceKey::generate().unwrap())
        .call(call)
        .unwrap();
    (reply, server.join().unwrap())
}

fn request_line(seen: &Seen) -> &str {
    seen.head.lines().next().unwrap()
}

fn sent(seen: &Seen) -> serde_json::Value {
    serde_json::from_str(&seen.body).unwrap()
}

fn focus() -> serde_json::Value {
    serde_json::json!({ "data": fixture("commit-quote")["data"] })
}

fn person() -> serde_json::Value {
    let mut person = fixture("log-latest")["data"][0]["author"].clone();
    person["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
    person
}

#[test]
fn a_commit_and_its_replies_are_read_from_its_routes() {
    let id = CommitId("7a000000-0000-4000-8000-00000000000a".to_owned());
    let (reply, seen) = exchange(&Call::Commit(id.clone()), &fixture("commit-quote"));
    assert!(matches!(reply, Reply::Thread(page) if page.commit.commit.id == id));
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/commits/7a000000-0000-4000-8000-00000000000a HTTP/1.1"
    );

    let call = Call::Replies {
        id,
        cursor: Some("eyJpZCI6MX0+/=".to_owned()),
    };
    let (reply, seen) = exchange(&call, &fixture("replies-paged"));
    assert!(matches!(reply, Reply::Replies(page) if page.replies.len() == 3));
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/commits/7a000000-0000-4000-8000-00000000000a/replies?cursor=eyJpZCI6MX0%2B%2F%3D HTTP/1.1"
    );
}

#[test]
fn starring_and_following_post_to_turn_on_and_delete_to_turn_off() {
    let id = CommitId("7a000000-0000-4000-8000-00000000000a".to_owned());
    for (on, method) in [(true, "POST"), (false, "DELETE")] {
        let star = Call::Star { id: id.clone(), on };
        let (reply, seen) = exchange(&star, &focus());
        assert!(matches!(reply, Reply::Commit(focus) if focus.card.commit.id == id));
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/commits/{}/star HTTP/1.1", id.0)
        );

        let follow = Call::Follow {
            username: "ada".to_owned(),
            on,
        };
        let (reply, seen) = exchange(&follow, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/users/ada/follow HTTP/1.1")
        );
    }
}

#[test]
fn a_profile_and_its_lists_are_read_from_the_user_routes() {
    let profile = serde_json::json!({
        "data": person(),
        "is_following": true,
        "follows_you": false,
        "pinned_commit": null,
    });
    let (reply, seen) = exchange(&Call::User("ada".to_owned()), &profile);
    assert!(
        matches!(reply, Reply::User(profile) if profile.person.user.username == "ada" && profile.is_following)
    );
    assert_eq!(request_line(&seen), "GET /api/v1/users/ada HTTP/1.1");

    let rows = serde_json::json!({
        "data": [fixture("log-latest")["data"][0]],
        "meta": { "next_cursor": "next" },
    });
    let commits = |replies, cursor: Option<&str>| Call::Commits {
        username: "ada".to_owned(),
        replies,
        cursor: cursor.map(str::to_owned),
    };
    let routes = [
        (commits(false, None), "/api/v1/users/ada/commits?replies=0"),
        (
            commits(true, Some("a+b")),
            "/api/v1/users/ada/commits?replies=1&cursor=a%2Bb",
        ),
        (Call::Starred { cursor: None }, "/api/v1/starred"),
        (
            Call::Starred {
                cursor: Some("a+b".to_owned()),
            },
            "/api/v1/starred?cursor=a%2Bb",
        ),
    ];
    for (call, path) in routes {
        let (reply, seen) = exchange(&call, &rows);
        assert!(
            matches!(&reply, Reply::Rows(page) if page.rows.len() == 1 && page.next_cursor.as_deref() == Some("next")),
            "{call:?}"
        );
        assert_eq!(request_line(&seen), format!("GET {path} HTTP/1.1"));
    }

    let people = serde_json::json!({ "data": [person()], "meta": { "next_cursor": null } });
    for (circle, path) in [
        (
            Circle::Followers,
            "/api/v1/users/ada/followers?cursor=a%2Bb",
        ),
        (
            Circle::Following,
            "/api/v1/users/ada/following?cursor=a%2Bb",
        ),
    ] {
        let call = Call::People {
            username: "ada".to_owned(),
            circle,
            cursor: Some("a+b".to_owned()),
        };
        let (reply, seen) = exchange(&call, &people);
        assert!(matches!(reply, Reply::People(page) if page.people.len() == 1));
        assert_eq!(request_line(&seen), format!("GET {path} HTTP/1.1"));
    }
}

#[test]
fn acting_on_a_commit_sends_what_the_server_needs() {
    let id = CommitId("7a000000-0000-4000-8000-00000000000a".to_owned());
    let route = format!("/api/v1/commits/{}", id.0);

    let (reply, seen) = exchange(&Call::Delete(id.clone()), &serde_json::json!({}));
    assert!(matches!(reply, Reply::Done));
    assert_eq!(request_line(&seen), format!("DELETE {route} HTTP/1.1"));
    assert_eq!(seen.body, "");

    let mut pastel = focus();
    pastel["pasteis_remaining"] = serde_json::json!(2);
    let (reply, seen) = exchange(&Call::Pastel(id.clone()), &pastel);
    assert!(matches!(reply, Reply::Pastel { commit, remaining: 2 } if commit.card.commit.id == id));
    assert_eq!(
        request_line(&seen),
        format!("POST {route}/pasteis HTTP/1.1")
    );

    let vote = Call::Vote {
        id: id.clone(),
        option: "tabs".to_owned(),
    };
    let (reply, seen) = exchange(&vote, &focus());
    assert!(matches!(reply, Reply::Commit(_)));
    assert_eq!(
        request_line(&seen),
        format!("POST {route}/poll/votes HTTP/1.1")
    );
    assert_eq!(sent(&seen), serde_json::json!({ "option": "tabs" }));

    for (details, expected) in [
        (None, serde_json::Value::Null),
        (
            Some("links to malware"),
            serde_json::json!("links to malware"),
        ),
    ] {
        let report = Call::Report {
            id: id.clone(),
            reason: Reason::Spam,
            details: details.map(str::to_owned),
        };
        let (reply, seen) = exchange(&report, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("POST {route}/reports HTTP/1.1")
        );
        assert_eq!(
            sent(&seen),
            serde_json::json!({ "reason": "spam", "details": expected })
        );
    }
}

#[test]
fn a_preview_posts_the_draft_and_reads_its_segments() {
    let segments = serde_json::json!({
        "data": { "body_segments": fixture("commit-quote")["data"]["body_segments"] },
    });
    let (reply, seen) = exchange(&Call::Preview("hello **world**".to_owned()), &segments);
    assert!(matches!(reply, Reply::Preview(segments) if segments.len() == 1));
    assert_eq!(request_line(&seen), "POST /api/v1/commits/preview HTTP/1.1");
    assert_eq!(
        sent(&seen),
        serde_json::json!({ "body": "hello **world**" })
    );
}

#[test]
fn a_reply_names_its_parent_and_a_quote_names_its_commit() {
    let id = CommitId("7a000000-0000-4000-8000-00000000000a".to_owned());
    for (target, parent, quoted) in [
        (Target::New, None, None),
        (Target::Reply(id.clone()), Some(&id.0), None),
        (Target::Quote(id.clone()), None, Some(&id.0)),
    ] {
        let post = Call::Post {
            target,
            body: "same".to_owned(),
            attachment: Attachment::None,
        };
        let (reply, seen) = exchange(&post, &focus());
        assert!(matches!(reply, Reply::Commit(_)));
        assert_eq!(request_line(&seen), "POST /api/v1/commits HTTP/1.1");
        assert_eq!(
            sent(&seen),
            serde_json::json!({ "body": "same", "parent_id": parent, "quoted_commit_id": quoted })
        );
    }
}

const ME: &str = r#"{"data":{"username":"ada","name":"Ada"},"unread_notifications_count":2}"#;

fn answered(dir: &Path, status: &str, body: &str, call: &Call) -> Result<Reply, Error> {
    let (base, server) = serve(status, body);
    let reply = Client::new(base, DeviceKey::generate().unwrap())
        .with_cache(Cache::at(dir.to_owned()))
        .call(call);
    server.join().unwrap();
    reply
}

#[test]
fn the_account_and_first_page_are_kept_for_the_next_launch() {
    let dir = scratch("cache-kept");
    answered(&dir, "200 OK", ME, &Call::Me).unwrap();
    answered(
        &dir,
        "200 OK",
        &fixture("log-latest").to_string(),
        &Call::Page(Log::Latest),
    )
    .unwrap();

    let start = Cache::at(dir).load().unwrap();
    assert_eq!(start.account.me.username, "ada");
    assert_eq!(start.account.unread, 2);
    let logs: Vec<Log> = start.pages.iter().map(|page| page.log).collect();
    assert_eq!(logs, [Log::Latest]);
}

#[test]
fn an_older_page_is_not_kept() {
    let dir = scratch("cache-older");
    answered(&dir, "200 OK", ME, &Call::Me).unwrap();
    let older = Call::Older {
        log: Log::Latest,
        cursor: "c1".to_owned(),
    };
    answered(&dir, "200 OK", &fixture("log-latest").to_string(), &older).unwrap();

    assert!(Cache::at(dir).load().unwrap().pages.is_empty());
}

#[test]
fn a_device_the_server_forgets_forgets_its_timeline() {
    let dir = scratch("cache-forgotten");
    answered(&dir, "200 OK", ME, &Call::Me).unwrap();
    answered(
        &dir,
        "200 OK",
        &fixture("log-latest").to_string(),
        &Call::Page(Log::Latest),
    )
    .unwrap();
    let refusal = r#"{"message":"This key isn't approved.","code":"device_not_approved"}"#;
    answered(
        &dir,
        "401 Unauthorized",
        refusal,
        &Call::Page(Log::Trending),
    )
    .unwrap_err();

    assert!(Cache::at(dir.clone()).load().is_none());
    assert!(!dir.exists());
}

#[test]
fn what_is_kept_is_readable_only_by_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("cache-private");
    answered(&dir, "200 OK", ME, &Call::Me).unwrap();

    let mode = fs::metadata(dir.join("me.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}
