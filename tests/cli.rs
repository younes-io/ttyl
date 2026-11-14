mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};

use common::scratch;
use ttyl::api::Client;
use ttyl::cache::Cache;
use ttyl::device::DeviceKey;

fn ttyl(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ttyl"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn version_prints_the_package_version() {
    for flag in ["--version", "-V"] {
        let output = ttyl(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("ttyl {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn help_prints_usage_and_the_server_variable() {
    for flag in ["--help", "-h"] {
        let output = ttyl(&[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("Usage: ttyl [--version | --help]"),
            "{stdout}"
        );
        assert!(stdout.contains("DEVNULL_URL"), "{stdout}");
        assert!(!stdout.contains("--redirect"), "{stdout}");
    }
}

#[test]
fn unknown_arguments_are_a_usage_error() {
    for args in [
        &["--bogus"][..],
        &["--version", "--bogus"],
        &["--bogus", "x"],
        &["status", "--bogus"],
        &["commit", "--bogus"],
        &["headlines", "--bogus"],
        &["blocked", "--bogus"],
        &["msg", "--bogus"],
        &["msg", "@grace", "--bogus"],
    ] {
        let output = ttyl(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unknown argument '--bogus'"), "{stderr}");
        assert!(stderr.contains("ttyl --help"), "{stderr}");
    }
}

#[test]
fn running_outside_a_terminal_says_so_instead_of_panicking() {
    let output = ttyl(&[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, "ttyl: not a terminal; run ttyl in one\n");
}

/// Answers one request with a canned response and hands back the body the client sent.
fn serve(status: &str, body: &str) -> (String, JoinHandle<String>) {
    let (base, server) = serve_in_turn(&[(status, body)]);
    (
        base,
        thread::spawn(move || server.join().unwrap().remove(0)),
    )
}

/// Answers one request after another with each canned response, and hands back the bodies the
/// client sent.
fn serve_in_turn(responses: &[(&str, &str)]) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let responses: Vec<String> = (responses.iter())
        .map(|(status, body)| {
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        })
        .collect();
    let server = thread::spawn(move || {
        (responses.iter())
            .map(|response| {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut sent = vec![0; length];
                reader.read_exact(&mut sent).unwrap();
                stream.write_all(response.as_bytes()).unwrap();
                String::from_utf8(sent).unwrap()
            })
            .collect()
    });
    (base, server)
}

/// Runs ttyl against `base` with its key in a scratch config, feeding `stdin` when given.
fn against(name: &str, base: &str, args: &[&str], stdin: Option<&str>) -> Output {
    at_home(&scratch(name), base, args, stdin)
}

/// Runs ttyl against `base` with its key and cache under `home`, feeding `stdin` when given.
fn at_home(home: &Path, base: &str, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ttyl"))
        .args(args)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("DEVNULL_URL", base)
        .env("TZ", "UTC")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        let written = child.stdin.take().unwrap().write_all(text.as_bytes());
        // ttyl may refuse its arguments and exit before reading stdin, closing the pipe first.
        if let Err(error) = written {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
        }
    }
    child.wait_with_output().unwrap()
}

fn posted() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/commit-quote.json"
    ))
    .unwrap()
}

#[test]
fn status_prints_the_unread_count_alone() {
    let (base, _server) = serve(
        "200 OK",
        r#"{"data":{"handle":"ada","name":"Ada"},"unread_notifications_count":3}"#,
    );
    let output = against("cli-status", &base, &["status"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "3\n");
}

#[test]
fn status_when_not_signed_in_says_how_to_sign_in() {
    let (base, _server) = serve(
        "403 Forbidden",
        r#"{"message":"This device is not approved.","code":"device_not_approved"}"#,
    );
    let output = against("cli-status-out", &base, &["status"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: not signed in; run ttyl to sign in\n"
    );
}

#[test]
fn commit_sends_the_message_and_prints_it_the_way_git_does() {
    let (base, server) = serve("201 Created", &posted());
    let output = against(
        "cli-commit",
        &base,
        &["commit", "-m", "shipped the parser"],
        None,
    );
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(sent["body"], "shipped the parser");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[log 000000a] this is the clearest explanation of keyset paging I have read.\n\
         https://dev-null.sh/@ken/commits/7a000000-0000-4000-8000-00000000000a\n"
    );
}

#[test]
fn commit_reads_its_text_from_stdin_without_m() {
    let (base, server) = serve("201 Created", &posted());
    let output = against(
        "cli-commit-stdin",
        &base,
        &["commit"],
        Some("piped in\n\nwith a body\n"),
    );
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(sent["body"], "piped in\n\nwith a body\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn an_empty_commit_is_refused_before_anything_is_sent() {
    let output = against(
        "cli-commit-empty",
        "http://127.0.0.1:9",
        &["commit"],
        Some("  \n"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: nothing to commit. the body is empty.\n"
    );
}

#[test]
fn a_first_commit_points_to_the_rules_on_the_web() {
    let (base, _server) = serve(
        "403 Forbidden",
        r#"{"message":"Read the commit rules first.","code":"commit_rules_required"}"#,
    );
    let output = against("cli-commit-rules", &base, &["commit", "-m", "hi"], None);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: Your first commit has to come from the web, so you can read the commit rules first.\n"
    );
}

const SCHEDULED: &str = r#"{"data":{"id":"s1","author":null,"body":"standup notes","body_segments":[],"preview_segments":null,"poll_options":null,"image_count":0,"scheduled_at":"2026-10-23T12:00:00+00:00","submitted_at":null,"status":"scheduled"}}"#;

#[test]
fn commit_at_schedules_it_and_prints_when_it_goes_out() {
    for (name, args, stdin) in [
        (
            "cli-at",
            &["commit", "--at", "in 2h", "-m", "standup notes"][..],
            None,
        ),
        (
            "cli-at-stdin",
            &["commit", "--at", "in 2h"],
            Some("standup notes"),
        ),
    ] {
        let (base, server) = serve("201 Created", SCHEDULED);
        let before = chrono::Utc::now();
        let output = against(name, &base, args, stdin);
        let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(sent["body"], "standup notes", "{name}");
        let at = sent["scheduled_at"].as_str().unwrap();
        assert!(at.ends_with("+00:00"), "{name}: {at}");
        let ahead = chrono::DateTime::parse_from_rfc3339(at).unwrap().to_utc() - before;
        assert!(
            (chrono::TimeDelta::minutes(119)..=chrono::TimeDelta::minutes(121)).contains(&ahead),
            "{name}: {at} is two hours on"
        );
        assert_eq!(output.status.code(), Some(0), "{name}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "scheduled for Fri 23 Oct 12:00\n",
            "{name}"
        );
    }
}

const MEMBER: &str = r#"{"data":{"handle":"ada","name":"Ada","organizations":[{"type":"organization","id":"o1","role":"member","handle":"Acme"}]},"unread_notifications_count":0}"#;

#[test]
fn commit_as_posts_as_an_organization_spelled_as_the_server_spells_it() {
    for (name, args, stdin) in [
        (
            "cli-as",
            &["commit", "--as", "@acme", "-m", "shipped"][..],
            None,
        ),
        ("cli-as-stdin", &["commit", "--as", "acme"], Some("shipped")),
    ] {
        let (base, server) = serve_in_turn(&[("200 OK", MEMBER), ("201 Created", &posted())]);
        let output = against(name, &base, args, stdin);
        let sent = server.join().unwrap();
        let post: serde_json::Value = serde_json::from_str(&sent[1]).unwrap();
        assert_eq!(
            (&post["author"], &post["body"]),
            (&"Acme".into(), &"shipped".into())
        );
        assert_eq!(output.status.code(), Some(0), "{name}");
    }
}

#[test]
fn commit_as_yourself_names_no_author() {
    let (base, server) = serve_in_turn(&[("200 OK", MEMBER), ("201 Created", &posted())]);
    let output = against(
        "cli-as-me",
        &base,
        &["commit", "--as", "@ada", "-m", "hi"],
        None,
    );
    let post: serde_json::Value = serde_json::from_str(&server.join().unwrap()[1]).unwrap();
    assert!(post.get("author").is_none(), "{post}");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn commit_as_an_organization_you_are_not_in_is_fatal_before_anything_is_read_or_sent() {
    let (base, server) = serve("200 OK", MEMBER);
    let output = against(
        "cli-as-refused",
        &base,
        &["commit", "--as", "@zeta"],
        Some("shipped"),
    );
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: fatal: you are not a member of @zeta\n"
    );
}

#[test]
fn a_time_that_wont_do_is_fatal_before_anything_is_read_or_sent() {
    for (when, why) in [
        (
            "soon",
            "can't read 'soon' as a time. try 14:30, tomorrow 9:00, fri 14:00, in 2h, or 2026-10-24T09:00:00+02:00",
        ),
        ("2020-01-01T00:00:00Z", "that time has passed"),
        ("in 40d", "dev-null.sh schedules up to 30 days ahead"),
        ("in 99999999d", "dev-null.sh schedules up to 30 days ahead"),
        (
            "in 100000000000000m100000000000000m",
            "dev-null.sh schedules up to 30 days ahead",
        ),
    ] {
        let output = against(
            "cli-at-refused",
            "http://127.0.0.1:9",
            &["commit", "--at", when],
            Some("standup notes"),
        );
        assert_eq!(output.status.code(), Some(1), "{when}");
        assert!(output.stdout.is_empty(), "{when}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("ttyl: fatal: {why}\n")
        );
    }
}

fn latest() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/log-latest.json"
    ))
    .unwrap()
}

#[test]
fn log_prints_commits_the_way_git_log_does() {
    let (base, _server) = serve("200 OK", &latest());
    let output = against("cli-log", &base, &["log", "latest"], None);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with(
            "commit 9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61\n\
             Author: Ada <@ada>\n\
             Date:   Wed Oct 21 09:42:00 2026 +0000\n\
             \n    switched the feed to `cursorPaginate(20)`. offsets fall over once the table grows.\n\
             \ncommit 9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a62 (in review)\n"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains('\x1b'), "no color into a pipe: {stdout:?}");
}

#[test]
fn log_oneline_prints_a_line_per_commit_for_grep_and_fzf() {
    let (base, _server) = serve("200 OK", &latest());
    let output = against(
        "cli-log-oneline",
        &base,
        &["log", "--oneline", "latest"],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 11, "{stdout}");
    assert_eq!(
        lines[0],
        "e4f5a61 @ada switched the feed to `cursorPaginate(20)`. offsets fall over once the table grows."
    );
    assert!(
        lines[1].starts_with("e4f5a62 (in review) @ada "),
        "{}",
        lines[1]
    );
}

#[test]
fn log_refuses_a_log_it_does_not_know() {
    let output = ttyl(&["log", "popular"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unknown argument 'popular'")
    );
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn msg_alone_prints_conversations_and_groups_with_their_unread_counts() {
    let (base, _server) = serve("200 OK", &fixture("remotes"));
    let output = against("cli-msg-list", &base, &["msg"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "@margaret\t0\tthe lock order is in the commit message now\n\
         @grace\t2\tdid you see the cursor patch?\n\
         #unix-room\t2\tpushed the fix for the cursor race, can someone review?\n\
         @linus\t0\t[image]\n\
         @ken\t0\t[voice note]\n"
    );
}

#[test]
fn msg_to_a_group_by_name_sends_to_the_org_it_lists() {
    let remotes = fixture("remotes");
    let back = sent_back("reviewing now");
    let (base, server) = serve_in_turn(&[("200 OK", &remotes), ("201 Created", &back)]);
    let output = against(
        "cli-msg-group",
        &base,
        &["msg", "#Unix-Room", "-m", "reviewing now"],
        None,
    );
    let sent = server.join().unwrap();
    let posted: serde_json::Value = serde_json::from_str(&sent[1]).unwrap();
    assert_eq!(posted["body"], "reviewing now");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[msg #unix-room] reviewing now\n"
    );
}

/// The remotes fixture as two pages, with the group on both, as when it moves while ttyl pages.
fn remotes_in_two_pages() -> (String, String) {
    let remotes: serde_json::Value = serde_json::from_str(&fixture("remotes")).unwrap();
    let rows = remotes["data"].as_array().unwrap();
    let page = |rows: &[serde_json::Value], has_more: bool| {
        serde_json::json!({ "data": rows, "has_more": has_more, "requests_count": 0 }).to_string()
    };
    (page(&rows[..4], true), page(&rows[3..], false))
}

#[test]
fn a_group_on_two_pages_lists_once_and_still_takes_messages() {
    let (first, second) = remotes_in_two_pages();
    let (base, _server) = serve_in_turn(&[("200 OK", &first), ("200 OK", &second)]);
    let output = against("cli-msg-list-twice", &base, &["msg"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches("#unix-room").count(), 1, "{stdout}");
    assert!(stdout.ends_with("@ken\t0\t[voice note]\n"), "{stdout}");

    let back = sent_back("reviewing now");
    let (base, server) = serve_in_turn(&[
        ("200 OK", &first),
        ("200 OK", &second),
        ("201 Created", &back),
    ]);
    let output = against(
        "cli-msg-group-twice",
        &base,
        &["msg", "#unix-room", "-m", "reviewing now"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let sent = server.join().unwrap();
    let posted: serde_json::Value = serde_json::from_str(&sent[2]).unwrap();
    assert_eq!(posted["body"], "reviewing now");
}

#[test]
fn msg_to_a_group_you_are_not_in_says_so_and_sends_nothing() {
    let (base, _server) = serve("200 OK", &fixture("remotes"));
    let output = against(
        "cli-msg-no-group",
        &base,
        &["msg", "#plan9", "-m", "hi"],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: you're in no group named #plan9; ttyl msg lists your conversations\n"
    );
}

fn sent_back(body: &str) -> String {
    serde_json::json!({ "data": {
        "id": 106, "user_id": "0a000000-0000-4000-8000-00000000000a",
        "client_id": "6f1d1c52-6a0e-4c43-9a41-2d4f0c3b8e11", "body": body,
        "images": [], "voice": null, "event": null, "created_at": "2026-10-21T09:58:00Z",
    }})
    .to_string()
}

#[test]
fn msg_sends_the_text_to_the_handle_and_prints_it() {
    let (base, server) = serve("201 Created", &sent_back("merging it now"));
    let output = against(
        "cli-msg",
        &base,
        &["msg", "@grace", "-m", "merging it now"],
        None,
    );
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(sent["body"], "merging it now");
    assert_eq!(
        sent["client_id"].as_str().map(str::len),
        Some(36),
        "each message carries a uuid so a retry can't send it twice: {sent}"
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[msg @grace] merging it now\n"
    );
}

#[test]
fn msg_reads_its_text_from_stdin_without_m() {
    let (base, server) = serve("201 Created", &sent_back("piped in\nsecond line"));
    let output = against(
        "cli-msg-stdin",
        &base,
        &["msg", "grace"],
        Some("piped in\nsecond line\n"),
    );
    let sent: serde_json::Value = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(sent["body"], "piped in\nsecond line");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[msg @grace] piped in\n"
    );
}

#[test]
fn an_empty_message_is_refused_before_anything_is_sent() {
    let output = against(
        "cli-msg-empty",
        "http://127.0.0.1:9",
        &["msg", "@grace"],
        Some("  \n"),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: nothing to send. the message is empty.\n"
    );
}

#[test]
fn msg_to_someone_dev_null_refuses_says_why() {
    let (base, _server) = serve(
        "403 Forbidden",
        r#"{"message":"@grace hasn't merged your request yet."}"#,
    );
    let output = against(
        "cli-msg-refused",
        &base,
        &["msg", "@grace", "-m", "hi"],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ttyl: @grace hasn't merged your request yet.\n"
    );
}

#[test]
fn headlines_prints_each_name_over_its_wrapped_recap_as_plain_text() {
    let hot = serde_json::json!({ "data": [
        {
            "id": "0199a000-0000-7000-8000-000000000001",
            "name": "Keyset pagination goes mainstream",
            "summary": "Teams moving to cursor pagination report steady p99s once tables pass a few million rows, though some miss jumping to page 40.",
        },
        { "id": "0199a000-0000-7000-8000-000000000002", "name": "Rust 2027 edition", "summary": null },
        { "id": "0199a000-0000-7000-8000-000000000003", "name": "Postgres 18 async IO", "summary": "Faster scans." },
    ] });
    let (base, _server) = serve("200 OK", &hot.to_string());
    let output = against("cli-headlines", &base, &["headlines"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Keyset pagination goes mainstream\n\
         \x20   Teams moving to cursor pagination report steady p99s once tables pass a\n\
         \x20   few million rows, though some miss jumping to page 40.\n\
         \n\
         Rust 2027 edition\n\
         \n\
         Postgres 18 async IO\n\
         \x20   Faster scans.\n"
    );
}

#[test]
fn headlines_with_nothing_hot_prints_nothing() {
    let (base, _server) = serve("200 OK", r#"{"data":[]}"#);
    let output = against("cli-headlines-none", &base, &["headlines"], None);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

#[test]
fn blocked_prints_one_handle_per_line_for_the_shell() {
    let person = |handle: &str| {
        serde_json::json!({
            "id": format!("id-{handle}"),
            "handle": handle,
            "name": handle,
            "avatar_url": null,
            "bio": null,
            "company": null,
            "location": null,
            "website": null,
            "html_url": format!("https://dev-null.sh/@{handle}"),
            "created_at": "2026-01-01T00:00:00Z",
            "lane": 1,
        })
    };
    let page = serde_json::json!({
        "data": [person("grace"), person("linus")],
        "meta": { "next_cursor": null },
    });
    let (base, _server) = serve("200 OK", &page.to_string());
    let output = against("cli-blocked", &base, &["blocked"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "grace\nlinus\n");
}

#[test]
fn muted_prints_one_handle_per_line_for_the_shell() {
    let page = serde_json::json!({
        "data": [{
            "id": "id-grace", "handle": "grace", "name": "grace", "avatar_url": null,
            "html_url": "https://dev-null.sh/@grace", "created_at": "2026-01-01T00:00:00Z", "lane": 1,
        }],
        "meta": { "next_cursor": null },
    });
    let (base, _server) = serve("200 OK", &page.to_string());
    let output = against("cli-muted", &base, &["muted"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "grace\n");
}

#[test]
fn devices_prints_one_device_per_line() {
    let devices = serde_json::json!({ "data": [{
        "id": "d-phone", "name": "my phone", "app_name": "dev-null", "fingerprint": "SHA256:phone",
        "approved_at": "2026-10-20T10:00:00Z", "last_used_at": "2026-10-21T08:00:00Z",
    }]});
    let (base, _server) = serve("200 OK", &devices.to_string());
    let output = against("cli-devices", &base, &["devices"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "d-phone\t2026-10-21T08:00:00Z\tSHA256:phone\tmy phone\n"
    );
}

#[test]
fn devices_revoke_takes_an_explicit_id() {
    let (base, server) = serve_in_turn(&[("200 OK", r#"{"data":[]}"#), ("204 No Content", "")]);
    let output = against("cli-revoke", &base, &["devices", "revoke", "d-phone"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(server.join().unwrap().len(), 2, "lists, then revokes");
    assert_eq!(ttyl(&["devices", "revoke"]).status.code(), Some(2));
}

#[test]
fn revoking_this_device_forgets_the_timeline_the_next_launch_would_draw() {
    let home = scratch("cli-revoke-this");
    let path = home.join("ttyl/device_ed25519");
    let key = DeviceKey::open(&path).unwrap();
    let devices = serde_json::json!({ "data": [{
        "id": "d-laptop", "name": "laptop", "app_name": "ttyl", "fingerprint": key.fingerprint(),
        "approved_at": "2026-10-20T10:00:00Z", "last_used_at": null,
    }]});
    let (base, server) = serve_in_turn(&[
        ("200 OK", ME),
        ("200 OK", &devices.to_string()),
        ("204 No Content", ""),
    ]);
    let cache = || Cache::under(&home.join("cache"), &base, &key);
    let timeline = Client::new(base.clone(), DeviceKey::load(&path).unwrap()).with_cache(cache());
    timeline.me().unwrap();
    assert!(cache().load().is_some(), "the timeline kept the account");

    let output = at_home(&home, &base, &["devices", "revoke", "d-laptop"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(server.join().unwrap().len(), 3);
    assert!(cache().load().is_none(), "nothing left to draw");
}

const ME: &str = r#"{"data":{"handle":"ada","name":"Ada","pasteis_mode":true,"preferred_lane":null,"automatic_lane":3,"remotes":null},"unread_notifications_count":0}"#;
const SETTINGS: &str = r#"{"data":{"timezone":"Europe/Lisbon","show_local_time":false,"translation_language":null,"automatic_translation":true,"translation_languages":[{"code":"fr","name":"French","native_name":"français"}],"is_admin":false,"push":{}}}"#;

#[test]
fn config_lists_settings_as_key_value_lines_leaving_out_unset_ones() {
    let (base, _server) = serve_in_turn(&[("200 OK", ME), ("200 OK", SETTINGS)]);
    let output = against("cli-config-list", &base, &["config"], None);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "account.pasteis=true\naccount.color=auto\nregion.timezone=Europe/Lisbon\nregion.local-time=false\nregion.auto-translate=true\nremotes.enabled=false\n"
    );
}

#[test]
fn config_prints_one_value_and_fails_quietly_when_it_is_unset() {
    let (base, _server) = serve_in_turn(&[("200 OK", ME), ("200 OK", SETTINGS)]);
    let output = against(
        "cli-config-get",
        &base,
        &["config", "region.timezone"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Europe/Lisbon\n");

    let (base, _server) = serve_in_turn(&[("200 OK", ME), ("200 OK", SETTINGS)]);
    let output = against(
        "cli-config-unset",
        &base,
        &["config", "region.language"],
        None,
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

#[test]
fn config_sets_a_value_through_its_route() {
    let (base, server) = serve_in_turn(&[("200 OK", ME), ("200 OK", SETTINGS), ("200 OK", ME)]);
    let output = against(
        "cli-config-set",
        &base,
        &["config", "region.language", "French"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let seen = server.join().unwrap();
    assert_eq!(seen[2], r#"{"translation_language":"fr"}"#);
}

#[test]
fn config_says_fatal_on_a_bad_key_or_value() {
    let output = ttyl(&["config", "region.tz"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "fatal: unknown key 'region.tz'. 'ttyl --help' lists them\n"
    );

    let (base, _server) = serve_in_turn(&[("200 OK", ME), ("200 OK", SETTINGS)]);
    let output = against(
        "cli-config-bad",
        &base,
        &["config", "account.pasteis", "maybe"],
        None,
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "fatal: bad boolean config value 'maybe' for 'account.pasteis'\n"
    );
}
