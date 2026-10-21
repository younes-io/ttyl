#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use devnull::device::DeviceKey;

pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `ssh-keygen -Y verify` the way an SSHSIG verifier independent of this crate would check the request.
pub fn openssh_accepts(
    key: &DeviceKey,
    signature: &str,
    message: &str,
    namespace: &str,
    dir: &str,
) -> bool {
    let dir = scratch(dir);
    fs::write(
        dir.join("allowed"),
        format!("devnull {}\n", key.public_key()),
    )
    .unwrap();
    let body: Vec<&str> = signature
        .as_bytes()
        .chunks(70)
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect();
    fs::write(
        dir.join("sig"),
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            body.join("\n")
        ),
    )
    .unwrap();
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-I", "devnull", "-n", namespace, "-f"])
        .arg(dir.join("allowed"))
        .arg("-s")
        .arg(dir.join("sig"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("ssh-keygen is installed");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(message.as_bytes())
        .unwrap();
    child.wait().unwrap().success()
}

use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset};
use devnull::api::{Account, Call, Reply};
use devnull::engine::{Effect, Engine, Event};
use devnull::model::{CommitId, Log, LogPage};
use devnull::signin::{Attempt, REDIRECT_URI};
use devnull::welcome::Wait;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn page(log: Log) -> LogPage {
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/log-{}.json",
        env!("CARGO_MANIFEST_DIR"),
        log.slug()
    ))
    .unwrap();
    LogPage::parse(&json).unwrap()
}

pub fn older(log: Log, prefix: &str, count: usize, next_cursor: Option<&str>) -> LogPage {
    let first = page(log);
    let rows = (0..count)
        .map(|index| {
            let mut row = first.rows[index % first.rows.len()].clone();
            row.commit.id = CommitId(format!("{prefix}-{index}"));
            row.commit.short_hash = format!("{prefix}{index:03}");
            row
        })
        .collect();
    LogPage {
        rows,
        next_cursor: next_cursor.map(str::to_owned),
        ..first
    }
}

pub fn cursor(log: Log) -> String {
    page(log)
        .next_cursor
        .expect("the fixture has an older page")
}

pub fn runs(effects: &[Effect]) -> Vec<&Call> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Run(_, call) => Some(call),
            Effect::StartSignIn | Effect::Quit | Effect::Abort(_) => None,
        })
        .collect()
}

pub fn account(unread: u32) -> Account {
    serde_json::from_value(serde_json::json!({
        "data": { "username": "ada", "name": "Ada" },
        "unread_notifications_count": unread,
    }))
    .unwrap()
}

pub fn now() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-21T10:00:00+00:00").unwrap()
}

pub fn following_nobody() -> LogPage {
    LogPage {
        rows: Vec::new(),
        ..page(Log::Following)
    }
}

pub fn boot(at: Instant) -> Engine {
    boot_serving(at, page)
}

pub fn boot_serving(at: Instant, pages: fn(Log) -> LogPage) -> Engine {
    let mut engine = Engine::new("devnull-tui".to_owned(), at);
    let effects = engine.pump(at);
    let [Effect::Run(ticket, Call::Me)] = effects.as_slice() else {
        panic!("the engine asks for /me first: {effects:?}");
    };
    engine.update(Event::Reply(*ticket, Ok(Reply::Account(account(3)))), at);
    serve_with(&mut engine, at, pages);
    engine
}

pub fn serve(engine: &mut Engine, at: Instant) {
    serve_with(engine, at, page);
}

pub fn serve_with(engine: &mut Engine, at: Instant, pages: fn(Log) -> LogPage) {
    loop {
        let effects = engine.pump(at);
        if effects.is_empty() {
            return;
        }
        for effect in effects {
            match effect {
                Effect::Run(ticket, Call::Page(log)) => {
                    engine.update(Event::Reply(ticket, Ok(Reply::Page(pages(log)))), at);
                }
                Effect::Run(ticket, Call::Me) => {
                    engine.update(Event::Reply(ticket, Ok(Reply::Account(account(3)))), at);
                }
                other => panic!("unexpected effect while serving: {other:?}"),
            }
        }
    }
}

pub struct Browser {
    pub state: String,
    pub verifier: String,
}

impl Browser {
    pub fn redirect(&self, query: &str) -> String {
        format!("{REDIRECT_URI}?{query}&state={}", self.state)
    }
}

pub fn open_browser(engine: &mut Engine, at: Instant) -> Browser {
    let effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        at,
    );
    assert_eq!(effects, [Effect::StartSignIn]);
    let attempt = Attempt::random();
    let browser = Browser {
        state: attempt.state.clone(),
        verifier: attempt.verifier.clone(),
    };
    let url = attempt.authorize_url("https://dev-null.sh", "SHA256:test", "devnull-tui");
    engine.update(Event::SignIn(Ok(Wait::without_listener(attempt, url))), at);
    browser
}

pub fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

pub fn press(engine: &mut Engine, keys: &str, at: Instant) -> Vec<Effect> {
    let mut effects = Vec::new();
    for ch in keys.chars() {
        effects.extend(engine.update(Event::Key(key(ch)), at));
        effects.extend(engine.pump(at));
    }
    effects
}

pub fn buffer(engine: &mut Engine, width: u16, height: u16, at: Instant) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| engine.draw(frame, at, &now()))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn screen(engine: &mut Engine, width: u16, height: u16, at: Instant) -> Vec<String> {
    let buffer = buffer(engine, width, height, at);
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

pub fn contains(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|row| row.contains(needle))
}

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub fn tap(engine: &mut Engine, code: KeyCode, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), at);
    effects.extend(engine.pump(at));
    effects
}
