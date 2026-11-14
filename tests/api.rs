mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{openssh_accepts, scratch};
use ttyl::api::{Call, Client, Error, Paging, Reply};
use ttyl::attach::{self, Attachment, Image, Poll};
use ttyl::cache::Cache;
use ttyl::device::DeviceKey;
use ttyl::draft::Target;
use ttyl::model::{
    Chat, Circle, ClientId, CommitId, Extra, Filter, Folder, HeadlineId, Hush, Log, MessageId,
    MessagePage, Notice, NotificationId, OrgId, Party, ReactionsVersion, Reason, Refile,
    ScheduledId, Translation, Untranslated, Who,
};

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
        r#"{"data":{"id":"0a000000-0000-4000-8000-00000000000a","handle":"ada","name":"Ada","bio":null},"unread_notifications_count":2}"#,
    );
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let account = client.redeem("the-code", "the-verifier").unwrap();
    let seen = server.join().unwrap();

    assert_eq!(account.me.handle, "ada");
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
    let (base, server) = serve(
        "200 OK",
        r#"{"data":{"id":"0b000000-0000-4000-8000-00000000000b","handle":"grace","name":null}}"#,
    );
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let account = client.me().unwrap();
    let seen = server.join().unwrap();

    assert_eq!(account.me.id.0, "0b000000-0000-4000-8000-00000000000b");
    assert_eq!(account.me.handle, "grace");
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
    let (base, _server) = serve(
        "200 OK",
        r#"{"data":{"id":"0a000000-0000-4000-8000-00000000000a","name":"Ada"}}"#,
    );
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .me()
        .unwrap_err();

    assert!(
        matches!(&error, Error::WrongShape(detail) if detail.contains("handle")),
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
        author: None,
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
        author: None,
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
        author: None,
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
        author: None,
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
        author: None,
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
            who: Who::Person("ada".to_owned()),
            on,
        };
        let (reply, seen) = exchange(&follow, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/users/ada/follow HTTP/1.1")
        );

        let follow = Call::Follow {
            who: Who::Organization("acme".to_owned()),
            on,
        };
        let (reply, seen) = exchange(&follow, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/organizations/acme/follow HTTP/1.1")
        );
    }
}

#[test]
fn blocking_posts_to_block_and_deletes_to_unblock() {
    for (on, method) in [(true, "POST"), (false, "DELETE")] {
        let block = Call::Block {
            handle: "grace".to_owned(),
            on,
        };
        let answer = serde_json::json!({ "data": person(), "is_blocking": on });
        let (reply, seen) = exchange(&block, &answer);
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/users/grace/block HTTP/1.1")
        );
    }
}

#[test]
fn pinning_puts_and_unpinning_deletes_and_both_answer_with_the_commit() {
    let id = CommitId("7a000000-0000-4000-8000-00000000000a".to_owned());
    for (on, method) in [(true, "PUT"), (false, "DELETE")] {
        let mut answer = focus();
        answer["data"]["is_pinned"] = serde_json::json!(on);
        let (reply, seen) = exchange(&Call::Pin { id: id.clone(), on }, &answer);
        assert!(
            matches!(reply, Reply::Commit(focus) if focus.card.commit.id == id && focus.card.commit.is_pinned == on)
        );
        assert_eq!(
            request_line(&seen),
            format!("{method} /api/v1/commits/{}/pin HTTP/1.1", id.0)
        );
    }
}

#[test]
fn the_people_you_block_are_read_a_page_at_a_time() {
    let page = serde_json::json!({
        "data": [person()],
        "meta": { "next_cursor": "eyJpZCI6MX0+/=" },
    });
    let (base, server) = serve("200 OK", &page.to_string());
    let client = Client::new(base, DeviceKey::generate().unwrap());
    let blocked = client.blocked(None).unwrap();
    assert_eq!(blocked.people[0].user.handle, "ada");
    assert_eq!(blocked.next_cursor.as_deref(), Some("eyJpZCI6MX0+/="));
    assert_eq!(
        request_line(&server.join().unwrap()),
        "GET /api/v1/me/blocked HTTP/1.1"
    );

    let (base, server) = serve("200 OK", &page.to_string());
    let client = Client::new(base, DeviceKey::generate().unwrap());
    client.blocked(Some("eyJpZCI6MX0+/=")).unwrap();
    assert_eq!(
        request_line(&server.join().unwrap()),
        "GET /api/v1/me/blocked?cursor=eyJpZCI6MX0%2B%2F%3D HTTP/1.1"
    );
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
        matches!(reply, Reply::User(profile) if profile.person.user.handle == "ada" && profile.is_following)
    );
    assert_eq!(request_line(&seen), "GET /api/v1/users/ada HTTP/1.1");

    let rows = serde_json::json!({
        "data": [fixture("log-latest")["data"][0]],
        "meta": { "next_cursor": "next" },
    });
    let commits = |replies, cursor: Option<&str>| Call::Commits {
        handle: "ada".to_owned(),
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
            handle: "ada".to_owned(),
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

    let (reply, seen) = exchange(&Call::Restore(id.clone()), &focus());
    assert!(matches!(reply, Reply::Commit(focus) if focus.card.commit.id == id));
    assert_eq!(
        request_line(&seen),
        format!("POST {route}/restoration HTTP/1.1")
    );

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
            author: None,
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

const ME: &str = r#"{"data":{"id":"0a000000-0000-4000-8000-00000000000a","handle":"ada","name":"Ada"},"unread_notifications_count":2}"#;

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
    assert_eq!(start.account.me.handle, "ada");
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

fn kept(dir: &Path) {
    answered(dir, "200 OK", ME, &Call::Me).unwrap();
    answered(
        dir,
        "200 OK",
        &fixture("log-latest").to_string(),
        &Call::Page(Log::Latest),
    )
    .unwrap();
}

#[test]
fn revoking_this_device_forgets_the_timeline_the_next_launch_would_draw() {
    let dir = scratch("cache-revoked");
    kept(&dir);
    let id = ttyl::model::DeviceId("0f0e".to_owned());
    answered(&dir, "204 No Content", "", &Call::Revoke { id, this: true }).unwrap();

    assert!(Cache::at(dir).load().is_none(), "nothing left to draw");
}

#[test]
fn revoking_another_device_keeps_this_ones_timeline() {
    let dir = scratch("cache-revoked-other");
    kept(&dir);
    let id = ttyl::model::DeviceId("0f0e".to_owned());
    answered(
        &dir,
        "204 No Content",
        "",
        &Call::Revoke { id, this: false },
    )
    .unwrap();

    assert_eq!(Cache::at(dir).load().unwrap().pages.len(), 1);
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

#[test]
fn search_reads_commits_and_people_from_their_routes() {
    let mut rows = fixture("log-latest")["data"].clone();
    for row in rows.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("reply_preview");
    }
    let commits = serde_json::json!({ "data": rows });
    let (reply, seen) = exchange(&Call::SearchCommits("offset & cursor".to_owned()), &commits);
    assert!(
        matches!(reply, Reply::Rows(page) if !page.rows.is_empty() && page.next_cursor.is_none())
    );
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/search/commits?q=offset%20%26%20cursor HTTP/1.1"
    );

    let mut grace = person();
    grace["is_followed"] = serde_json::json!(true);
    let profiles =
        serde_json::json!({ "users": [grace], "organizations": [{ "slug": "laravel" }] });
    let (reply, seen) = exchange(&Call::SearchPeople("gra".to_owned()), &profiles);
    assert!(
        matches!(&reply, Reply::People(page) if page.people.len() == 1 && page.people[0].is_followed == Some(true)),
        "{reply:?}"
    );
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/search/profiles?q=gra HTTP/1.1"
    );
}

#[test]
fn settings_are_read_from_their_route() {
    let settings = serde_json::json!({ "data": {
        "timezone": "Europe/Paris",
        "show_local_time": true,
        "translation_language": "fr",
        "automatic_translation": false,
        "translation_languages": [{ "code": "fr", "name": "French", "native_name": "français" }],
        "is_admin": false,
        "push": {},
    }});
    let (reply, seen) = exchange(&Call::Settings, &settings);
    let Reply::Settings(settings) = reply else {
        panic!("{reply:?}");
    };
    let zone = settings.timezone.expect("a timezone");
    assert_eq!(
        (zone.name.as_str(), zone.local_time),
        ("Europe/Paris", true)
    );
    assert_eq!(settings.translation_language.as_deref(), Some("fr"));
    assert!(!settings.automatic_translation);
    let languages: Vec<_> = (settings.languages.iter())
        .map(|language| (language.code.as_str(), language.native_name.as_str()))
        .collect();
    assert_eq!(languages, [("fr", "français")]);
    assert_eq!(request_line(&seen), "GET /api/v1/me/settings HTTP/1.1");
}

fn me_answer() -> serde_json::Value {
    serde_json::json!({
        "data": { "id": "0a000000-0000-4000-8000-00000000000a", "handle": "ada", "name": "Ada", "pasteis_mode": true, "preferred_lane": "#ff8800",
            "automatic_lane": 2, "remotes": { "from": "everyone", "pull_receipts": false, "everyone": true } },
        "unread_notifications_count": 1,
    })
}

#[test]
fn each_setting_is_put_on_its_route_and_answered_as_me() {
    use ttyl::config::{Audience, Change};
    use ttyl::model::{Lane, Zone};
    let cases = [
        (
            Change::Pasteis(false),
            "/api/v1/me/pasteis-mode",
            serde_json::json!({ "enabled": false }),
        ),
        (
            Change::Color(None),
            "/api/v1/me/lane",
            serde_json::json!({ "lane": null }),
        ),
        (
            Change::Color(Some(Lane::Palette(4))),
            "/api/v1/me/lane",
            serde_json::json!({ "lane": 4 }),
        ),
        (
            Change::Color(Some(Lane::Custom {
                r: 255,
                g: 136,
                b: 0,
            })),
            "/api/v1/me/lane",
            serde_json::json!({ "lane": null, "color": "#ff8800" }),
        ),
        (
            Change::Timezone(Some(Zone {
                name: "Asia/Tokyo".to_owned(),
                local_time: true,
            })),
            "/api/v1/me/timezone",
            serde_json::json!({ "timezone": "Asia/Tokyo", "show_local_time": true }),
        ),
        (
            Change::Timezone(None),
            "/api/v1/me/timezone",
            serde_json::json!({ "timezone": null, "show_local_time": false }),
        ),
        (
            Change::Language(Some("de".to_owned())),
            "/api/v1/me/translation-language",
            serde_json::json!({ "translation_language": "de" }),
        ),
        (
            Change::AutoTranslate(false),
            "/api/v1/me/automatic-translation",
            serde_json::json!({ "enabled": false }),
        ),
        (
            Change::Remotes(true),
            "/api/v1/me/remotes",
            serde_json::json!({ "enabled": true }),
        ),
        (
            Change::Prefs {
                from: Audience::Everyone,
                pull_receipts: false,
            },
            "/api/v1/me/remote-preferences",
            serde_json::json!({ "from": "everyone", "pull_receipts": false }),
        ),
    ];
    for (change, path, body) in cases {
        let (reply, seen) = exchange(&Call::Configure(change), &me_answer());
        assert_eq!(request_line(&seen), format!("PUT {path} HTTP/1.1"));
        assert_eq!(sent(&seen), body);
        let Reply::Account(account) = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(
            account.me.preferred_lane,
            Some(Lane::Custom {
                r: 255,
                g: 136,
                b: 0
            })
        );
        assert_eq!(account.me.automatic_lane, 2);
        assert!(account.me.remotes.is_some_and(|remotes| remotes.everyone));
    }
}

#[test]
fn muted_and_blocked_people_are_read_from_their_routes() {
    let page = serde_json::json!({ "data": [], "meta": { "next_cursor": null } });
    let (_, seen) = exchange(
        &Call::Muted {
            cursor: Some("c2".to_owned()),
        },
        &page,
    );
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/me/muted?cursor=c2 HTTP/1.1"
    );
    let (_, seen) = exchange(&Call::Blocked { cursor: None }, &page);
    assert_eq!(request_line(&seen), "GET /api/v1/me/blocked HTTP/1.1");
}

#[test]
fn devices_are_listed_and_revoked_by_id() {
    let devices = serde_json::json!({ "data": [{
        "id": "0f0e", "name": "laptop", "app_name": null, "fingerprint": "SHA256:x",
        "approved_at": "2026-10-20T10:00:00Z", "last_used_at": null,
    }]});
    let (reply, seen) = exchange(&Call::Devices, &devices);
    assert_eq!(request_line(&seen), "GET /api/v1/me/devices HTTP/1.1");
    assert!(
        matches!(&reply, Reply::Devices(list) if list.len() == 1 && list[0].last_used_at.is_none()),
        "{reply:?}"
    );
    let id = ttyl::model::DeviceId("0f0e".to_owned());
    let revoke = Call::Revoke { id, this: false };
    let (reply, seen) = exchange(&revoke, &serde_json::json!({}));
    assert_eq!(
        request_line(&seen),
        "DELETE /api/v1/me/devices/0f0e HTTP/1.1"
    );
    assert!(matches!(reply, Reply::Done), "{reply:?}");
}

fn translate() -> Call {
    Call::Translate {
        id: CommitId("7a000000-0000-4000-8000-00000000000a".to_owned()),
        language: "en".to_owned(),
    }
}

#[test]
fn a_translation_is_asked_for_in_a_language_and_comes_back_as_segments() {
    let translation = serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "fr",
        "source_name": "French",
        "available": true,
        "translated": {
            "body_segments": [
                { "type": "text", "text": "run " },
                { "type": "code", "code": "cargo test" },
                { "type": "text", "text": " first" },
            ],
            "preview_segments": null,
        },
    }});
    let (reply, seen) = exchange(&translate(), &translation);
    let Reply::Translation(translation) = reply else {
        panic!("{reply:?}");
    };
    let Translation::Done(translated) = *translation else {
        panic!("{translation:?}");
    };
    assert_eq!(translated.from, "French");
    assert_eq!(translated.body, "run `cargo test` first");
    assert_eq!(translated.body_segments.len(), 3);
    assert_eq!(
        request_line(&seen),
        "POST /api/v1/commits/7a000000-0000-4000-8000-00000000000a/translation HTTP/1.1"
    );
    assert_eq!(sent(&seen), serde_json::json!({ "language": "en" }));
}

#[test]
fn a_commit_already_in_the_language_comes_back_untranslated() {
    let translation = serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "en",
        "source_name": "English",
        "available": false,
        "translated": null,
    }});
    let (reply, _) = exchange(&translate(), &translation);
    assert!(
        matches!(&reply, Reply::Translation(translation) if matches!(&**translation, Translation::Skipped(Untranslated::Already(name)) if name == "English")),
        "{reply:?}"
    );

    let emoji = serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "und",
        "source_name": "Unknown language",
        "available": false,
        "translated": null,
    }});
    let (reply, _) = exchange(&translate(), &emoji);
    assert!(
        matches!(&reply, Reply::Translation(translation) if matches!(&**translation, Translation::Skipped(Untranslated::NoLanguage))),
        "{reply:?}"
    );
}

#[test]
fn a_failed_translation_says_what_dev_null_said() {
    let (base, server) = serve(
        "503 Service Unavailable",
        r#"{"message":"Could not translate this commit. Try again."}"#,
    );
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .call(&translate())
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error.to_string(),
        "Could not translate this commit. Try again."
    );
}

#[test]
fn conversations_are_read_from_remotes_and_paged_by_conversation() {
    let call = Call::Conversations {
        folder: Folder::Open,
        query: String::new(),
        before: None,
    };
    let (reply, seen) = exchange(&call, &fixture("remotes"));
    assert_eq!(request_line(&seen), "GET /api/v1/remotes HTTP/1.1");
    let Reply::Conversations(page) = reply else {
        panic!("{reply:?}")
    };
    let titles: Vec<String> = (page.conversations.iter())
        .map(|conversation| conversation.party.title())
        .collect();
    assert_eq!(
        titles,
        ["@margaret", "@linus", "@grace", "#unix-room", "@ken"],
        "orgs come in dev-null.sh's order among the people"
    );
    let Party::Org(org) = &page.conversations[3].party else {
        panic!("{:?}", page.conversations[3])
    };
    assert_eq!(org.id, OrgId(UNIX_ROOM.to_owned()));
    assert_eq!(org.members.len(), 4);
    let last = page.conversations[3].last.as_ref().unwrap();
    assert_eq!(
        last.author.as_ref().map(|author| author.handle.as_str()),
        Some("grace")
    );
    assert_eq!(page.next, None);
    assert_eq!(page.requests, 0);
    assert!(page.conversations[0].filed.pinned);

    let call = Call::Conversations {
        folder: Folder::Open,
        query: String::new(),
        before: Some("c1000000-0000-4000-8000-00000000000e".to_owned()),
    };
    let (_, seen) = exchange(&call, &fixture("remotes"));
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/remotes?before=c1000000-0000-4000-8000-00000000000e HTTP/1.1"
    );
}

#[test]
fn a_folder_and_a_find_go_to_the_server_as_its_tab_and_q() {
    for (folder, query, path) in [
        (Folder::Requests, "", "/api/v1/remotes?tab=fetched"),
        (Folder::Stashed, "gra", "/api/v1/remotes?tab=stashed&q=gra"),
        (
            Folder::Open,
            "@grace hopper",
            "/api/v1/remotes?q=%40grace%20hopper",
        ),
    ] {
        let call = Call::Conversations {
            folder,
            query: query.to_owned(),
            before: None,
        };
        let (_, seen) = exchange(&call, &fixture("remotes"));
        assert_eq!(request_line(&seen), format!("GET {path} HTTP/1.1"));
    }
}

#[test]
fn pin_stash_and_mute_go_to_the_conversation_they_file() {
    let grace = Chat::Direct("grace".to_owned());
    let room = Chat::Org(OrgId(UNIX_ROOM.to_owned()));
    let room_path = format!("/api/v1/remotes/orgs/{UNIX_ROOM}");
    for (chat, change, method, path, body) in [
        (
            &grace,
            Refile::Pin(true),
            "PUT",
            "/api/v1/remotes/@grace/pin".to_owned(),
            r#"{"pinned":true}"#,
        ),
        (
            &room,
            Refile::Pin(false),
            "PUT",
            format!("{room_path}/pin"),
            r#"{"pinned":false}"#,
        ),
        (
            &grace,
            Refile::Stash(true),
            "POST",
            "/api/v1/remotes/@grace/archive".to_owned(),
            "",
        ),
        (
            &room,
            Refile::Stash(false),
            "DELETE",
            format!("{room_path}/archive"),
            "",
        ),
        (
            &grace,
            Refile::Mute(Some(Hush::Tomorrow)),
            "PUT",
            "/api/v1/remotes/@grace/mute".to_owned(),
            r#"{"duration":"tomorrow"}"#,
        ),
        (
            &room,
            Refile::Mute(None),
            "PUT",
            format!("{room_path}/mute"),
            r#"{"duration":"off"}"#,
        ),
    ] {
        let call = Call::File {
            chat: chat.clone(),
            change,
        };
        let (reply, seen) = exchange(&call, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done), "{change:?}: {reply:?}");
        assert_eq!(request_line(&seen), format!("{method} {path} HTTP/1.1"));
        assert_eq!(seen.body, body, "{change:?}");
    }
}

#[test]
fn a_request_is_merged_or_dropped_by_posting_to_its_verb() {
    for (merge, verb) in [(true, "merge"), (false, "drop")] {
        let call = Call::Answer {
            handle: "grace".to_owned(),
            merge,
        };
        let (reply, seen) = exchange(&call, &serde_json::json!({}));
        assert!(matches!(reply, Reply::Done));
        assert_eq!(
            request_line(&seen),
            format!("POST /api/v1/remotes/@grace/{verb} HTTP/1.1")
        );
    }
}

#[test]
fn people_to_start_a_conversation_with_come_with_why_they_cannot_be() {
    let answer = serde_json::json!({ "candidates": [
        { "user": person(), "reason": null },
        { "user": person(), "reason": "doesn't follow you" },
    ] });
    let (reply, seen) = exchange(&Call::Candidates("gr ace".to_owned()), &answer);
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/remotes/candidates?q=gr%20ace HTTP/1.1"
    );
    let Reply::Candidates(people) = reply else {
        panic!("{reply:?}")
    };
    let reasons: Vec<Option<&str>> = people.iter().map(|p| p.reason.as_deref()).collect();
    assert_eq!(reasons, [None, Some("doesn't follow you")]);
}

#[test]
fn a_conversation_is_read_newest_page_first_then_before_or_after_a_message() {
    let thread = fixture("remote-thread");
    for (paging, path) in [
        (Paging::Latest, "/api/v1/remotes/@grace/messages"),
        (
            Paging::Before(MessageId(101)),
            "/api/v1/remotes/@grace/messages?before=101",
        ),
        (
            Paging::Since {
                after: MessageId(105),
                reactions: ReactionsVersion(7),
            },
            "/api/v1/remotes/@grace/messages?after=105&reactions_after=7",
        ),
    ] {
        let call = Call::Messages {
            chat: Chat::Direct("grace".to_owned()),
            paging,
        };
        let (reply, seen) = exchange(&call, &thread);
        assert_eq!(request_line(&seen), format!("GET {path} HTTP/1.1"));
        let Reply::Messages(page) = reply else {
            panic!("{reply:?}")
        };
        assert_eq!(page.messages.len(), 4);
        assert_eq!(page.read_through, MessageId(102));
        assert_eq!(page.partner_read_through, Some(MessageId(102)));
    }
}

#[test]
fn a_poll_reads_the_reactions_changed_since_and_how_far_the_partner_read() {
    let mut thread = fixture("remote-thread");
    thread["data"]["reactions_version"] = serde_json::json!(9);
    thread["data"]["reaction_changes"] = serde_json::json!([{ "id": 101, "reactions": [
        { "user_id": "0a000000-0000-4000-8000-00000000000a", "emoji": "👍" },
    ] }]);
    let page = MessagePage::parse(&thread.to_string()).unwrap();
    assert_eq!(page.reactions_version, ReactionsVersion(9));
    let [(id, reactions)] = page.reaction_changes.as_slice() else {
        panic!("{:?}", page.reaction_changes)
    };
    assert_eq!(*id, MessageId(101));
    assert_eq!(reactions[0].emoji, "👍");

    thread["data"]["partner_read_through_id"] = serde_json::json!(0);
    let page = MessagePage::parse(&thread.to_string()).unwrap();
    assert_eq!(
        page.partner_read_through, None,
        "0 means dev-null.sh doesn't share how far they read"
    );
}

#[test]
fn a_reply_names_the_message_it_answers_and_reads_back_its_start() {
    let client_id = "6f1d1c52-6a0e-4c43-9a41-2d4f0c3b8e12";
    let call = Call::SendMessage {
        chat: Chat::Direct("grace".to_owned()),
        body: "after the standup".to_owned(),
        client_id: ClientId(client_id.to_owned()),
        reply_to: Some(MessageId(101)),
    };
    let message = serde_json::json!({ "data": {
        "id": 106, "user_id": "0a000000-0000-4000-8000-00000000000a", "client_id": client_id,
        "body": "after the standup", "images": [], "voice": null, "event": null,
        "reply_to_id": 101,
        "reply_to": { "id": 101, "user_id": "0b000000-0000-4000-8000-00000000000b",
            "body": "\nare you around tomorrow?\nthe wrap code", "images_count": 0, "has_voice": false },
        "reactions": [],
        "created_at": "2026-10-21T09:58:00Z",
    }});
    let (reply, seen) = exchange(&call, &message);
    assert_eq!(
        request_line(&seen),
        "POST /api/v1/remotes/@grace/messages HTTP/1.1"
    );
    assert_eq!(
        sent(&seen),
        serde_json::json!({ "body": "after the standup", "client_id": client_id, "reply_to_id": 101 })
    );
    let Reply::Message(message) = reply else {
        panic!("{reply:?}")
    };
    let reply_to = message.reply_to.unwrap();
    assert_eq!(reply_to.id, MessageId(101));
    let quote = reply_to.quote.unwrap();
    assert_eq!(quote.user_id.0, "0b000000-0000-4000-8000-00000000000b");
    assert_eq!(quote.text, "are you around tomorrow?");
}

#[test]
fn a_reaction_is_posted_or_deleted_with_its_emoji_and_answers_with_all_of_them() {
    let answer = serde_json::json!({ "reactions": [
        { "user_id": "0b000000-0000-4000-8000-00000000000b", "emoji": "✅" },
        { "user_id": "0a000000-0000-4000-8000-00000000000a", "emoji": "✅" },
    ] });
    for (on, method) in [(true, "POST"), (false, "DELETE")] {
        let (base, server) = serve("200 OK", &answer.to_string());
        let client = Client::new(base, DeviceKey::generate().unwrap());
        let call = Call::React {
            chat: Chat::Direct("grace".to_owned()),
            message: MessageId(105),
            emoji: "✅".to_owned(),
            on,
        };
        let reply = client.call(&call).unwrap();
        let seen = server.join().unwrap();
        let path = "/api/v1/remotes/messages/105/reactions";
        assert_eq!(request_line(&seen), format!("{method} {path} HTTP/1.1"));
        assert_eq!(sent(&seen), serde_json::json!({ "emoji": "✅" }));
        assert!(
            signature_holds(
                client.key(),
                &seen,
                method,
                path,
                &format!("api-react-{method}")
            ),
            "the {method} is signed over its body"
        );
        let Reply::Reactions(reactions) = reply else {
            panic!("{reply:?}")
        };
        let emoji: Vec<&str> = reactions.iter().map(|each| each.emoji.as_str()).collect();
        assert_eq!(emoji, ["✅", "✅"]);
    }
}

#[test]
fn a_ping_posts_to_the_person_or_the_whole_org() {
    let ping = serde_json::json!({ "data": {
        "id": 107, "user_id": "0a000000-0000-4000-8000-00000000000a", "client_id": null,
        "body": null, "images": [], "voice": null, "event": "ping",
        "created_at": "2026-10-21T10:00:00Z",
    }});
    for (chat, path) in [
        (
            Chat::Direct("grace".to_owned()),
            "/api/v1/remotes/@grace/ping".to_owned(),
        ),
        (
            Chat::Org(OrgId(UNIX_ROOM.to_owned())),
            format!("/api/v1/remotes/orgs/{UNIX_ROOM}/ping"),
        ),
    ] {
        let (reply, seen) = exchange(&Call::Ping(chat), &ping);
        assert_eq!(request_line(&seen), format!("POST {path} HTTP/1.1"));
        assert_eq!(seen.body, "", "a ping carries nothing");
        assert!(matches!(reply, Reply::Message(message) if message.extra == Some(Extra::Ping)));
    }
}

#[test]
fn marking_read_puts_how_far_and_sending_posts_the_text_with_its_client_id() {
    let call = Call::MarkRead {
        chat: Chat::Direct("grace".to_owned()),
        through: MessageId(105),
    };
    let (base, server) = serve("204 No Content", "");
    let reply = Client::new(base, DeviceKey::generate().unwrap())
        .call(&call)
        .unwrap();
    let seen = server.join().unwrap();
    assert!(matches!(reply, Reply::Done));
    assert_eq!(
        request_line(&seen),
        "PUT /api/v1/remotes/@grace/read HTTP/1.1"
    );
    assert_eq!(sent(&seen), serde_json::json!({ "through": 105 }));

    let client_id = "6f1d1c52-6a0e-4c43-9a41-2d4f0c3b8e11";
    let call = Call::SendMessage {
        chat: Chat::Direct("grace".to_owned()),
        body: "merging it now".to_owned(),
        client_id: ClientId(client_id.to_owned()),
        reply_to: None,
    };
    let message = serde_json::json!({ "data": {
        "id": 106, "user_id": "0a000000-0000-4000-8000-00000000000a", "client_id": client_id,
        "body": "merging it now", "images": [], "voice": null, "event": null,
        "created_at": "2026-10-21T09:58:00Z",
    }});
    let (reply, seen) = exchange(&call, &message);
    assert_eq!(
        request_line(&seen),
        "POST /api/v1/remotes/@grace/messages HTTP/1.1"
    );
    assert_eq!(
        sent(&seen),
        serde_json::json!({ "body": "merging it now", "client_id": client_id })
    );
    assert!(matches!(reply, Reply::Message(message) if message.id == MessageId(106)));
}

const UNIX_ROOM: &str = "c1000000-0000-4000-8000-0000000000f0";

#[test]
fn an_org_is_read_marked_and_sent_to_by_its_id() {
    let thread = fixture("remote-org-thread");
    let base = format!("/api/v1/remotes/orgs/{UNIX_ROOM}");
    let org = Chat::Org(OrgId(UNIX_ROOM.to_owned()));
    for (paging, query) in [
        (Paging::Latest, ""),
        (Paging::Before(MessageId(200)), "?before=200"),
        (
            Paging::Since {
                after: MessageId(210),
                reactions: ReactionsVersion(3),
            },
            "?after=210&reactions_after=3",
        ),
    ] {
        let call = Call::Messages {
            chat: org.clone(),
            paging,
        };
        let (reply, seen) = exchange(&call, &thread);
        assert_eq!(
            request_line(&seen),
            format!("GET {base}/messages{query} HTTP/1.1")
        );
        let Reply::Messages(page) = reply else {
            panic!("{reply:?}")
        };
        assert_eq!(page.party.title(), "#unix-room");
        assert_eq!(page.read_through, MessageId(204));
        let authors: Vec<&str> = (page.messages.iter())
            .filter_map(|message| message.author.as_ref())
            .map(|author| author.handle.as_str())
            .collect();
        assert_eq!(
            authors,
            ["margaret", "margaret", "linus", "ada", "grace", "grace"]
        );
        assert_eq!(
            page.messages[1].notice,
            Some(Notice::Added("linus".to_owned()))
        );
        assert_eq!(
            page.messages[1].told().as_deref(),
            Some("margaret added linus")
        );
    }

    let (base_url, server) = serve("204 No Content", "");
    let call = Call::MarkRead {
        chat: org.clone(),
        through: MessageId(210),
    };
    Client::new(base_url, DeviceKey::generate().unwrap())
        .call(&call)
        .unwrap();
    let seen = server.join().unwrap();
    assert_eq!(request_line(&seen), format!("PUT {base}/read HTTP/1.1"));
    assert_eq!(sent(&seen), serde_json::json!({ "through": 210 }));

    let client_id = "6f1d1c52-6a0e-4c43-9a41-2d4f0c3b8e11";
    let call = Call::SendMessage {
        chat: org,
        body: "reviewing now".to_owned(),
        client_id: ClientId(client_id.to_owned()),
        reply_to: Some(MessageId(208)),
    };
    let message = serde_json::json!({ "data": {
        "id": 211, "user_id": "0a000000-0000-4000-8000-00000000000a", "client_id": client_id,
        "body": "reviewing now", "images": [], "voice": null, "event": null,
        "created_at": "2026-10-21T09:58:00Z",
    }});
    let (reply, seen) = exchange(&call, &message);
    assert_eq!(
        request_line(&seen),
        format!("POST {base}/messages HTTP/1.1")
    );
    assert_eq!(
        sent(&seen),
        serde_json::json!({ "body": "reviewing now", "client_id": client_id, "reply_to_id": 208 })
    );
    assert!(matches!(reply, Reply::Message(message) if message.id == MessageId(211)));
}

#[test]
fn related_reads_the_commits_like_one_from_its_route() {
    let mut rows = fixture("log-latest")["data"].clone();
    for row in rows.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("reply_preview");
    }
    rows.as_array_mut().unwrap().truncate(3);
    let id = CommitId("9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61".to_owned());
    let (reply, seen) = exchange(&Call::Related(id), &serde_json::json!({ "data": rows }));
    assert!(
        matches!(&reply, Reply::Rows(page) if page.rows.len() == 3 && page.next_cursor.is_none()),
        "{reply:?}"
    );
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/commits/9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61/related HTTP/1.1"
    );
}

#[test]
fn headlines_reads_the_hot_topics_from_their_route() {
    let hot = serde_json::json!({ "data": [
        { "id": "0199a000-0000-7000-8000-000000000001", "name": "Postgres 18 async IO", "summary": "Faster scans." },
        { "id": "0199a000-0000-7000-8000-000000000002", "name": "Rust 2027 edition", "summary": null },
    ] });
    let (reply, seen) = exchange(&Call::Headlines, &hot);
    let Reply::Headlines(headlines) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(headlines.len(), 2);
    assert_eq!(headlines[0].name, "Postgres 18 async IO");
    assert_eq!(headlines[0].summary.as_deref(), Some("Faster scans."));
    assert_eq!(headlines[1].summary, None, "a stale recap comes as null");
    assert_eq!(request_line(&seen), "GET /api/v1/headlines HTTP/1.1");
}

#[test]
fn a_headline_reads_its_commits_name_and_recap_from_its_route() {
    let mut rows = fixture("log-latest")["data"].clone();
    for row in rows.as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("reply_preview");
    }
    let page = serde_json::json!({ "data": rows, "name": "Postgres 18 async IO", "summary": null });
    let id = HeadlineId("0199a000-0000-7000-8000-000000000001".to_owned());
    let (reply, seen) = exchange(&Call::Headline(id), &page);
    let Reply::Headline(page) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(page.name, "Postgres 18 async IO");
    assert_eq!(page.summary, None);
    assert!(!page.rows.is_empty());
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/headlines/0199a000-0000-7000-8000-000000000001 HTTP/1.1"
    );
}

/// A scheduled commit as the API sends it, with the `status` it has reached.
fn scheduled(id: &str, body: Option<&str>, status: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "author": null,
        "body": body,
        "body_segments": [],
        "preview_segments": null,
        "poll_options": null,
        "image_count": 0,
        "scheduled_at": "2026-10-23T12:00:00+00:00",
        "submitted_at": null,
        "status": status,
    })
}

#[test]
fn scheduling_posts_the_body_and_its_time_with_the_offset_it_was_read_in() {
    let call = Call::Schedule {
        body: "shipped it".to_owned(),
        attachment: Attachment::None,
        at: chrono::DateTime::parse_from_rfc3339("2026-10-23T14:00:00+02:00").unwrap(),
        author: None,
    };
    let answer = serde_json::json!({ "data": scheduled("s1", Some("shipped it"), "scheduled") });
    let (reply, seen) = exchange(&call, &answer);
    assert_eq!(
        request_line(&seen),
        "POST /api/v1/scheduled-commits HTTP/1.1"
    );
    assert_eq!(
        sent(&seen),
        serde_json::json!({ "body": "shipped it", "scheduled_at": "2026-10-23T14:00:00+02:00" })
    );
    let Reply::Scheduled(scheduled) = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(scheduled.id, ScheduledId("s1".to_owned()));
    assert_eq!(scheduled.at.to_rfc3339(), "2026-10-23T12:00:00+00:00");
}

#[test]
fn the_queue_keeps_only_what_still_waits() {
    let page = serde_json::json!({
        "data": [
            scheduled("s1", Some("standup notes"), "scheduled"),
            scheduled("s2", None, "approved"),
            scheduled("s3", None, "deleted"),
        ],
        "links": {},
        "meta": {},
    });
    let (reply, seen) = exchange(&Call::Queue, &page);
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/scheduled-commits HTTP/1.1"
    );
    let Reply::Queue(items) = reply else {
        panic!("{reply:?}");
    };
    let ids: Vec<&str> = items.iter().map(|item| item.id.0.as_str()).collect();
    assert_eq!(ids, ["s1"]);
    assert_eq!(items[0].body, "standup notes");
}

#[test]
fn canceling_deletes_and_sending_now_posts_a_submission() {
    let id = ScheduledId("9b000000-0000-4000-8000-000000000001".to_owned());
    let (base, server) = serve("204 No Content", "");
    let reply = Client::new(base, DeviceKey::generate().unwrap())
        .call(&Call::Unschedule(id.clone()))
        .unwrap();
    assert!(matches!(reply, Reply::Done), "{reply:?}");
    assert_eq!(
        request_line(&server.join().unwrap()),
        "DELETE /api/v1/scheduled-commits/9b000000-0000-4000-8000-000000000001 HTTP/1.1"
    );

    for (status, pending) in [("approved", false), ("pending", true)] {
        let answer = serde_json::json!({ "data": scheduled(&id.0, None, status) });
        let (reply, seen) = exchange(&Call::SendNow(id.clone()), &answer);
        assert!(
            matches!(reply, Reply::Submitted { pending: was } if was == pending),
            "{status}: {reply:?}"
        );
        assert_eq!(
            request_line(&seen),
            "POST /api/v1/scheduled-commits/9b000000-0000-4000-8000-000000000001/submission HTTP/1.1"
        );
    }
}

#[test]
fn one_already_gone_is_a_refusal_with_the_server_reason() {
    let gone = "This commit was already submitted or canceled. Refresh to see its status.";
    let (base, server) = serve(
        "409 Conflict",
        &serde_json::json!({ "message": gone }).to_string(),
    );
    let error = Client::new(base, DeviceKey::generate().unwrap())
        .call(&Call::SendNow(ScheduledId("s1".to_owned())))
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(error.to_string(), gone);
    assert!(!error.may_have_landed());
}

fn organization() -> serde_json::Value {
    serde_json::json!({
        "handle": "acme",
        "name": "Acme",
        "avatar_url": null,
        "bio": null,
        "location": null,
        "website": null,
        "github_url": "https://github.com/acme",
        "html_url": "https://dev-null.sh/acme",
        "lane": null,
    })
}

#[test]
fn an_organizations_page_commits_and_mentions_have_their_own_routes() {
    let mut data = organization();
    data["followers_count"] = serde_json::json!(7);
    data["admins"] = serde_json::json!([]);
    let page = serde_json::json!({
        "data": data,
        "is_following": true,
        "can_claim": false,
        "settings_url": null,
    });
    let (reply, seen) = exchange(&Call::Organization("acme".to_owned()), &page);
    assert!(matches!(
        reply,
        Reply::Organization(page) if page.is_following && page.organization.followers_count == Some(7)
    ));
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/organizations/acme HTTP/1.1"
    );

    let rows = serde_json::json!({
        "data": [fixture("log-latest")["data"][0]],
        "meta": { "next_cursor": "next" },
    });
    for (mentions, cursor, path) in [
        (false, None, "/api/v1/organizations/acme/commits"),
        (
            true,
            Some("a+b"),
            "/api/v1/organizations/acme/mentions?cursor=a%2Bb",
        ),
    ] {
        let call = Call::OrganizationCommits {
            handle: "acme".to_owned(),
            mentions,
            cursor: cursor.map(str::to_owned),
        };
        let (reply, seen) = exchange(&call, &rows);
        assert!(
            matches!(&reply, Reply::Rows(page) if page.rows.len() == 1),
            "{call:?}"
        );
        assert_eq!(request_line(&seen), format!("GET {path} HTTP/1.1"));
    }

    let mut followed = organization();
    followed["is_followed"] = serde_json::json!(true);
    let list = serde_json::json!({ "data": [followed], "meta": { "next_cursor": "o2" } });
    let call = Call::FollowedOrganizations {
        handle: "ada".to_owned(),
        cursor: Some("o2".to_owned()),
    };
    let (reply, seen) = exchange(&call, &list);
    assert!(matches!(
        &reply,
        Reply::Organizations(list) if list.organizations[0].is_followed == Some(true)
            && list.next_cursor.as_deref() == Some("o2")
    ));
    assert_eq!(
        request_line(&seen),
        "GET /api/v1/users/ada/following/organizations?cursor=o2 HTTP/1.1"
    );
}

#[test]
fn a_commit_as_an_organization_names_it_and_one_as_you_names_nobody() {
    for (author, field) in [
        (Some("acme".to_owned()), Some(serde_json::json!("acme"))),
        (None, None),
    ] {
        let post = Call::Post {
            target: Target::New,
            body: "shipped".to_owned(),
            attachment: Attachment::None,
            author,
        };
        let (_, seen) = exchange(&post, &focus());
        let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(body.get("author").cloned(), field, "{body}");
    }
    let schedule = Call::Schedule {
        body: "shipped".to_owned(),
        attachment: Attachment::None,
        at: chrono::DateTime::parse_from_rfc3339("2026-10-23T14:00:00+02:00").unwrap(),
        author: Some("acme".to_owned()),
    };
    let answer = serde_json::json!({ "data": scheduled("s1", Some("shipped"), "scheduled") });
    let (_, seen) = exchange(&schedule, &answer);
    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(body["author"], "acme", "{body}");
}
