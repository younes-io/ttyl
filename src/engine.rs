use std::collections::HashSet;
use std::io;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::api::{Account, Call, Error, Failure, Reply};
use crate::clock::{FRAME, MINUTE, POLL, RETRY, SECOND, TOAST, Timer, Timers};
use crate::fatal;
use crate::keys::{
    self, Act, Action, GATE_SCOPES, HELP_SCOPES, InputMode, Keymap, LOG_SCOPES, Resolve,
    SPLASH_SCOPES, Scope, THREAD_SCOPES,
};
use crate::log::{LOGS, LogScreen, Retry};
use crate::model::{CommitId, Log};
use crate::sched::{Priority, Scheduler, Ticket};
use crate::signin::Callback;
use crate::store::{Node, Slot, Store};
use crate::theme;
use crate::thread::ThreadScreen;
use crate::welcome::{self, Step, Wait};

pub const WORKERS: usize = 2;
pub const AHEAD: usize = 5;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_CELLS: usize = 2;
const HINT_INDENT: &str = " ";

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
    Copy(String),
    Open(String),
}

pub enum Phase {
    Splash,
    SignIn(Step),
    Ready(Box<Session>),
}

pub struct Session {
    pub store: Store,
    pub log: LogScreen,
    threads: Vec<ThreadScreen>,
    help: bool,
    asked_to_refresh: HashSet<Log>,
}

impl Session {
    fn screen_scopes(&self) -> &'static [Scope] {
        if self.threads.is_empty() {
            LOG_SCOPES
        } else {
            THREAD_SCOPES
        }
    }

    fn screen_allows(&self, action: Action) -> bool {
        if let Action::Commit(act) = action {
            return self.allows(act);
        }
        matches!(action, Action::Quit | Action::Help)
            || match self.threads.last() {
                Some(thread) => thread.can(action, &self.store),
                None => self.log.can(action, &self.store),
            }
    }

    fn selected(&self) -> Option<&CommitId> {
        match self.threads.last() {
            Some(thread) => Some(thread.selected()),
            None => self.log.selected_id(self.log.active(), &self.store),
        }
    }

    fn allows(&self, act: Act) -> bool {
        let Some(Node::Live(row)) = self.selected().and_then(|id| self.store.node(id)) else {
            return false;
        };
        let own = row.commit.author.username == self.store.me.username;
        row.deletion.is_none()
            && match act {
                Act::Star | Act::Follow => !own,
                Act::Pastel => !own && self.store.me.pasteis_mode,
                Act::Copy | Act::Browse => true,
            }
    }

    fn on_thread(&mut self, action: Action, now: Instant) -> Option<CommitId> {
        match action {
            Action::Back => {
                self.threads.pop();
            }
            Action::Show(_) | Action::NextLog => {
                self.threads.clear();
                self.log.act(action, &self.store);
            }
            Action::Open => {
                if let Some(id) = self
                    .threads
                    .last()
                    .and_then(|thread| thread.opens(&self.store))
                {
                    self.threads.push(ThreadScreen::new(id.clone(), now));
                    return Some(id);
                }
            }
            action => {
                if let Some(thread) = self.threads.last_mut() {
                    thread.act(action, &self.store);
                }
            }
        }
        None
    }

    fn can(&self, action: Action) -> bool {
        if self.help {
            matches!(action, Action::Quit | Action::Close)
        } else {
            self.screen_allows(action)
        }
    }
}

fn fetch_thread(sched: &mut Scheduler, timers: &mut Timers, id: CommitId) {
    timers.clear(&Timer::Commit(id.clone()));
    timers.clear(&Timer::Replies(id.clone()));
    timers.clear(&Timer::More(id.clone()));
    sched.send(Call::Commit(id.clone()), Priority::Now);
    sched.send(Call::Replies { id, cursor: None }, Priority::Now);
}

fn followed(username: &str, on: bool) -> String {
    let done = if on { "following" } else { "unfollowed" };
    format!("{done} @{username} · f to undo")
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

    pub fn thread(&self) -> Option<&ThreadScreen> {
        match &self.phase {
            Phase::Ready(session) => session.threads.last(),
            Phase::Splash | Phase::SignIn(_) => None,
        }
    }

    pub fn update(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        match event {
            Event::Key(key) => self.key(key, now),
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
                    self.poll_new();
                    self.timers.set(Timer::Poll, now + POLL);
                }
                Timer::Retry => self.sched.send(Call::Me, Priority::Now),
                Timer::RelativeTimes => self.timers.set(Timer::RelativeTimes, now + MINUTE),
                Timer::Toast => self.toast = None,
                Timer::Reload(log) => self.sched.send(Call::Page(log), Priority::Later),
                Timer::Older(_) | Timer::Commit(_) | Timer::Replies(_) | Timer::More(_) => {}
            }
        }
        self.load_older();
        for (call, priority) in self.thread_wants() {
            self.sched.send(call, priority);
        }
        if self.health.is_none()
            && let Phase::Ready(session) = &mut self.phase
        {
            let sched = &self.sched;
            for call in session.store.writes(|key| sched.pending(key)) {
                self.sched.send(call, Priority::Now);
            }
        }
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
            (!self.thread_wants().is_empty()).then_some(now),
            self.placeholder_at().filter(|at| *at > now),
        ]
        .into_iter()
        .flatten()
        .min()
    }

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
        if left <= AHEAD && !self.timers.is_set(&Timer::Older(log)) {
            let cursor = cursor.clone();
            self.sched.send(Call::Older { log, cursor }, Priority::Now);
        }
    }

    fn thread_wants(&self) -> Vec<(Call, Priority)> {
        let Phase::Ready(session) = &self.phase else {
            return Vec::new();
        };
        let Some(thread) = session.threads.last() else {
            return Vec::new();
        };
        thread
            .wants(&session.store)
            .into_iter()
            .filter(|(call, _, timer)| {
                !self.timers.is_set(timer) && !self.sched.pending(&call.key())
            })
            .map(|(call, priority, _)| (call, priority))
            .collect()
    }

    fn placeholder_at(&self) -> Option<Instant> {
        let Phase::Ready(session) = &self.phase else {
            return None;
        };
        session.threads.last()?.placeholder_at(&session.store)
    }

    fn poll_new(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        let log = session.log.active();
        if log == Log::Trending {
            return;
        }
        if let Slot::Loaded(listing) = session.store.slot(log)
            && let Some(newest) = listing.ids.first()
        {
            let since = newest.clone();
            self.sched.send(Call::Since { log, since }, Priority::Later);
        }
    }

    fn next_call(&self, now: Instant) -> Option<Duration> {
        [Timer::Retry, Timer::Poll]
            .into_iter()
            .chain(LOGS.iter().map(|log| Timer::Reload(*log)))
            .filter_map(|timer| self.timers.remaining(&timer, now))
            .min()
    }

    fn counting_down(&self) -> bool {
        self.timers.is_set(&Timer::Retry)
            || LOGS.iter().any(|log| {
                self.timers.is_set(&Timer::Reload(*log)) || self.timers.is_set(&Timer::Older(*log))
            })
            || self
                .timers
                .any(|timer| matches!(timer, Timer::Commit(_) | Timer::Replies(_) | Timer::More(_)))
    }

    fn scopes(&self) -> &'static [Scope] {
        match &self.phase {
            Phase::Splash => SPLASH_SCOPES,
            Phase::SignIn(_) => GATE_SCOPES,
            Phase::Ready(session) if session.help => HELP_SCOPES,
            Phase::Ready(session) => session.screen_scopes(),
        }
    }

    fn input_mode(&self) -> InputMode {
        match &self.phase {
            Phase::Ready(session) => session.log.input_mode(),
            Phase::Splash | Phase::SignIn(_) => InputMode::Commands,
        }
    }

    fn key(&mut self, event: KeyEvent, now: Instant) -> Vec<Effect> {
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
            Resolve::Act(action) => self.act(action, now),
            Resolve::Pending | Resolve::Text(_) | Resolve::Ignored => Vec::new(),
        }
    }

    fn act(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match (&mut self.phase, action) {
            (_, Action::Quit) => return vec![Effect::Quit],
            (Phase::Ready(session), Action::Help) => session.help = true,
            (Phase::Ready(session), Action::Close) => session.help = false,
            (Phase::Splash, Action::Refresh) => {
                self.timers.clear(&Timer::Retry);
                self.sched.send(Call::Me, Priority::Now);
            }
            (Phase::SignIn(_), Action::SignIn) => return vec![Effect::StartSignIn],
            (Phase::SignIn(step), Action::StartOver) => *step = Step::Ready(None),
            (Phase::Ready(session), Action::Refresh) if !session.threads.is_empty() => {
                let thread = session.threads.last().expect("a thread is open");
                let focus = thread.focus().clone();
                self.timers.clear(&Timer::Commit(focus.clone()));
                self.sched.send(Call::Commit(focus.clone()), Priority::Now);
                for owner in thread.lists(&session.store) {
                    self.timers.clear(&Timer::Replies(owner.clone()));
                    self.timers.clear(&Timer::More(owner.clone()));
                    let priority = if owner == focus {
                        Priority::Now
                    } else {
                        Priority::Later
                    };
                    let call = Call::Replies {
                        id: owner,
                        cursor: None,
                    };
                    self.sched.send(call, priority);
                }
            }
            (Phase::Ready(_), Action::Commit(act)) => return self.on_commit(act, now),
            (Phase::Ready(session), action) if !session.threads.is_empty() => {
                if let Some(id) = session.on_thread(action, now) {
                    fetch_thread(&mut self.sched, &mut self.timers, id);
                }
            }
            (Phase::Ready(session), Action::Open) => {
                if let Some(id) = session
                    .log
                    .selected_id(session.log.active(), &session.store)
                    .cloned()
                {
                    session.threads.push(ThreadScreen::new(id.clone(), now));
                    fetch_thread(&mut self.sched, &mut self.timers, id);
                }
            }
            (Phase::Ready(session), Action::Refresh) => {
                let log = session.log.active();
                session.asked_to_refresh.insert(log);
                self.timers.clear(&Timer::Reload(log));
                self.timers.clear(&Timer::Older(log));
                self.sched.send(Call::Page(log), Priority::Now);
            }
            (Phase::Ready(session), action) => session.log.act(action, &session.store),
            (Phase::Splash | Phase::SignIn(_), _) => {}
        }
        Vec::new()
    }

    fn on_commit(&mut self, act: Act, now: Instant) -> Vec<Effect> {
        let Phase::Ready(session) = &mut self.phase else {
            return Vec::new();
        };
        let Some(id) = session.selected().cloned() else {
            return Vec::new();
        };
        let Some(Node::Live(row)) = session.store.node(&id) else {
            return Vec::new();
        };
        let author = row.commit.author.username.clone();
        let url = row.commit.html_url.clone();
        let (toast, effects) = match act {
            Act::Star => {
                session.store.star(&id);
                (None, Vec::new())
            }
            Act::Pastel => {
                session.store.give_pastel(&id);
                (None, Vec::new())
            }
            Act::Follow => (
                session
                    .store
                    .follow(&author)
                    .map(|on| followed(&author, on)),
                Vec::new(),
            ),
            Act::Copy => (Some(format!("copied {url}")), vec![Effect::Copy(url)]),
            Act::Browse => (
                Some("opened in the browser".to_owned()),
                vec![Effect::Open(url)],
            ),
        };
        if let Some(text) = toast {
            self.toast(text, now);
        }
        effects
    }

    fn arrived(&mut self, call: &Call, reply: Reply, now: Instant) {
        let Phase::Ready(session) = &mut self.phase else {
            if let Reply::Account(account) = reply {
                self.open(account, now);
            }
            return;
        };
        match call {
            Call::Commit(id) => self.timers.clear(&Timer::Commit(id.clone())),
            Call::Replies { id, cursor: None } => self.timers.clear(&Timer::Replies(id.clone())),
            Call::Replies {
                id,
                cursor: Some(_),
            } => self.timers.clear(&Timer::More(id.clone())),
            _ => {}
        }
        let Call::Page(log) = call else {
            let answered = match (call, &reply) {
                (Call::User(username), Reply::User { is_following }) => {
                    Some((username, *is_following))
                }
                _ => None,
            };
            let gave = match &reply {
                Reply::Pastel { commit, remaining } => Some(format!(
                    "gave a pastel to @{} · {remaining} left today",
                    commit.card.commit.author.username
                )),
                _ => None,
            };
            session.store.apply(call, reply);
            let toast = gave.or_else(|| {
                let (username, server) = answered?;
                let want = session.store.following(username)?;
                (want != server).then(|| followed(username, want))
            });
            if let Some(text) = toast {
                self.toast(text, now);
            }
            return;
        };
        let refreshed = session.asked_to_refresh.remove(log);
        let selected = session.log.selected_id(*log, &session.store).cloned();
        session.store.apply(call, reply);
        session
            .log
            .reselect(*log, selected.as_ref(), &session.store);
        if refreshed {
            self.toast(format!("{} refreshed", log.slug()), now);
        }
    }

    fn open(&mut self, account: Account, now: Instant) {
        self.phase = Phase::Ready(Box::new(Session {
            store: Store::new(account),
            log: LogScreen::default(),
            threads: Vec::new(),
            help: false,
            asked_to_refresh: HashSet::new(),
        }));
        for log in LOGS {
            self.sched.send(Call::Page(log), Priority::Now);
        }
        self.timers.clear(&Timer::Retry);
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
                let offline = matches!(failure, Failure::Offline(_));
                let connection = match call {
                    Call::Page(log) => {
                        session.asked_to_refresh.remove(&log);
                        session.store.fail(log, failure.clone());
                        self.timers.set(Timer::Reload(log), now + RETRY);
                        offline
                    }
                    Call::Older { log, .. } => {
                        self.timers.set(Timer::Older(log), now + RETRY);
                        false
                    }
                    Call::Commit(id) => {
                        if error.is_not_found() {
                            session.store.missing(&id);
                        } else {
                            session.store.fail_commit(&id, failure.clone());
                            self.timers.set(Timer::Commit(id), now + RETRY);
                        }
                        false
                    }
                    Call::Replies { id, cursor } => {
                        let more = cursor.is_some();
                        if error.is_not_found() {
                            session.store.no_replies(&id, more);
                        } else {
                            session.store.fail_replies(&id, more, failure.clone());
                            let timer = if more {
                                Timer::More(id)
                            } else {
                                Timer::Replies(id)
                            };
                            self.timers.set(timer, now + RETRY);
                        }
                        false
                    }
                    Call::Star { .. } | Call::Follow { .. } | Call::User(_)
                        if !matches!(failure, Failure::Refused(_)) =>
                    {
                        true
                    }
                    Call::Star { .. }
                    | Call::Follow { .. }
                    | Call::User(_)
                    | Call::Delete(_)
                    | Call::Pastel(_)
                    | Call::Report { .. } => {
                        session.store.refused(&call);
                        let text = match (&error, &call) {
                            (Error::WrongShape(_), Call::Star { id, .. }) => {
                                format!("fatal: could not star {}", id.short_hash())
                            }
                            (
                                Error::WrongShape(_),
                                Call::Follow { username, .. } | Call::User(username),
                            ) => format!("fatal: could not follow @{username}"),
                            _ => error.to_string(),
                        };
                        self.toast = Some(text);
                        self.timers.set(Timer::Toast, now + TOAST);
                        offline
                    }
                    Call::Me | Call::Redeem { .. } | Call::Since { .. } => true,
                };
                if connection {
                    self.health = Some(failure);
                }
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
        let room =
            usize::from(frame.area().width).saturating_sub(HINT_INDENT.len() + SPINNER_CELLS);
        let mut hints = keys::hints(self.scopes(), self.keymap.pending(), &|action| {
            can(&self.phase, self.health.as_ref(), action)
        });
        if self.keymap.pending().is_empty() {
            hints = keys::fit(&hints, room);
        }
        let help = hints
            .split("  ")
            .find(|hint| hint.starts_with("? "))
            .map(|hint| format!("  {hint}"))
            .unwrap_or_default();
        let bottom = match &self.toast {
            Some(toast) if self.keymap.pending().is_empty() => toast.clone(),
            _ => hints,
        };
        let next_call = self.next_call(now);
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
                frame.render_widget(session.log.top_bar(&session.store, top.width), top);
                let active = session.log.active();
                let loaded = match session.store.slot(active) {
                    Slot::Loaded(listing) => Some(listing),
                    Slot::Loading | Slot::Failed(_) => None,
                };
                let on_log = session.threads.is_empty();
                if let Some(listing) = loaded
                    && listing.new_commits > 0
                    && on_log
                {
                    frame.render_widget(pill(listing.new_commits), banner);
                }
                let retry = Retry {
                    page: self.timers.remaining(&Timer::Reload(active), now),
                    older: self.timers.remaining(&Timer::Older(active), now),
                };
                let health = self.health.as_ref().map(|failure| (failure, next_call));
                let failure = if on_log {
                    loaded.and_then(|listing| match &listing.failure {
                        Some(failure) => Some((failure, retry.page)),
                        None => health,
                    })
                } else {
                    session
                        .threads
                        .last()
                        .and_then(|thread| {
                            let focus = thread.focus();
                            let failure = session.store.thread(focus)?.failure.as_ref()?;
                            let wait = self.timers.remaining(&Timer::Commit(focus.clone()), now);
                            Some((failure, wait))
                        })
                        .or(health)
                };
                let error = failure
                    .filter(|_| self.toast.is_none() && self.keymap.pending().is_empty())
                    .and_then(|(failure, wait)| {
                        fatal::hint(failure, wait, room.saturating_sub(help.len()))
                    });
                let hint = match error {
                    Some(error) => Line::from(vec![
                        Span::styled(
                            format!("{HINT_INDENT}{error}"),
                            Style::new().fg(theme::DESTRUCTIVE),
                        ),
                        Span::styled(help, theme::muted()),
                    ]),
                    None => Line::styled(format!("{HINT_INDENT}{bottom}"), theme::muted()),
                };
                match session.threads.last_mut() {
                    Some(thread) => {
                        thread.draw(frame, inset(body), &session.store, &self.timers, now, wall)
                    }
                    None => session
                        .log
                        .draw(frame, inset(body), &session.store, retry, wall),
                }
                frame.render_widget(hint, foot);
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
            Some(failure) => lines.extend(fatal::screen(
                failure,
                self.timers.remaining(&Timer::Retry, now),
            )),
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

fn pill(count: u32) -> Line<'static> {
    let noun = if count == 1 { "commit" } else { "commits" };
    Line::styled(
        format!("↑ {count} new {noun} · R to load"),
        Style::new().fg(theme::PRIMARY),
    )
    .centered()
}

fn draw_help(frame: &mut Frame, session: &Session) {
    let allows = |action| session.screen_allows(action);
    let rows: Vec<String> = keys::help(session.screen_scopes(), &allows)
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
