//! The screen a device without approval opens on. It sends the user to dev-null.sh to approve this terminal and
//! waits for the browser to hand the approval back.

use std::fs;
use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::api::{Client, Me};
use crate::signin::{Attempt, Callback, Handler, Inbox};
use crate::theme;

enum Step {
    /// Waiting for the user to start, with what went wrong last time.
    Ready(Option<String>),
    Browser(Wait),
    Redeeming {
        code: String,
        verifier: String,
    },
}

/// Holds the `devnull://` handler while the browser is out, and gives it back when dropped.
struct Wait {
    attempt: Attempt,
    url: String,
    inbox: Inbox,
    _handler: Handler,
}

/// Runs until the device is approved, or `None` when the user quits first.
pub fn run(terminal: &mut DefaultTerminal, client: &Client) -> io::Result<Option<Me>> {
    let device_name = device_name();
    let mut step = Step::Ready(None);
    loop {
        terminal.draw(|frame| draw(frame, &step, &device_name))?;
        step = match step {
            Step::Redeeming { code, verifier } => match client.redeem(&code, &verifier) {
                Ok(account) => return Ok(Some(account.me)),
                Err(error) => Step::Ready(Some(error.to_string())),
            },
            Step::Browser(wait) => match wait.inbox.poll() {
                Ok(link) => match link.and_then(|link| wait.attempt.callback(&link)) {
                    Some(Callback::Approved(code)) => Step::Redeeming {
                        code,
                        verifier: wait.attempt.verifier,
                    },
                    Some(Callback::Denied) => {
                        Step::Ready(Some("You cancelled in the browser.".to_owned()))
                    }
                    None => Step::Browser(wait),
                },
                Err(error) => Step::Ready(Some(error.to_string())),
            },
            ready => ready,
        };
        if matches!(step, Step::Redeeming { .. }) || !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match (&step, key.code) {
            (_, KeyCode::Char('Q')) => return Ok(None),
            (_, KeyCode::Char('c')) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(None);
            }
            (Step::Ready(_), KeyCode::Enter) => {
                step = start(client, &device_name)
                    .unwrap_or_else(|error| Step::Ready(Some(error.to_string())));
            }
            (Step::Browser(_), KeyCode::Esc) => step = Step::Ready(None),
            _ => {}
        }
    }
}

fn start(client: &Client, device_name: &str) -> io::Result<Step> {
    let handler = Handler::install()?;
    let inbox = Inbox::bind()?;
    let attempt = Attempt::random();
    let url = attempt.authorize_url(client.base(), &client.key().fingerprint(), device_name);
    // The link stays on screen, so a desktop without a browser to open still lets the user finish by hand.
    if let Ok(mut browser) = Command::new("xdg-open")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        thread::spawn(move || browser.wait());
    }
    Ok(Step::Browser(Wait {
        attempt,
        url,
        inbox,
        _handler: handler,
    }))
}

/// What dev-null.sh lists this terminal as, within the server's 100 characters.
fn device_name() -> String {
    let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    match host.trim() {
        "" => "devnull-tui".to_owned(),
        host => format!("devnull-tui on {host}").chars().take(100).collect(),
    }
}

fn draw(frame: &mut Frame, step: &Step, device_name: &str) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::new().bg(theme::BACKGROUND).fg(theme::FOREGROUND)),
        area,
    );
    let mut lines = vec![
        Line::styled(
            "/dev/null",
            Style::new().fg(theme::BRAND).add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    let keys = match step {
        Step::Ready(notice) => {
            lines.push(Line::raw("Sign in to read your logs."));
            lines.push(Line::raw(format!(
                "Your browser opens dev-null.sh, where you approve this terminal as \"{device_name}\"."
            )));
            if let Some(notice) = notice {
                lines.push(Line::default());
                lines.push(Line::styled(
                    notice.as_str(),
                    Style::new().fg(theme::DESTRUCTIVE),
                ));
            }
            "enter sign in  Q quit"
        }
        Step::Browser(wait) => {
            lines.push(Line::raw("Waiting for you in the browser."));
            lines.push(Line::raw(format!(
                "Approve \"{device_name}\" on dev-null.sh to finish."
            )));
            lines.push(Line::default());
            lines.push(Line::styled(
                "If no browser opened, visit this link:",
                theme::muted(),
            ));
            lines.push(Line::styled(
                wait.url.as_str(),
                Style::new().fg(theme::PRIMARY),
            ));
            "esc start over  Q quit"
        }
        Step::Redeeming { .. } => {
            lines.push(Line::raw("Signing in."));
            ""
        }
    };
    lines.push(Line::default());
    lines.push(Line::styled(keys, theme::muted()));

    let width = area.width.saturating_sub(4).min(72);
    let column = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height / 5,
        width,
        height: area.height - area.height / 5,
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), column);
}
