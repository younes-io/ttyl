use std::collections::HashSet;
use std::io;
use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::api::{Account, Call, Error, Reply};
use crate::clock::{FRAME, MINUTE, POLL, RETRY, SECOND, TOAST, Timer, Timers};
use crate::keys::{
    self, Action, GATE_SCOPES, HELP_SCOPES, InputMode, Keymap, LOG_SCOPES, Resolve, SPLASH_SCOPES,
    Scope,
};
use crate::log::{LOGS, LogScreen, Retry};
use crate::model::Log;
use crate::sched::{Priority, Scheduler, Ticket};
use crate::signin::Callback;
use crate::store::{Slot, Store};
use crate::theme;
use crate::welcome::{self, Step, Wait};

pub const WORKERS: usize = 2;
pub const AHEAD: usize = 5;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub enum Event {
    Key(KeyEvent),
    Reply(Ticket, Result<Reply, Error>),
    Tick,
    SignIn(io::Result<Wait>),
    Link(io::Result<String>),
    InputLost(io::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Run(Ticket, Call),
    StartSignIn,
    Quit,
    Abort(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Down,
    Offline(String),
    Refused(String),
}

impl Failure {
    fn of(error: &Error) -> Self {
        match error {
            Error::Unexpected(_) => Failure::Down,
            Error::Http(error) => Failure::Offline(error.to_string()),
            Error::Api { message, .. } => Failure::Refused(message.clone()),
            Error::Malformed(_) => Failure::Refused(error.to_string()),
        }
    }
}

pub enum Phase {
    Splash,
    SignIn(Step),
    Ready(Session),
}

pub struct Session {
    pub store: Store,
    pub log: LogScreen,
    help: bool,
    asked_to_refresh: HashSet<Log>,
}

impl Session {
    fn log_allows(&self, action: Action) -> bool {
        matches!(action, Action::Quit | Action::Help) || self.log.can(action, &self.store)
    }

    fn can(&self, action: Action) -> bool {
        if self.help {
            matches!(action, Action::Quit | Action::Close)
        } else {
            self.log_allows(action)
        }
    }
}

fn can(phase: &Phase, health: Option<&Failure>, action: Action) -> bool {
    match (phase, action) {
        (_, Action::Quit) => true,
        (Phase::Splash, Action::Refresh) => health.is_some(),
        (Phase::Splash, _) => false,
        (Phase::SignIn(step), Action::SignIn) => matches!(step, Step::Ready(_)),
        (Phase::SignIn(step), Action::StartOver) => matches!(step, Step::Browser(_)),
        (Phase::SignIn(_), _) => false,
        (Phase::Ready(session), action) => session.can(action),
    }
}

pub struct Engine {
    phase: Phase,
    health: Option<Failure>,
    sched: Scheduler,
    timers: Timers,
    keymap: Keymap,
    toast: Option<String>,
    device_name: String,
    start: Instant,
}

impl Engine {
    pub fn new(device_name: String, now: Instant) -> Self {
        let mut sched = Scheduler::new(WORKERS);
        sched.send(Call::Me, Priority::Now);
        Engine {
            phase: Phase::Splash,
            health: None,
            sched,
            timers: Timers::default(),
            keymap: Keymap::default(),
            toast: None,
            device_name,
            start: now,
        }
    }

    pub fn log(&self) -> Option<&LogScreen> {
        match &self.phase {
            Phase::Ready(session) => Some(&session.log),
            Phase::Splash | Phase::SignIn(_) => None,
        }
    }

    pub fn update(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        match event {
            Event::Key(key) => self.key(key),
            Event::Reply(ticket, result) => {
                if let Some((call, priority)) = self.sched.finish(ticket) {
                    match result {
                        Ok(reply) => {
                            self.health = None;
                            self.arrived(&call, reply, now);
                        }
                        Err(error) => self.failed(call, priority, error, now),
                    }
                }
                Vec::new()
            }
            Event::Tick => Vec::new(),
            Event::SignIn(result) => {
                if let Phase::SignIn(step) = &mut self.phase {
                    *step = match result {
                        Ok(wait) => Step::Browser(wait),
                        Err(error) => Step::Ready(Some(error.to_string())),
                    };
                }
                Vec::new()
            }
            Event::Link(result) => {
                self.link(result);
                Vec::new()
            }
            Event::InputLost(error) => {
                vec![Effect::Abort(format!("can't read the keyboard: {error}"))]
            }
        }
    }

    pub fn pump(&mut self, now: Instant) -> Vec<Effect> {
        for timer in self.timers.due(now) {
            match timer {
                Timer::Poll => {
                    self.sched.send(Call::Me, Priority::Later);
                    self.timers.set(Timer::Poll, now + POLL);
                }
                Timer::Retry => self.sched.send(Call::Me, Priority::Now),
                Timer::RelativeTimes => self.timers.set(Timer::RelativeTimes, now + MINUTE),
                Timer::Toast => self.toast = None,
                Timer::Reload(log) => self.sched.send(Call::Page(log), Priority::Later),
                Timer::Older(_) => {}
            }
        }
        self.load_older();
        self.sched
            .release(now)
            .into_iter()
            .map(|(ticket, call)| Effect::Run(ticket, call))
            .collect()
    }

    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        [
            self.timers.next(),
            self.sched.next_wake(now),
            self.sched.busy().then(|| now + FRAME),
            self.counting_down().then(|| now + SECOND),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Asks for the active log's next page once the cursor is within `AHEAD` commits of the end, unless the last
    /// try failed and its retry is not due yet.
    fn load_older(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        let log = session.log.active();
        let Slot::Loaded(listing) = session.store.slot(log) else {
            return;
        };
        let Some(cursor) = &listing.next_cursor else {
            return;
        };
        let left = listing.ids.len().saturating_sub(session.log.selected() + 1);
        if left <= AHEAD && !self.timers.is_set(Timer::Older(log)) {
            let cursor = cursor.clone();
            self.sched.send(Call::Older { log, cursor }, Priority::Now);
        }
    }

    fn counting_down(&self) -> bool {
        self.timers.is_set(Timer::Retry)
            || LOGS.iter().any(|log| {
                self.timers.is_set(Timer::Reload(*log)) || self.timers.is_set(Timer::Older(*log))
            })
    }

    fn scopes(&self) -> &'static [Scope] {
        match &self.phase {
            Phase::Splash => SPLASH_SCOPES,
            Phase::SignIn(_) => GATE_SCOPES,
            Phase::Ready(session) if session.help => HELP_SCOPES,
            Phase::Ready(_) => LOG_SCOPES,
        }
    }

    fn input_mode(&self) -> InputMode {
        match &self.phase {
            Phase::Ready(session) => session.log.input_mode(),
            Phase::Splash | Phase::SignIn(_) => InputMode::Commands,
        }
    }

    fn key(&mut self, event: KeyEvent) -> Vec<Effect> {
        let scopes = self.scopes();
        let mode = self.input_mode();
        let Engine {
            keymap,
            phase,
            health,
            ..
        } = self;
        let can = |action| can(phase, health.as_ref(), action);
        match keymap.resolve(event, mode, scopes, &can) {
            Resolve::Act(action) => self.act(action),
            Resolve::Pending | Resolve::Text(_) | Resolve::Ignored => Vec::new(),
        }
    }

    fn act(&mut self, action: Action) -> Vec<Effect> {
        match (&mut self.phase, action) {
            (_, Action::Quit) => return vec![Effect::Quit],
            (Phase::Splash, Action::Refresh) => {
                self.timers.clear(Timer::Retry);
                self.sched.send(Call::Me, Priority::Now);
            }
            (Phase::SignIn(_), Action::SignIn) => return vec![Effect::StartSignIn],
            (Phase::SignIn(step), Action::StartOver) => *step = Step::Ready(None),
            (Phase::Ready(session), Action::Refresh) => {
                let log = session.log.active();
                session.asked_to_refresh.insert(log);
                self.timers.clear(Timer::Reload(log));
                self.timers.clear(Timer::Older(log));
                self.sched.send(Call::Page(log), Priority::Now);
            }
            (Phase::Ready(session), Action::Help) => session.help = true,
            (Phase::Ready(session), Action::Close) => session.help = false,
            (Phase::Ready(session), action) => session.log.act(action, &session.store),
            (Phase::Splash | Phase::SignIn(_), _) => {}
        }
        Vec::new()
    }

    fn arrived(&mut self, call: &Call, reply: Reply, now: Instant) {
        let Phase::Ready(session) = &mut self.phase else {
            if let Reply::Account(account) = reply {
                self.open(account, now);
            }
            return;
        };
        let refreshed = match call {
            Call::Page(log) if session.asked_to_refresh.remove(log) => Some(*log),
            _ => None,
        };
        session.store.apply(call, reply);
        if let Some(log) = refreshed {
            self.toast(format!("{} refreshed", log.slug()), now);
        }
    }

    fn open(&mut self, account: Account, now: Instant) {
        self.phase = Phase::Ready(Session {
            store: Store::new(account),
            log: LogScreen::default(),
            help: false,
            asked_to_refresh: HashSet::new(),
        });
        for log in LOGS {
            self.sched.send(Call::Page(log), Priority::Now);
        }
        self.timers.clear(Timer::Retry);
        self.timers.set(Timer::Poll, now + POLL);
        self.timers.set(Timer::RelativeTimes, now + MINUTE);
    }

    fn failed(&mut self, call: Call, priority: Priority, error: Error, now: Instant) {
        if let Error::Api {
            status: 429,
            retry_after,
            ..
        } = &error
        {
            let wait = retry_after.unwrap_or(RETRY);
            self.sched.hold(now + wait);
            self.sched.send(call, priority);
            if matches!(self.phase, Phase::Ready(_)) {
                self.toast(
                    format!("rate limited, retrying in {}s", wait.as_secs()),
                    now,
                );
            }
            return;
        }
        if error.needs_sign_in() && !matches!(self.phase, Phase::SignIn(_)) {
            self.sign_out();
            return;
        }
        let failure = Failure::of(&error);
        match &mut self.phase {
            Phase::Splash => {
                self.health = Some(failure);
                self.timers.set(Timer::Retry, now + RETRY);
            }
            Phase::SignIn(step) => *step = Step::Ready(Some(error.to_string())),
            Phase::Ready(session) => {
                match call {
                    Call::Page(log) => {
                        session.asked_to_refresh.remove(&log);
                        session.store.fail(log, error.to_string());
                        self.timers.set(Timer::Reload(log), now + RETRY);
                    }
                    Call::Older { log, .. } => self.timers.set(Timer::Older(log), now + RETRY),
                    Call::Me | Call::Redeem { .. } => {}
                }
                self.health = Some(failure);
            }
        }
    }

    fn sign_out(&mut self) {
        self.sched.reset();
        self.phase = Phase::SignIn(Step::Ready(None));
        self.health = None;
        self.timers = Timers::default();
        self.toast = None;
    }

    fn link(&mut self, result: io::Result<String>) {
        let Phase::SignIn(step) = &mut self.phase else {
            return;
        };
        let Step::Browser(wait) = step else {
            return;
        };
        match result {
            Ok(link) => match wait.attempt.callback(&link) {
                Some(Callback::Approved(code)) => {
                    let verifier = wait.attempt.verifier.clone();
                    self.sched
                        .send(Call::Redeem { code, verifier }, Priority::Now);
                    *step = Step::Redeeming;
                }
                Some(Callback::Denied) => {
                    *step = Step::Ready(Some("You cancelled in the browser.".to_owned()));
                }
                None => {}
            },
            Err(error) => *step = Step::Ready(Some(error.to_string())),
        }
    }

    fn toast(&mut self, text: String, now: Instant) {
        self.toast = Some(text);
        self.timers.set(Timer::Toast, now + TOAST);
    }

    pub fn draw<Tz: TimeZone>(&mut self, frame: &mut Frame, now: Instant, wall: &DateTime<Tz>) {
        let hints = keys::hints(self.scopes(), self.keymap.pending(), &|action| {
            can(&self.phase, self.health.as_ref(), action)
        });
        let bottom = match &self.toast {
            Some(toast) if self.keymap.pending().is_empty() => toast.clone(),
            _ => hints,
        };
        match &mut self.phase {
            Phase::Splash => self.draw_splash(frame, now, &bottom),
            Phase::SignIn(step) => welcome::draw(frame, step, &self.device_name, &bottom),
            Phase::Ready(session) => {
                let area = frame.area();
                frame.render_widget(
                    Block::new().style(Style::new().bg(theme::BACKGROUND).fg(theme::FOREGROUND)),
                    area,
                );
                let [top, banner, body, foot] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Fill(1),
                    Constraint::Length(1),
                ])
                .areas(area);
                frame.render_widget(session.log.tabs(), top);
                frame.render_widget(account_line(&session.store).right_aligned(), top);
                if let Some(failure) = &self.health {
                    frame.render_widget(
                        Line::styled(
                            format!(" {}", banner_text(failure)),
                            Style::new().fg(theme::DESTRUCTIVE),
                        ),
                        banner,
                    );
                }
                let active = session.log.active();
                let retry = Retry {
                    page: self.timers.remaining(Timer::Reload(active), now),
                    older: self.timers.remaining(Timer::Older(active), now),
                };
                session
                    .log
                    .draw(frame, inset(body), &session.store, retry, wall);
                frame.render_widget(Line::styled(format!(" {bottom}"), theme::muted()), foot);
                if self.sched.busy() {
                    let frame_index =
                        usize::try_from((now - self.start).as_millis() / 100).unwrap_or(0);
                    frame.render_widget(
                        Line::styled(
                            format!("{} ", SPINNER[frame_index % SPINNER.len()]),
                            Style::new().fg(theme::PRIMARY),
                        )
                        .right_aligned(),
                        foot,
                    );
                }
                if session.help {
                    draw_help(frame, session);
                }
            }
        }
    }

    fn draw_splash(&self, frame: &mut Frame, now: Instant, hints: &str) {
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
        match &self.health {
            None => lines.push(Line::styled("Loading…", theme::muted())),
            Some(failure) => {
                lines.push(Line::styled(
                    "fatal: unable to access 'https://dev-null.sh/':",
                    Style::new().fg(theme::DESTRUCTIVE),
                ));
                match failure {
                    Failure::Down => {
                        lines.push(Line::raw("  the site is in maintenance"));
                        lines.push(Line::default());
                        lines.push(Line::styled(
                            "We are not down, we're making sure the only",
                            theme::muted(),
                        ));
                        lines.push(Line::styled(
                            "remote code running here is ours.",
                            theme::muted(),
                        ));
                    }
                    Failure::Offline(detail) | Failure::Refused(detail) => {
                        lines.push(Line::raw(format!("  {detail}")));
                    }
                }
                if let Some(left) = self.timers.remaining(Timer::Retry, now) {
                    lines.push(Line::default());
                    lines.push(Line::styled(
                        format!("Retrying in {}s.", left.as_secs()),
                        theme::muted(),
                    ));
                }
            }
        }
        lines.push(Line::default());
        lines.push(Line::styled(hints.to_owned(), theme::muted()));
        let column = Rect {
            y: area.y + area.height / 5,
            height: area.height - area.height / 5,
            ..centered(area, 72)
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), column);
    }
}

fn account_line(store: &Store) -> Line<'static> {
    let mut spans = Vec::new();
    if store.unread > 0 {
        spans.push(Span::styled("● ", Style::new().fg(theme::PRIMARY)));
        spans.push(Span::styled(format!("{}  ", store.unread), theme::muted()));
    }
    spans.push(Span::styled(
        format!("@{} ", store.me.username),
        theme::muted(),
    ));
    Line::from(spans)
}

fn banner_text(failure: &Failure) -> String {
    match failure {
        Failure::Down => "fatal: the site is in maintenance · your log is still here".to_owned(),
        Failure::Offline(_) => {
            "fatal: could not reach dev-null.sh · your log is still here".to_owned()
        }
        Failure::Refused(message) => format!("fatal: {message}"),
    }
}

fn draw_help(frame: &mut Frame, session: &Session) {
    let rows: Vec<String> = keys::help(LOG_SCOPES, &|action| session.log_allows(action))
        .into_iter()
        .map(|(key, help)| format!("{key:>8}  {help}"))
        .collect();
    let width = rows
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0)
        + 4;
    let height = rows.len() + 2;
    let area = frame.area();
    let width = u16::try_from(width).unwrap_or(u16::MAX).min(area.width);
    let height = u16::try_from(height).unwrap_or(u16::MAX).min(area.height);
    let box_area = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, box_area);
    let block = Block::new()
        .borders(Borders::ALL)
        .title(" keys ")
        .style(Style::new().bg(theme::BACKGROUND).fg(theme::FOREGROUND));
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);
    let lines: Vec<Line> = rows
        .into_iter()
        .map(|row| Line::raw(format!(" {row}")))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    }
}

fn centered(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    Rect {
        x: area.x + (area.width - width) / 2,
        width,
        ..area
    }
}
