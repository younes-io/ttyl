use std::fs;
use std::io;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::api::Client;
use crate::engine::Event;
use crate::signin::{Attempt, Handler, Inbox};
use crate::theme;

pub enum Step {
    /// Waiting for the user to start, with what went wrong last time.
    Ready(Option<String>),
    Browser(Wait),
    Redeeming,
}

pub struct Wait {
    pub attempt: Attempt,
    pub url: String,
    _listener: Option<Listener>,
}

impl Wait {
    /// A wait with nothing listening for the redirect, so links reach it only through `Event::Link`.
    pub fn new(attempt: Attempt, url: String) -> Self {
        Wait {
            attempt,
            url,
            _listener: None,
        }
    }
}

/// Holds the `devnull://` handler while the browser is out, and gives it back when dropped.
struct Listener {
    _handler: Handler,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(client: &Client, device_name: &str, events: Sender<Event>) -> io::Result<Wait> {
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
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match inbox.poll() {
                    Ok(None) => thread::sleep(Duration::from_millis(100)),
                    Ok(Some(link)) => {
                        let _ = events.send(Event::Link(Ok(link)));
                    }
                    Err(error) => {
                        let _ = events.send(Event::Link(Err(error)));
                        return;
                    }
                }
            }
        })
    };
    Ok(Wait {
        attempt,
        url,
        _listener: Some(Listener {
            _handler: handler,
            stop,
            thread: Some(thread),
        }),
    })
}

/// What dev-null.sh lists this terminal as, within the server's 100 characters.
pub fn device_name() -> String {
    let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    match host.trim() {
        "" => "devnull-tui".to_owned(),
        host => format!("devnull-tui on {host}").chars().take(100).collect(),
    }
}

pub fn draw(frame: &mut Frame, step: &Step, device_name: &str, keys: &str) {
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
    match step {
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
        }
        Step::Redeeming => lines.push(Line::raw("Signing in.")),
    }
    lines.push(Line::default());
    lines.push(Line::styled(keys.to_owned(), theme::muted()));

    let width = area.width.saturating_sub(4).min(72);
    let column = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height / 5,
        width,
        height: area.height - area.height / 5,
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), column);
}
