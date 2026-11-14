mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};

use common::scratch;

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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = thread::spawn(move || {
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
    });
    (base, server)
}

/// Runs ttyl against `base` with its key in a scratch config, feeding `stdin` when given.
fn against(name: &str, base: &str, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ttyl"))
        .args(args)
        .env("XDG_CONFIG_HOME", scratch(name))
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
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
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
