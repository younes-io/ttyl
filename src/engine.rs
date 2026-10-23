use std::borrow::Cow;
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::api::{self, Account, Call, Error, Failure, Reply};
use crate::attach::{self, Look};
use crate::clock::{
    FRAME, GLOW, MINUTE, POLL, RETRY, SECOND, SPARK_FRAME, Spark, TOAST, Timer, Timers,
};
use crate::compose::{Composer, Prompt};
use crate::draft::{self, Drafts, Target};
use crate::fatal;
use crate::inbox::{self, GroupList, Inbox, Opens};
use crate::keys::{
    self, Act, Action, COMPOSER_SCOPES, CONFIRM_SCOPES, DETAILS_SCOPES, GATE_SCOPES, GROUP_SCOPES,
    HELP_SCOPES, INBOX_SCOPES, InputMode, Keymap, LOG_SCOPES, PEOPLE_SCOPES, PROFILE_PEOPLE_SCOPES,
    PROFILE_SCOPES, REPORT_SCOPES, RULES_SCOPES, Resolve, SECRET_SCOPES, SPLASH_SCOPES, Scope,
    THREAD_SCOPES, VOTE_SCOPES,
};
use crate::log::{self, LOGS, LogScreen, Retry};
use crate::model::{
    self, Commit, CommitId, Filter, Kind, Log, NotificationId, Reason, Row, Shown, Status, User,
};
use crate::popup;
use crate::profile::{self, Item, ProfileScreen, Tab};
use crate::saver;
use crate::sched::{CallKey, Priority, Scheduler, Ticket};
use crate::secret;
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
const PENDING: &str = "Your commit is pending review. Everyone will see it once it is approved.";
const EDITOR_FAILED: &str = "editor exited with an error. draft unchanged.";
const EDITOR_FAILED_SHORT: &str = "editor failed. draft unchanged.";
const NO_ANSWER: &str = "fatal: no answer from dev-null.sh. it may have gone through. check your log before sending again.";
const VANISHED: &str = "That commit is no longer available to reply to.";

pub enum Event {
    Key(KeyEvent),
    Paste(String),
    Reply(Ticket, Result<Reply, Error>),
    Tick,
    SignIn(io::Result<Wait>),
    Link(io::Result<String>),
    InputLost(io::Error),
    DraftFailed(io::Error),
    Edited(String),
    EditorFailed,
    /// What reading picked files found, and the text they came from if they were pasted.
    Read(Vec<(PathBuf, Look)>, Option<String>),
    /// The clipboard held nothing to attach, and why.
    Unclipped(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Run(Ticket, Call),
    StartSignIn,
    Quit,
    Abort(String),
    Copy(String),
    Open(String),
    Draft(draft::Write),
    Edit(String),
    /// Reads picked files to see whether they can go, and keeps the pasted text they came from.
    Read(Vec<PathBuf>, Option<String>),
    /// Takes an image, or the files or text, from the clipboard.
    Clip,
}

pub enum Phase {
    Splash,
    SignIn(Step),
    Ready(Box<Session>),
}

pub struct Session {
    pub store: Store,
    pub log: LogScreen,
    inbox: Inbox,
    stack: Vec<Screen>,
    overlay: Option<Overlay>,
    composer: Option<Composer>,
    asked_to_refresh: HashSet<Refresh>,
    /// Polls whose bars have filled in once.
    filled: HashSet<CommitId>,
}

/// A list the user asked to reload, answered with a toast once it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Refresh {
    Log(Log),
    Inbox(Filter),
}

/// What sits over the log, newest last. Back pops one.
enum Screen {
    Thread(Box<ThreadScreen>),
    Inbox,
    Group(Box<GroupList>),
    Profile(Box<ProfileScreen>),
}

enum Overlay {
    /// The key list, scrolled `top` rows down. Drawing sets `last`, the furthest it can scroll on
    /// this screen, so a short terminal can still reach every key.
    Help {
        top: usize,
        last: usize,
    },
    Delete(CommitId),
    Reason {
        id: CommitId,
        at: usize,
    },
    Details {
        id: CommitId,
        reason: Reason,
        text: String,
    },
    /// Picking an option in the poll inside `id`, which draws in place. The store holds the option.
    Vote {
        id: CommitId,
        options: usize,
    },
}

const SERVER_DETAILS_MAX: usize = 500;

impl Session {
    /// Where the selected card on screen was last drawn.
    fn card(&self) -> Option<Rect> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => thread.card(),
            Some(Screen::Profile(profile)) => profile.card(),
            Some(Screen::Inbox | Screen::Group(_)) => None,
            None => self.log.card(),
        }
    }

    fn thread(&self) -> Option<&ThreadScreen> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => Some(thread),
            Some(Screen::Inbox | Screen::Group(_) | Screen::Profile(_)) | None => None,
        }
    }

    /// The thread on screen, opened or peeked beside the log.
    fn shown_thread(&self, now: Instant) -> Option<&ThreadScreen> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => Some(thread),
            Some(Screen::Inbox | Screen::Group(_) | Screen::Profile(_)) => None,
            None => self.log.peek(&self.store, now),
        }
    }

    fn group(&self) -> Option<&GroupList> {
        match self.stack.last() {
            Some(Screen::Group(group)) => Some(group),
            _ => None,
        }
    }

    /// The open group list a reply belongs to, wherever it sits in the stack.
    fn group_mut(&mut self, id: &NotificationId) -> Option<&mut GroupList> {
        self.stack.iter_mut().rev().find_map(|screen| match screen {
            Screen::Group(group) if group.id() == id => Some(group.as_mut()),
            _ => None,
        })
    }

    fn profile(&self) -> Option<&ProfileScreen> {
        match self.stack.last() {
            Some(Screen::Profile(profile)) => Some(profile),
            _ => None,
        }
    }

    /// Every open profile of this person, wherever it sits in the stack.
    fn profiles_mut(&mut self, username: &str) -> impl Iterator<Item = &mut ProfileScreen> {
        self.stack
            .iter_mut()
            .filter_map(move |screen| match screen {
                Screen::Profile(profile) if profile.username() == username => {
                    Some(profile.as_mut())
                }
                _ => None,
            })
    }

    /// Whose profile p opens: the selected commit's author or the selected person, unless already shown.
    fn profile_target(&self) -> Option<String> {
        let username = match self.selected()? {
            Selected::Row(row) => row.commit.author.username.clone(),
            Selected::Note(commit) => commit.author.username.clone(),
            Selected::Person(user) => user.username.clone(),
        };
        (self.profile().map(ProfileScreen::username) != Some(&username)).then_some(username)
    }

    fn on_inbox(&self) -> bool {
        matches!(self.stack.last(), Some(Screen::Inbox))
    }

    fn screen_scopes(&self) -> &'static [Scope] {
        match self.stack.last() {
            None => LOG_SCOPES,
            Some(Screen::Thread(_)) => THREAD_SCOPES,
            Some(Screen::Inbox) => INBOX_SCOPES,
            Some(Screen::Group(group)) if group.people() => PEOPLE_SCOPES,
            Some(Screen::Group(_)) => GROUP_SCOPES,
            Some(Screen::Profile(profile)) if profile.people_tab() => PROFILE_PEOPLE_SCOPES,
            Some(Screen::Profile(_)) => PROFILE_SCOPES,
        }
    }

    fn screen_allows(&self, action: Action) -> bool {
        if let Action::Commit(act) = action {
            return self.allows(act);
        }
        if action == Action::Profile {
            return self.profile_target().is_some();
        }
        matches!(action, Action::Quit | Action::Help | Action::Saver)
            || (matches!(action, Action::Compose | Action::Inbox) && !self.on_inbox())
            || match self.stack.last() {
                Some(Screen::Thread(thread)) => thread.can(action, &self.store),
                Some(Screen::Inbox) => self.inbox.can(action, &self.store),
                Some(Screen::Group(group)) => group.can(action),
                Some(Screen::Profile(profile)) => profile.can(action),
                None => self.log.can(action, &self.store),
            }
    }

    fn selected(&self) -> Option<Selected<'_>> {
        let id = match self.stack.last() {
            Some(Screen::Thread(thread)) => thread.selected(),
            Some(Screen::Inbox) => {
                let group = self.inbox.selected()?;
                return Selected::of(&group.event, group.actors_count == 1);
            }
            Some(Screen::Group(group)) => return Selected::of(group.selected()?, true),
            Some(Screen::Profile(profile)) => match profile.selected()? {
                Item::Commit(id) => id,
                Item::Person(person) => return Some(Selected::Person(&person.user)),
            },
            None => self.log.selected_id(self.log.active(), &self.store)?,
        };
        match self.store.node(id)? {
            Node::Live(row) => Some(Selected::Row(row)),
            Node::Gone(_) => None,
        }
    }

    fn allows(&self, act: Act) -> bool {
        let me = &self.store.me.username;
        match self.selected() {
            Some(Selected::Row(row)) => {
                let own = row.commit.author.username == *me;
                row.deletion.is_none()
                    && match act {
                        Act::Star | Act::Follow => !own,
                        Act::Pastel => !own && self.store.me.pasteis_mode,
                        Act::Copy | Act::Browse => true,
                        Act::Delete => own,
                        Act::Report => !own,
                        Act::Reply | Act::Quote => true,
                        Act::Vote => {
                            !own && (row.media.poll.as_ref())
                                .is_some_and(|poll| !poll.is_closed && !poll.has_voted)
                        }
                        Act::Image(at) => at < row.media.images.len(),
                    }
            }
            // A notification's commit carries no star state, so its star and pastel wait for the thread.
            Some(Selected::Note(commit)) => match act {
                Act::Follow => commit.author.username != *me,
                Act::Copy | Act::Browse | Act::Reply | Act::Quote => true,
                Act::Star | Act::Pastel | Act::Delete | Act::Report | Act::Vote | Act::Image(_) => {
                    false
                }
            },
            Some(Selected::Person(user)) => act == Act::Follow && user.username != *me,
            None => false,
        }
    }

    fn on_screen(&mut self, action: Action, now: Instant) -> Option<CommitId> {
        match (self.stack.last_mut(), action) {
            (_, Action::Back) => {
                self.stack.pop();
            }
            (_, Action::Show(_) | Action::NextLog) => {
                self.stack.clear();
                self.log.act(action, &self.store);
            }
            (Some(Screen::Thread(thread)), Action::Open) => {
                if let Some(id) = thread.opens(&self.store) {
                    self.stack
                        .push(Screen::Thread(Box::new(ThreadScreen::new(id.clone(), now))));
                    return Some(id);
                }
            }
            (Some(Screen::Thread(thread)), action) => thread.act(action, &self.store),
            (Some(Screen::Inbox), action) => self.inbox.act(action, &self.store),
            (Some(Screen::Group(group)), action) => group.act(action),
            (Some(Screen::Profile(profile)), action) => {
                profile.act(action, now);
            }
            (None, _) => {}
        }
        None
    }

    fn can(&self, action: Action) -> bool {
        if action == Action::Poll {
            return self.overlay.is_none()
                && self.composer.as_ref().is_some_and(Composer::can_poll);
        }
        if let Some(Overlay::Help { last, .. }) = self.overlay {
            return match action {
                Action::Down | Action::Up => last > 0,
                _ => true,
            };
        }
        self.overlay.is_some() || self.composer.is_some() || self.screen_allows(action)
    }

    fn scopes(&self) -> &'static [Scope] {
        match self.overlay {
            Some(Overlay::Help { .. }) => HELP_SCOPES,
            Some(Overlay::Delete(_)) => CONFIRM_SCOPES,
            Some(Overlay::Reason { .. }) => REPORT_SCOPES,
            Some(Overlay::Vote { .. }) => VOTE_SCOPES,
            Some(Overlay::Details { .. }) => DETAILS_SCOPES,
            None => match self.composer.as_ref().map(Composer::prompt) {
                Some(Some(Prompt::Secret)) => SECRET_SCOPES,
                Some(Some(Prompt::Rules)) => RULES_SCOPES,
                Some(None) => COMPOSER_SCOPES,
                None => self.screen_scopes(),
            },
        }
    }

    fn input_mode(&self) -> InputMode {
        match self.overlay {
            Some(Overlay::Delete(_) | Overlay::Details { .. }) => InputMode::Text,
            None if self
                .composer
                .as_ref()
                .is_some_and(|composer| composer.prompt() != Some(Prompt::Rules)) =>
            {
                InputMode::Text
            }
            Some(Overlay::Help { .. } | Overlay::Reason { .. } | Overlay::Vote { .. }) | None => {
                InputMode::Commands
            }
        }
    }

    fn pasted(&mut self, pasted: &str) {
        if let Some(Overlay::Details { text, .. }) = &mut self.overlay {
            let room = SERVER_DETAILS_MAX.saturating_sub(text.chars().count());
            let flat = pasted
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch });
            text.extend(flat.take(room));
        }
    }

    fn typed(&mut self, event: KeyEvent) {
        match &mut self.overlay {
            Some(Overlay::Delete(_)) => self.overlay = None,
            Some(Overlay::Details { text, .. }) => match event.code {
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(ch)
                    if !event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && text.chars().count() < SERVER_DETAILS_MAX =>
                {
                    text.push(ch);
                }
                _ => {}
            },
            Some(Overlay::Help { .. } | Overlay::Reason { .. } | Overlay::Vote { .. }) | None => {}
        }
    }

    fn on_overlay(&mut self, action: Action) -> Option<Call> {
        let overlay = self.overlay.take()?;
        let last = Reason::ALL.len() - 1;
        let (next, call) = match (overlay, action) {
            (_, Action::Close) => (None, None),
            (Overlay::Help { top, last }, Action::Down) => (
                Some(Overlay::Help {
                    top: (top + 1).min(last),
                    last,
                }),
                None,
            ),
            (Overlay::Help { top, last }, Action::Up) => (
                Some(Overlay::Help {
                    top: top.saturating_sub(1),
                    last,
                }),
                None,
            ),
            (Overlay::Delete(id), Action::Confirm) => {
                self.store.start_delete(&id);
                (None, Some(Call::Delete(id)))
            }
            (Overlay::Reason { id, at }, Action::Down) => (
                Some(Overlay::Reason {
                    id,
                    at: (at + 1).min(last),
                }),
                None,
            ),
            (Overlay::Reason { id, at }, Action::Up) => (
                Some(Overlay::Reason {
                    id,
                    at: at.saturating_sub(1),
                }),
                None,
            ),
            (Overlay::Reason { id, at }, Action::Confirm) => (
                Some(Overlay::Details {
                    id,
                    reason: Reason::ALL[at],
                    text: String::new(),
                }),
                None,
            ),
            (Overlay::Reason { id, .. }, Action::Pick(reason)) => (
                Some(Overlay::Details {
                    id,
                    reason,
                    text: String::new(),
                }),
                None,
            ),
            (Overlay::Vote { id, options }, Action::Down | Action::Up | Action::Choose(_)) => {
                let at = self.store.picking().unwrap_or(0);
                let at = match action {
                    Action::Down => (at + 1).min(options.saturating_sub(1)),
                    Action::Up => at.saturating_sub(1),
                    Action::Choose(chosen) if chosen < options => chosen,
                    _ => at,
                };
                self.store.pick(Some((id.clone(), at)));
                (Some(Overlay::Vote { id, options }), None)
            }
            (Overlay::Vote { id, .. }, Action::Confirm) => {
                let at = self.store.picking().unwrap_or(0);
                let poll = self.store.row(&id).and_then(|row| row.media.poll.as_ref());
                let option = poll
                    .and_then(|poll| poll.options.get(at))
                    .map(|option| option.id.clone());
                (None, option.map(|option| Call::Vote { id, option }))
            }
            (Overlay::Details { id, reason, text }, Action::Confirm) => {
                let details = text.trim();
                let details = (!details.is_empty()).then(|| details.to_owned());
                (
                    None,
                    Some(Call::Report {
                        id,
                        reason,
                        details,
                    }),
                )
            }
            (overlay, _) => (Some(overlay), None),
        };
        if !matches!(next, Some(Overlay::Vote { .. })) && self.store.picking().is_some() {
            self.store.pick(None);
        }
        self.overlay = next;
        call
    }

    fn prompt(&self, room: usize) -> Option<String> {
        match &self.overlay {
            Some(Overlay::Delete(id)) => {
                let short = id.short_hash();
                let warned = format!("delete {short}? this can't be undone [y/N]");
                Some(if warned.len() <= room {
                    warned
                } else {
                    format!("delete {short}? [y/N]")
                })
            }
            Some(
                Overlay::Help { .. }
                | Overlay::Reason { .. }
                | Overlay::Vote { .. }
                | Overlay::Details { .. },
            )
            | None => None,
        }
    }
}

fn post(sched: &mut Scheduler, composer: &Composer) {
    let call = Call::Post {
        target: composer.target().clone(),
        body: composer.text().to_owned(),
        attachment: composer.attachment(),
    };
    sched.send(call, Priority::Now);
}

fn fetch_thread(sched: &mut Scheduler, timers: &mut Timers, id: CommitId) {
    timers.clear(&Timer::Commit(id.clone()));
    timers.clear(&Timer::Replies(id.clone()));
    timers.clear(&Timer::More(id.clone()));
    sched.send(Call::Commit(id.clone()), Priority::Now);
    sched.send(Call::Replies { id, cursor: None }, Priority::Now);
}

/// Reloads the thread and every reply list it shows, so replies to replies come in too.
fn refresh_thread(
    sched: &mut Scheduler,
    timers: &mut Timers,
    thread: &ThreadScreen,
    store: &Store,
) {
    let focus = thread.focus().clone();
    timers.clear(&Timer::Commit(focus.clone()));
    sched.send(Call::Commit(focus.clone()), Priority::Now);
    for owner in thread.lists(store) {
        timers.clear(&Timer::Replies(owner.clone()));
        timers.clear(&Timer::More(owner.clone()));
        let priority = if owner == focus {
            Priority::Now
        } else {
            Priority::Later
        };
        sched.send(
            Call::Replies {
                id: owner,
                cursor: None,
            },
            priority,
        );
    }
}

fn fetch_inbox(sched: &mut Scheduler, timers: &mut Timers, inbox: &mut Inbox, now: Instant) {
    let filter = inbox.filter();
    inbox.asked(now);
    timers.clear(&Timer::Inbox(filter));
    sched.send(
        Call::Notifications {
            filter,
            cursor: None,
        },
        Priority::Now,
    );
}

fn open_profile(sched: &mut Scheduler, session: &mut Session, username: String, now: Instant) {
    let profile = ProfileScreen::new(username, now);
    sched.send(Call::User(profile.username().to_owned()), Priority::Now);
    sched.send(profile.tab_call(), Priority::Now);
    session.stack.push(Screen::Profile(Box::new(profile)));
}

fn fetch_group(sched: &mut Scheduler, group: &GroupList) {
    let call = Call::Group {
        id: group.id().clone(),
        filter: group.filter(),
        cursor: None,
    };
    sched.send(call, Priority::Now);
}

/// What the commit keys act on: a log or thread row, a notification's commit, or a person who followed.
enum Selected<'a> {
    Row(Box<Cow<'a, Row>>),
    Note(&'a Commit),
    Person(&'a User),
}

impl<'a> Selected<'a> {
    fn of(event: &'a model::Event, alone: bool) -> Option<Self> {
        match (&event.kind, &event.commit) {
            (Kind::Followed, _) => alone.then_some(Selected::Person(&event.actor)),
            (_, Some(Shown::Live(commit))) => Some(Selected::Note(commit)),
            _ => None,
        }
    }
}

/// Puts the composer up, and reads the images a draft brought back, which may have changed since.
fn opened(session: &mut Session, composer: Composer) -> Vec<Effect> {
    let unread = composer.unread();
    session.composer = Some(composer);
    if unread.is_empty() {
        Vec::new()
    } else {
        vec![Effect::Read(unread, None)]
    }
}

fn followed(username: &str, on: bool) -> String {
    let done = if on { "following" } else { "unfollowed" };
    format!("{done} @{username} · f to undo")
}

fn can(phase: &Phase, health: Option<&Failure>, sending: bool, action: Action) -> bool {
    match (phase, action) {
        (_, Action::Quit) => true,
        (_, Action::Send | Action::Editor | Action::Poll | Action::Attach) if sending => false,
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
    drafts: Drafts,
    saver: Option<saver::Saver>,
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
            drafts: Drafts::default(),
            saver: None,
        }
    }

    pub fn with_drafts(self, drafts: Drafts) -> Self {
        Engine { drafts, ..self }
    }

    pub fn draft(&self, target: &Target) -> Option<&str> {
        self.drafts.get(target).map(|draft| draft.text.as_str())
    }

    pub fn log(&self) -> Option<&LogScreen> {
        match &self.phase {
            Phase::Ready(session) => Some(&session.log),
            Phase::Splash | Phase::SignIn(_) => None,
        }
    }

    pub fn thread(&self) -> Option<&ThreadScreen> {
        match &self.phase {
            Phase::Ready(session) => session.thread(),
            Phase::Splash | Phase::SignIn(_) => None,
        }
    }

    pub fn update(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        let effects = self.handle(event, now);
        self.fill_poll(now);
        effects
    }

    /// The first time the cursor lands on a poll whose results show, its bars fill in.
    fn fill_poll(&mut self, now: Instant) {
        let Phase::Ready(session) = &mut self.phase else {
            return;
        };
        let Some(Selected::Row(row)) = session.selected() else {
            return;
        };
        let shown = (row.media.poll.as_ref()).is_some_and(|poll| {
            poll.options
                .iter()
                .any(|option| option.percentage.is_some())
        });
        let id = row.commit.id.clone();
        if shown && session.filled.insert(id.clone()) {
            self.timers
                .set(Timer::Spark(Spark::Fill, id), now + Spark::Fill.lasts());
        }
    }

    fn handle(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        match event {
            Event::Key(key) => self.key(key, now),
            Event::Paste(text) => {
                if let Phase::Ready(session) = &mut self.phase
                    && session.composer.is_none()
                {
                    session.pasted(&text);
                }
                // A file dropped on the terminal arrives as a pasted path.
                if let Some(paths) = attach::dropped(&text)
                    && self.composing()
                {
                    return vec![Effect::Read(paths, Some(text))];
                }
                self.edit(|composer| composer.paste(&text))
            }
            Event::Reply(ticket, result) => {
                let Some((call, priority)) = self.sched.finish(ticket) else {
                    return Vec::new();
                };
                match result {
                    Ok(reply) => {
                        self.health = None;
                        return self.arrived(&call, reply, now);
                    }
                    Err(error) => self.failed(call, priority, error, now),
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
            Event::DraftFailed(error) => {
                self.toast(format!("fatal: could not save your draft: {error}"), now);
                Vec::new()
            }
            Event::Edited(text) => self.edit(|composer| composer.replace(&text)),
            Event::Read(looks, pasted) => {
                self.edit(|composer| composer.read(looks, pasted.as_deref()))
            }
            Event::Unclipped(why) => {
                if let Phase::Ready(session) = &mut self.phase
                    && let Some(composer) = &mut session.composer
                {
                    composer.unattached(&why);
                }
                Vec::new()
            }
            Event::EditorFailed => {
                if let Phase::Ready(session) = &mut self.phase
                    && let Some(composer) = &mut session.composer
                {
                    composer.note_or(
                        EDITOR_FAILED,
                        EDITOR_FAILED_SHORT,
                        Style::new().fg(theme::DESTRUCTIVE),
                    );
                }
                Vec::new()
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
                Timer::Glow | Timer::Spark(..) => {}
                Timer::Reload(log) => self.sched.send(Call::Page(log), Priority::Later),
                Timer::Inbox(filter) => {
                    if let Phase::Ready(session) = &mut self.phase
                        && session.on_inbox()
                        && session.inbox.filter() == filter
                    {
                        fetch_inbox(&mut self.sched, &mut self.timers, &mut session.inbox, now);
                    }
                }
                Timer::Older(_)
                | Timer::Commit(_)
                | Timer::Replies(_)
                | Timer::More(_)
                | Timer::OlderInbox(_) => {}
            }
        }
        self.load_older();
        self.load_older_inbox();
        self.load_older_group();
        self.load_older_profile();
        for (call, priority) in self.thread_wants(now) {
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
            (!self.thread_wants(now).is_empty()).then_some(now),
            self.placeholder_at().filter(|at| *at > now),
            self.saver.as_ref().map(|_| now + saver::STEP),
            self.timers
                .any(|timer| matches!(timer, Timer::Spark(..)))
                .then(|| now + SPARK_FRAME),
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

    fn load_older_inbox(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        if !session.on_inbox() {
            return;
        }
        if let Some((filter, cursor)) = session.inbox.wants_older()
            && !self.timers.is_set(&Timer::OlderInbox(filter))
        {
            let cursor = Some(cursor);
            self.sched
                .send(Call::Notifications { filter, cursor }, Priority::Now);
        }
    }

    fn load_older_group(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        if let Some(group) = session.group()
            && let Some(cursor) = group.wants_older()
        {
            let call = Call::Group {
                id: group.id().clone(),
                filter: group.filter(),
                cursor: Some(cursor),
            };
            self.sched.send(call, Priority::Now);
        }
    }

    fn load_older_profile(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        if let Some(call) = session.profile().and_then(ProfileScreen::wants_older) {
            self.sched.send(call, Priority::Now);
        }
    }

    fn thread_wants(&self, now: Instant) -> Vec<(Call, Priority)> {
        let Phase::Ready(session) = &self.phase else {
            return Vec::new();
        };
        let Some(thread) = session.shown_thread(now) else {
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
        match session.stack.last() {
            Some(Screen::Thread(thread)) => thread.placeholder_at(&session.store),
            Some(Screen::Inbox) => session.inbox.placeholder_at(),
            Some(Screen::Group(group)) => group.placeholder_at(),
            Some(Screen::Profile(profile)) => profile.placeholder_at(),
            None => session.log.peek_due(),
        }
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
            || self.timers.any(|timer| {
                matches!(
                    timer,
                    Timer::Commit(_)
                        | Timer::Replies(_)
                        | Timer::More(_)
                        | Timer::Inbox(_)
                        | Timer::OlderInbox(_)
                )
            })
    }

    fn scopes(&self) -> &'static [Scope] {
        match &self.phase {
            Phase::Splash => SPLASH_SCOPES,
            Phase::SignIn(_) => GATE_SCOPES,
            Phase::Ready(session) => session.scopes(),
        }
    }

    fn input_mode(&self) -> InputMode {
        match &self.phase {
            Phase::Ready(session) => session.input_mode(),
            Phase::Splash | Phase::SignIn(_) => InputMode::Commands,
        }
    }

    fn sending(&self) -> bool {
        let Phase::Ready(session) = &self.phase else {
            return false;
        };
        session.composer.as_ref().is_some_and(|composer| {
            self.sched
                .pending(&CallKey::Post(composer.target().clone()))
        })
    }

    fn key(&mut self, event: KeyEvent, now: Instant) -> Vec<Effect> {
        if self.saver.take().is_some() {
            return Vec::new();
        }
        let scopes = self.scopes();
        let mode = self.input_mode();
        let sending = self.sending();
        let Engine {
            keymap,
            phase,
            health,
            ..
        } = self;
        let can = |action| can(phase, health.as_ref(), sending, action);
        match keymap.resolve(event, mode, scopes, &can) {
            Resolve::Act(action) => self.act(action, now),
            Resolve::Text(event) => {
                if let Phase::Ready(session) = &mut self.phase {
                    match &mut session.composer {
                        None => session.typed(event),
                        Some(composer) if composer.prompt() == Some(Prompt::Secret) => {
                            composer.answered();
                            return Vec::new();
                        }
                        Some(_) => {}
                    }
                }
                self.edit(|composer| composer.typed(event))
            }
            Resolve::Pending | Resolve::Ignored => Vec::new(),
        }
    }

    /// Whether a composer is open and can take changes.
    fn composing(&self) -> bool {
        !self.sending()
            && matches!(&self.phase, Phase::Ready(session) if session.composer.is_some())
    }

    fn edit(&mut self, edit: impl FnOnce(&mut Composer) -> bool) -> Vec<Effect> {
        if self.sending() {
            return Vec::new();
        }
        let Phase::Ready(session) = &mut self.phase else {
            return Vec::new();
        };
        let Some(composer) = &mut session.composer else {
            return Vec::new();
        };
        if !edit(composer) {
            return Vec::new();
        }
        self.drafts
            .write(composer.target(), &composer.draft())
            .map(Effect::Draft)
            .into_iter()
            .collect()
    }

    fn act(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match (&mut self.phase, action) {
            (_, Action::Quit) => return vec![Effect::Quit],
            (Phase::Ready(session), action) if session.overlay.is_some() => {
                if let Some(call) = session.on_overlay(action) {
                    self.sched.send(call, Priority::Now);
                }
            }
            (Phase::Ready(session), Action::Close) if session.composer.is_some() => {
                if session
                    .composer
                    .as_mut()
                    .is_some_and(|composer| !composer.dismiss())
                {
                    session.composer = None;
                }
            }
            (Phase::Ready(session), Action::Send) => {
                if let Some(composer) = &mut session.composer {
                    match composer.refusal() {
                        Some(refusal) => composer.note(refusal, theme::muted()),
                        None if secret::detected(composer.text()) => composer.ask(Prompt::Secret),
                        None => post(&mut self.sched, composer),
                    }
                }
            }
            (Phase::Ready(session), Action::Editor) => {
                if let Some(composer) = &session.composer {
                    return vec![Effect::Edit(composer.text().to_owned())];
                }
            }
            (Phase::Ready(_), Action::Poll) => return self.edit(Composer::toggle_poll),
            (Phase::Ready(session), Action::Attach) if session.composer.is_some() => {
                return vec![Effect::Clip];
            }
            (Phase::Ready(session), Action::Confirm) => {
                if let Some(composer) = &mut session.composer {
                    composer.answered();
                    post(&mut self.sched, composer);
                }
            }
            (Phase::Ready(session), Action::Back) if session.composer.is_some() => {
                if let Some(composer) = &mut session.composer {
                    composer.answered();
                }
            }
            (Phase::Ready(session), Action::Link(link)) => {
                if let Some(url) = session.profile().and_then(|profile| profile.link(link)) {
                    let shown = url
                        .trim_start_matches("https://")
                        .trim_start_matches("http://");
                    self.toast(
                        format!("opened {} in the browser", shown.trim_end_matches('/')),
                        now,
                    );
                    return vec![Effect::Open(url)];
                }
            }
            (Phase::Ready(_), Action::Site) => {
                self.toast("opened in the browser".to_owned(), now);
                return vec![Effect::Open(format!("{}/", api::BASE))];
            }
            (Phase::Ready(session), Action::Help) => {
                session.overlay = Some(Overlay::Help { top: 0, last: 0 });
            }
            (Phase::Ready(_), Action::Saver) => self.saver = Some(saver::Saver::new(now)),
            (Phase::Ready(session), Action::Compose) => {
                let draft = self.drafts.get(&Target::New);
                return opened(session, Composer::open(Target::New, None, draft));
            }
            (Phase::Splash, Action::Refresh) => {
                self.timers.clear(&Timer::Retry);
                self.sched.send(Call::Me, Priority::Now);
            }
            (Phase::SignIn(_), Action::SignIn) => return vec![Effect::StartSignIn],
            (Phase::SignIn(step), Action::StartOver) => *step = Step::Ready(None),
            (Phase::Ready(session), Action::Inbox) => {
                if !session.on_inbox() {
                    session.stack.push(Screen::Inbox);
                    fetch_inbox(&mut self.sched, &mut self.timers, &mut session.inbox, now);
                }
            }
            (
                Phase::Ready(session),
                Action::Refresh | Action::NextFilter | Action::PreviousFilter,
            ) if session.on_inbox() => {
                session.inbox.act(action, &session.store);
                if action == Action::Refresh {
                    session
                        .asked_to_refresh
                        .insert(Refresh::Inbox(session.inbox.filter()));
                }
                fetch_inbox(&mut self.sched, &mut self.timers, &mut session.inbox, now);
            }
            (Phase::Ready(session), Action::Open) if session.on_inbox() => {
                match session.inbox.opens() {
                    Some(Opens::Thread(id)) => {
                        session
                            .stack
                            .push(Screen::Thread(Box::new(ThreadScreen::new(id.clone(), now))));
                        fetch_thread(&mut self.sched, &mut self.timers, id);
                    }
                    Some(Opens::Group(head)) => {
                        let group = GroupList::new(*head, session.inbox.filter(), now);
                        fetch_group(&mut self.sched, &group);
                        session.stack.push(Screen::Group(Box::new(group)));
                    }
                    None => {}
                }
            }
            (Phase::Ready(session), Action::Profile) => {
                if let Some(username) = session.profile_target() {
                    open_profile(&mut self.sched, session, username, now);
                }
            }
            (Phase::Ready(session), Action::Open) if session.group().is_some() => {
                let opens = session.group().and_then(GroupList::opens);
                self.open_row(opens, now);
            }
            (Phase::Ready(session), Action::Open) if session.profile().is_some() => {
                let opens = session.profile().and_then(ProfileScreen::opens);
                self.open_row(opens, now);
            }
            (Phase::Ready(session), Action::Refresh) if session.profile().is_some() => {
                let Some(Screen::Profile(profile)) = session.stack.last_mut() else {
                    unreachable!("a profile is open");
                };
                profile.reload(now);
                self.sched
                    .send(Call::User(profile.username().to_owned()), Priority::Now);
                self.sched.send(profile.tab_call(), Priority::Now);
            }
            (Phase::Ready(session), Action::NextFilter | Action::PreviousFilter)
                if session.profile().is_some() =>
            {
                let Some(Screen::Profile(profile)) = session.stack.last_mut() else {
                    unreachable!("a profile is open");
                };
                if profile.act(action, now) {
                    self.sched.send(profile.tab_call(), Priority::Now);
                }
            }
            (Phase::Ready(session), Action::Refresh) if session.group().is_some() => {
                fetch_group(&mut self.sched, session.group().expect("a group is open"));
            }
            (Phase::Ready(session), Action::Refresh) if session.thread().is_some() => {
                let thread = session.thread().expect("a thread is open");
                refresh_thread(&mut self.sched, &mut self.timers, thread, &session.store);
            }
            (Phase::Ready(_), Action::Commit(act)) => return self.on_commit(act, now),
            (Phase::Ready(session), action) if !session.stack.is_empty() => {
                if let Some(id) = session.on_screen(action, now) {
                    fetch_thread(&mut self.sched, &mut self.timers, id);
                }
            }
            (Phase::Ready(session), Action::Open) => {
                if let Some(id) = session
                    .log
                    .selected_id(session.log.active(), &session.store)
                    .cloned()
                {
                    session
                        .stack
                        .push(Screen::Thread(Box::new(ThreadScreen::new(id.clone(), now))));
                    fetch_thread(&mut self.sched, &mut self.timers, id);
                }
            }
            (Phase::Ready(session), Action::Refresh) => {
                let log = session.log.active();
                session.asked_to_refresh.insert(Refresh::Log(log));
                self.timers.clear(&Timer::Reload(log));
                self.timers.clear(&Timer::Older(log));
                self.sched.send(Call::Page(log), Priority::Now);
                if let Some(peek) = session.log.peek(&session.store, now) {
                    refresh_thread(&mut self.sched, &mut self.timers, peek, &session.store);
                }
            }
            (Phase::Ready(session), action) => session.log.act(action, &session.store),
            (Phase::Splash | Phase::SignIn(_), _) => {}
        }
        Vec::new()
    }

    /// Pushes what a group or profile row opens: a thread, or someone's profile.
    fn open_row(&mut self, opens: Option<profile::Opens>, now: Instant) {
        let Phase::Ready(session) = &mut self.phase else {
            return;
        };
        match opens {
            Some(profile::Opens::Thread(id)) => {
                session
                    .stack
                    .push(Screen::Thread(Box::new(ThreadScreen::new(id.clone(), now))));
                fetch_thread(&mut self.sched, &mut self.timers, id);
            }
            Some(profile::Opens::Profile(username)) => {
                open_profile(&mut self.sched, session, username, now);
            }
            None => {}
        }
    }

    fn on_commit(&mut self, act: Act, now: Instant) -> Vec<Effect> {
        let Phase::Ready(session) = &mut self.phase else {
            return Vec::new();
        };
        let commit = match session.selected() {
            Some(Selected::Row(row)) => row.commit.clone(),
            Some(Selected::Note(commit)) => commit.clone(),
            Some(Selected::Person(user)) => {
                let author = user.username.clone();
                if let Some(on) = session.store.follow(&author) {
                    self.toast(followed(&author, on), now);
                }
                return Vec::new();
            }
            None => return Vec::new(),
        };
        let (id, author, url) = (commit.id, commit.author.username, commit.html_url);
        let (toast, effects) = match act {
            Act::Star => {
                let spark = Timer::Spark(Spark::Star, id.clone());
                if session.store.star(&id) == Some(true) {
                    self.timers.set(spark, now + Spark::Star.lasts());
                } else {
                    self.timers.clear(&spark);
                }
                (None, Vec::new())
            }
            Act::Pastel => {
                session.store.give_pastel(&id);
                self.timers.set(
                    Timer::Spark(Spark::Pastel, id.clone()),
                    now + Spark::Pastel.lasts(),
                );
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
            Act::Image(at) => {
                match (session.store.row(&id)).and_then(|row| row.media.images.get(at)) {
                    Some(image) => (
                        Some(format!("opened image {} in the browser", at + 1)),
                        vec![Effect::Open(image.url.clone())],
                    ),
                    None => (None, Vec::new()),
                }
            }
            Act::Delete => {
                session.overlay = Some(Overlay::Delete(id));
                (None, Vec::new())
            }
            Act::Report => {
                session.overlay = Some(Overlay::Reason { id, at: 0 });
                (None, Vec::new())
            }
            Act::Vote => {
                if let Some(poll) = (session.store.row(&id)).and_then(|row| row.media.poll.as_ref())
                {
                    let options = poll.options.len();
                    session.store.pick(Some((id.clone(), 0)));
                    session.overlay = Some(Overlay::Vote { id, options });
                }
                (None, Vec::new())
            }
            Act::Reply | Act::Quote => {
                let target = match act {
                    Act::Reply => Target::Reply(id),
                    _ => Target::Quote(id),
                };
                let draft = self.drafts.get(&target);
                (
                    None,
                    opened(session, Composer::open(target, Some(&author), draft)),
                )
            }
        };
        if let Some(text) = toast {
            self.toast(text, now);
        }
        effects
    }

    fn arrived(&mut self, call: &Call, reply: Reply, now: Instant) -> Vec<Effect> {
        let Phase::Ready(session) = &mut self.phase else {
            if let Reply::Account(account) = reply {
                self.open(account, now);
            }
            return Vec::new();
        };
        if let (Call::Post { target, .. }, Reply::Commit(focus)) = (call, &reply) {
            let commit = &focus.card.commit;
            let toast = match commit.status {
                Status::Pending => PENDING.to_owned(),
                Status::Approved | Status::Rejected => {
                    let first = commit.body.lines().next().unwrap_or("");
                    format!("[log {}] {first}", commit.short_hash)
                }
            };
            if session
                .composer
                .as_ref()
                .is_some_and(|composer| composer.target() == target)
            {
                session.composer = None;
            }
            let selected = session.log.anchor(Log::Latest, &session.store);
            let following = session.log.anchor(Log::Following, &session.store);
            session.store.apply(call, reply);
            session
                .log
                .reselect(Log::Latest, selected.as_ref(), &session.store);
            session
                .log
                .reselect(Log::Following, following.as_ref(), &session.store);
            self.toast(toast, now);
            return self
                .drafts
                .discard(target)
                .map(Effect::Draft)
                .into_iter()
                .collect();
        }
        match call {
            Call::Commit(id) => self.timers.clear(&Timer::Commit(id.clone())),
            Call::Replies { id, cursor: None } => self.timers.clear(&Timer::Replies(id.clone())),
            Call::Replies {
                id,
                cursor: Some(_),
            } => self.timers.clear(&Timer::More(id.clone())),
            Call::Notifications { filter, cursor } => {
                if let Reply::Notifications(page) = reply {
                    match cursor {
                        None => {
                            self.timers.clear(&Timer::Inbox(*filter));
                            session.store.unread = 0;
                            session.inbox.land(page);
                            if session.asked_to_refresh.remove(&Refresh::Inbox(*filter)) {
                                self.toast(format!("{} refreshed", inbox::TITLE), now);
                            }
                        }
                        Some(cursor) => {
                            self.timers.clear(&Timer::OlderInbox(*filter));
                            session.inbox.append(cursor, page);
                        }
                    }
                }
                return Vec::new();
            }
            Call::Group { id, cursor, .. } => {
                if let Reply::Group(page) = reply
                    && let Some(group) = session.group_mut(id)
                {
                    group.land(cursor.as_deref(), page);
                }
                return Vec::new();
            }
            Call::Commits { .. } | Call::People { .. } => {
                let Some((username, tab, cursor)) = Tab::of(call) else {
                    return Vec::new();
                };
                let (items, next): (Vec<Item>, _) = match reply {
                    Reply::Rows(page) => {
                        let ids = session.store.absorb_rows(page.rows);
                        (
                            ids.into_iter().map(Item::Commit).collect(),
                            page.next_cursor,
                        )
                    }
                    Reply::People(page) => {
                        let next = page.next_cursor.clone();
                        let people = page.people.clone();
                        session.store.apply(call, Reply::People(page));
                        let items = people
                            .into_iter()
                            .map(|person| Item::Person(Box::new(person)))
                            .collect();
                        (items, next)
                    }
                    _ => return Vec::new(),
                };
                for profile in session.profiles_mut(username) {
                    profile.land(tab, cursor, items.clone(), next.clone());
                }
                return Vec::new();
            }
            Call::User(username) => {
                if let Reply::User(profile) = &reply {
                    for screen in session.profiles_mut(username) {
                        screen.set_profile(profile.as_ref().clone());
                    }
                }
            }
            _ => {}
        }
        let Call::Page(log) = call else {
            let answered = match (call, &reply) {
                (Call::User(username), Reply::User(profile)) => {
                    Some((username, profile.is_following))
                }
                _ => None,
            };
            let said = match (call, &reply) {
                (_, Reply::Pastel { commit, remaining }) => Some(format!(
                    "gave a pastel to @{} · {remaining} left today",
                    commit.card.commit.author.username
                )),
                (Call::Delete(id), Reply::Done) => Some(format!("deleted {}", id.short_hash())),
                (Call::Report { id, .. }, Reply::Done) => {
                    Some(format!("reported {}. thanks.", id.short_hash()))
                }
                (Call::Vote { option, .. }, Reply::Commit(focus)) => {
                    let poll = focus.card.media.poll.as_ref();
                    let chosen =
                        poll.and_then(|poll| poll.options.iter().find(|each| each.id == *option));
                    Some(match chosen {
                        Some(chosen) => format!("voted for {}", chosen.label),
                        None => "voted".to_owned(),
                    })
                }
                _ => None,
            };
            let unread = session.store.unread;
            session.store.apply(call, reply);
            // The results come out with the vote, so its bars fill in now.
            if let Call::Vote { id, .. } = call {
                session.filled.insert(id.clone());
                self.timers.set(
                    Timer::Spark(Spark::Fill, id.clone()),
                    now + Spark::Fill.lasts(),
                );
            }
            if session.store.unread > unread {
                self.timers.set(Timer::Glow, now + GLOW);
                // A new notification may be a reply in the thread on screen.
                if let Some(thread) = session.shown_thread(now) {
                    refresh_thread(&mut self.sched, &mut self.timers, thread, &session.store);
                }
            }
            let toast = said.or_else(|| {
                let (username, server) = answered?;
                let want = session.store.following(username)?;
                (want != server).then(|| followed(username, want))
            });
            if let Some(text) = toast {
                self.toast(text, now);
            }
            return Vec::new();
        };
        let refreshed = session.asked_to_refresh.remove(&Refresh::Log(*log));
        let selected = session.log.selected_id(*log, &session.store).cloned();
        session.store.apply(call, reply);
        session
            .log
            .reselect(*log, selected.as_ref(), &session.store);
        if refreshed {
            self.toast(format!("{} refreshed", log.slug()), now);
        }
        Vec::new()
    }

    fn open(&mut self, account: Account, now: Instant) {
        self.phase = Phase::Ready(Box::new(Session {
            store: Store::new(account),
            log: LogScreen::default(),
            inbox: Inbox::default(),
            stack: Vec::new(),
            overlay: None,
            composer: None,
            asked_to_refresh: HashSet::new(),
            filled: HashSet::new(),
        }));
        for log in LOGS {
            self.sched.send(Call::Page(log), Priority::Now);
        }
        self.timers.clear(&Timer::Retry);
        self.timers.set(Timer::Poll, now + POLL);
        self.timers.set(Timer::RelativeTimes, now + MINUTE);
    }

    fn failed(&mut self, call: Call, priority: Priority, error: Error, now: Instant) {
        if let Call::Post { target, .. } = &call
            && !error.needs_sign_in()
        {
            self.refused_post(target, &error, now);
            return;
        }
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
                if let Call::User(username) = &call {
                    for profile in session.profiles_mut(username) {
                        profile.fail_header(failure.clone());
                    }
                }
                let connection = match call {
                    Call::Page(log) => {
                        session.asked_to_refresh.remove(&Refresh::Log(log));
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
                    Call::Notifications {
                        filter,
                        cursor: None,
                    } => {
                        session.asked_to_refresh.remove(&Refresh::Inbox(filter));
                        session.inbox.fail(filter, failure.clone());
                        self.timers.set(Timer::Inbox(filter), now + RETRY);
                        offline
                    }
                    Call::Notifications { filter, .. } => {
                        self.timers.set(Timer::OlderInbox(filter), now + RETRY);
                        false
                    }
                    Call::Group { id, .. } => {
                        if let Some(group) = session.group_mut(&id) {
                            group.fail(failure.clone());
                        }
                        offline
                    }
                    Call::Commits { .. } | Call::People { .. } => {
                        if let Some((username, tab, _)) = Tab::of(&call) {
                            for profile in session.profiles_mut(username) {
                                profile.fail_tab(tab, failure.clone());
                            }
                        }
                        offline
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
                    | Call::Vote { .. }
                    | Call::Report { .. } => {
                        session.store.refused(&call);
                        match &call {
                            Call::Star { id, .. } => {
                                self.timers.clear(&Timer::Spark(Spark::Star, id.clone()))
                            }
                            Call::Pastel(id) => {
                                self.timers.clear(&Timer::Spark(Spark::Pastel, id.clone()))
                            }
                            _ => {}
                        }
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
                    Call::Post { .. } => false,
                };
                if connection {
                    self.health = Some(failure);
                }
            }
        }
    }

    fn refused_post(&mut self, target: &Target, error: &Error, now: Instant) {
        let rules = error.is("commit_rules_required");
        let text = if error.may_have_landed() {
            NO_ANSWER.to_owned()
        } else if error.is_not_found() && *target != Target::New {
            VANISHED.to_owned()
        } else {
            error.to_string()
        };
        if let Phase::Ready(session) = &mut self.phase
            && let Some(composer) = &mut session.composer
            && composer.target() == target
        {
            if rules {
                composer.ask(Prompt::Rules);
            } else {
                composer.note(text, Style::new().fg(theme::DESTRUCTIVE));
            }
            return;
        }
        self.toast(text, now);
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
        if let Some(saver) = &mut self.saver {
            return saver.draw(frame, now);
        }
        if let Phase::Ready(session) = &mut self.phase
            && let Some(Overlay::Help { top, .. }) = session.overlay
        {
            // Measured before the footer, which offers scrolling only when the list overflows.
            let screen = Rect {
                height: frame.area().height.saturating_sub(1),
                ..frame.area()
            };
            let (_, _, last) = help_layout(screen, session);
            session.overlay = Some(Overlay::Help {
                top: top.min(last),
                last,
            });
        }
        let room =
            usize::from(frame.area().width).saturating_sub(HINT_INDENT.len() + SPINNER_CELLS);
        let sending = self.sending();
        let mut hints = keys::hints(self.scopes(), self.keymap.pending(), &|action| {
            can(&self.phase, self.health.as_ref(), sending, action)
        });
        if self.keymap.pending().is_empty() {
            hints = keys::fit(&hints, room);
        }
        let help = hints
            .split("  ")
            .find(|hint| hint.starts_with("? "))
            .map(|hint| format!("  {hint}"))
            .unwrap_or_default();
        let overlaid = matches!(&self.phase, Phase::Ready(session) if session.overlay.is_some());
        let toast = match &self.toast {
            Some(toast) if self.keymap.pending().is_empty() && !overlaid => {
                Some(fatal::cut(toast, room))
            }
            _ => None,
        };
        let bottom = toast.clone().unwrap_or_else(|| hints.clone());
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
                let panel = session
                    .composer
                    .as_ref()
                    .map_or(0, |composer| composer.height(area.width, area.height));
                let [top, banner, body, panel, foot] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Fill(1),
                    Constraint::Length(panel),
                    Constraint::Length(1),
                ])
                .areas(area);
                let handle = session
                    .profile()
                    .map(|profile| format!("@{}", profile.username()));
                let title = (session.on_inbox() || session.group().is_some())
                    .then_some(inbox::TITLE)
                    .or(handle.as_deref());
                let glow = self.timers.is_set(&Timer::Glow) && !session.on_inbox();
                let bar = Block::new().style(Style::new().bg(theme::MUTED));
                frame.render_widget(bar.clone(), top);
                frame.render_widget(bar, foot);
                frame.render_widget(
                    session.log.top_bar(&session.store, title, glow, top.width),
                    top,
                );
                let active = session.log.active();
                let loaded = match session.store.slot(active) {
                    Slot::Loaded(listing) => Some(listing),
                    Slot::Loading | Slot::Failed(_) => None,
                };
                let on_log = session.stack.is_empty();
                if session.on_inbox() {
                    frame.render_widget(session.inbox.filter_bar(banner.width), banner);
                }
                if let Some(group) = session.group() {
                    frame.render_widget(group.banner(), banner);
                }
                if on_log {
                    let fresh = loaded.map_or(0, |listing| listing.new_commits);
                    let news = Style::new().fg(theme::PRIMARY);
                    if session.log.has_pane(inset(body), &session.store) {
                        if fresh > 0 {
                            frame.render_widget(
                                Line::styled(news_of(fresh), news).centered(),
                                banner,
                            );
                        }
                    } else {
                        let tail = if fresh > 0 {
                            vec![Span::styled(format!(" {} ─", news_of(fresh)), news)]
                        } else {
                            Vec::new()
                        };
                        let banner = inset(banner);
                        frame.render_widget(session.log.rule(banner.width, tail), banner);
                    }
                }
                let retry = Retry {
                    page: self.timers.remaining(&Timer::Reload(active), now),
                    older: self.timers.remaining(&Timer::Older(active), now),
                };
                let health = self.health.as_ref().map(|failure| (failure, next_call));
                let filter = session.inbox.filter();
                let failure = if session.on_inbox() {
                    session
                        .inbox
                        .failure()
                        .map(|failure| (failure, self.timers.remaining(&Timer::Inbox(filter), now)))
                        .or(health)
                } else if let Some(group) = session.group() {
                    group.failure().map(|failure| (failure, None)).or(health)
                } else if let Some(profile) = session.profile() {
                    profile.failure().map(|failure| (failure, None)).or(health)
                } else if on_log {
                    loaded.and_then(|listing| match &listing.failure {
                        Some(failure) => Some((failure, retry.page)),
                        None => health,
                    })
                } else {
                    session
                        .thread()
                        .and_then(|thread| {
                            let focus = thread.focus();
                            let failure = session.store.thread(focus)?.failure.as_ref()?;
                            let wait = self.timers.remaining(&Timer::Commit(focus.clone()), now);
                            Some((failure, wait))
                        })
                        .or(health)
                };
                let error = failure
                    .filter(|_| {
                        self.toast.is_none()
                            && self.keymap.pending().is_empty()
                            && session.composer.is_none()
                    })
                    .and_then(|(failure, wait)| {
                        fatal::hint(failure, wait, room.saturating_sub(help.len()))
                    });
                let hint = match (session.prompt(room), error) {
                    (Some(prompt), _) => Line::styled(
                        format!("{HINT_INDENT}{prompt}"),
                        Style::new().fg(theme::DESTRUCTIVE),
                    ),
                    (None, Some(error)) => Line::from(vec![
                        Span::styled(
                            format!("{HINT_INDENT}{error}"),
                            Style::new().fg(theme::DESTRUCTIVE),
                        ),
                        Span::styled(help, theme::muted()),
                    ]),
                    (None, None) => match &toast {
                        Some(toast) => {
                            Line::styled(format!("{HINT_INDENT}{toast}"), theme::muted())
                        }
                        None => keyed(&hints),
                    },
                };
                let beside_feed = session
                    .stack
                    .iter()
                    .all(|screen| matches!(screen, Screen::Thread(_)))
                    && session.log.has_pane(inset(body), &session.store);
                match session.stack.last_mut() {
                    Some(Screen::Thread(thread)) if beside_feed => {
                        let pane = session.log.draw(
                            frame,
                            inset(body),
                            &session.store,
                            &self.timers,
                            now,
                            wall,
                        );
                        if let Some(panes) = pane {
                            session.log.draw_box(frame, panes.feed, false);
                            let inner = log::draw_box(frame, panes.pane, "thread", true);
                            thread.draw(frame, inner, &session.store, &self.timers, now, wall);
                        }
                    }
                    Some(Screen::Thread(thread)) => {
                        thread.draw(frame, inset(body), &session.store, &self.timers, now, wall)
                    }
                    Some(Screen::Inbox) => {
                        let retry = Retry {
                            page: self.timers.remaining(&Timer::Inbox(filter), now),
                            older: self.timers.remaining(&Timer::OlderInbox(filter), now),
                        };
                        session.inbox.draw(frame, inset(body), retry, now, wall);
                    }
                    Some(Screen::Group(group)) => group.draw(frame, inset(body), now, wall),
                    Some(Screen::Profile(profile)) => {
                        profile.draw(frame, inset(body), &session.store, &self.timers, now, wall)
                    }
                    None => {
                        let pane = session.log.draw(
                            frame,
                            inset(body),
                            &session.store,
                            &self.timers,
                            now,
                            wall,
                        );
                        if let Some(panes) = pane {
                            session.log.draw_box(frame, panes.feed, true);
                            session.log.draw_peek(
                                frame,
                                panes.pane,
                                &session.store,
                                &self.timers,
                                now,
                                wall,
                            );
                        }
                    }
                }
                if let Some(composer) = &mut session.composer {
                    composer.draw(frame, panel, sending);
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
                let screen = Rect {
                    height: area.height.saturating_sub(1),
                    ..area
                };
                let card = session.card();
                match &session.overlay {
                    Some(Overlay::Help { top, .. }) => {
                        popup::dim(frame.buffer_mut(), screen, None);
                        draw_help(frame, screen, session, *top);
                    }
                    Some(Overlay::Reason { id, at }) => {
                        popup::dim(frame.buffer_mut(), screen, card);
                        draw_report(frame, screen, card, id, *at, None);
                    }
                    Some(Overlay::Details { id, reason, text }) => {
                        let at = Reason::ALL.iter().position(|each| each == reason);
                        popup::dim(frame.buffer_mut(), screen, card);
                        draw_report(frame, screen, card, id, at.unwrap_or(0), Some(text));
                    }
                    Some(Overlay::Delete(_) | Overlay::Vote { .. }) | None => {}
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
        let mut lines = vec![Line::from(theme::logo()), Line::default()];
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

fn news_of(count: u32) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("↑ {count} new {noun} · R to load")
}

/// The keys help lists, the box they go in, and the furthest row the list can scroll to.
fn help_layout(
    screen: Rect,
    session: &Session,
) -> (Vec<(String, &'static str, bool)>, Rect, usize) {
    let allows = |action| session.screen_allows(action);
    let rows = keys::help(session.screen_scopes(), &allows);
    let key_width = rows.iter().map(|(key, ..)| key.width()).max().unwrap_or(0);
    let width = rows
        .iter()
        .map(|(_, help, _)| key_width + 2 + help.width())
        .max()
        .unwrap_or(0)
        + 6;
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    let height = u16::try_from(rows.len() + 4).unwrap_or(u16::MAX);
    let area = popup::place(screen, width, height, None);
    let last = if area.height < height {
        rows.len()
            .saturating_sub(usize::from(area.height.saturating_sub(2)))
    } else {
        0
    };
    (rows, area, last)
}

/// Draws the keys from row `top`, which the caller has already held within reach.
fn draw_help(frame: &mut Frame, screen: Rect, session: &Session, top: usize) {
    let (rows, area, last) = help_layout(screen, session);
    let key_width = rows.iter().map(|(key, ..)| key.width()).max().unwrap_or(0);
    let block = Block::bordered().title(Span::styled(
        " keys ",
        Style::new()
            .fg(theme::FOREGROUND)
            .add_modifier(Modifier::BOLD),
    ));
    // A list too tall for the screen gives up its blank rows before it scrolls.
    let block = if last > 0 {
        block.padding(Padding::horizontal(2))
    } else {
        block.padding(Padding::new(2, 2, 1, 1))
    };
    let block = if top < last {
        block.title_bottom(Line::styled(" ↓ more ", theme::muted()).right_aligned())
    } else {
        block
    };
    let inner = popup::raise(frame, area, block);
    let lines: Vec<Line> = rows
        .into_iter()
        .skip(top)
        .map(|(key, help, live)| {
            let (key_style, help_style) = if live {
                (
                    Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    Style::new().fg(theme::FOREGROUND),
                )
            } else {
                (Style::new().fg(theme::FAINT), Style::new().fg(theme::FAINT))
            };
            Line::from(vec![
                Span::styled(format!("{key:>key_width$}"), key_style),
                Span::raw("  "),
                Span::styled(help, help_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Asks why a commit is being reported, beside the card it reports when there is room.
fn draw_report(
    frame: &mut Frame,
    screen: Rect,
    card: Option<Rect>,
    id: &CommitId,
    at: usize,
    details: Option<&str>,
) {
    let height = u16::try_from(Reason::ALL.len() + 7).unwrap_or(u16::MAX);
    let block = Block::bordered()
        .title(Span::styled(
            format!(" report {} ", id.short_hash()),
            Style::new()
                .fg(theme::FOREGROUND)
                .add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::uniform(1));
    let area = popup::place(screen, 50, height, card);
    let block = if area.height < height {
        block.padding(Padding::horizontal(1))
    } else {
        block
    };
    let inner = popup::raise(frame, area, block);
    let width = usize::from(inner.width);
    let picking = details.is_none();
    let mut lines: Vec<Line> = Reason::ALL
        .iter()
        .enumerate()
        .map(|(each, reason)| {
            let row = format!(" {}  {}", each + 1, reason.label());
            let row = format!("{row:<width$}");
            if each == at {
                Line::styled(row, popup::chosen())
            } else if picking {
                Line::raw(row)
            } else {
                Line::styled(row, theme::muted())
            }
        })
        .collect();
    if usize::from(inner.height) > Reason::ALL.len() + 2 {
        lines.push(Line::raw(""));
    }
    let text = details.unwrap_or("");
    let label = " details (optional)";
    let count = format!("{}/{SERVER_DETAILS_MAX} ", text.chars().count());
    let gap = width.saturating_sub(label.len() + count.len());
    lines.push(Line::styled(
        format!("{label}{}{count}", " ".repeat(gap)),
        theme::muted(),
    ));
    let room = width.saturating_sub(4);
    let shown: String = {
        let skip = text.chars().count().saturating_sub(room);
        text.chars().skip(skip).collect()
    };
    let field = format!(" > {shown}");
    let cursor = u16::try_from(field.chars().count()).unwrap_or(u16::MAX);
    let well = if picking {
        theme::muted()
    } else {
        Style::new().bg(theme::BACKGROUND).fg(theme::FOREGROUND)
    };
    lines.push(Line::styled(format!("{field:<width$}"), well));
    let excess = lines.len().saturating_sub(usize::from(inner.height));
    lines.drain(..excess);
    let row = u16::try_from(lines.len() - 1).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines), inner);
    if !picking && row < inner.height {
        frame.set_cursor_position((inner.x + cursor.min(inner.width), inner.y + row));
    }
}

/// Each hint's key stands out from what it does.
fn keyed(hints: &str) -> Line<'static> {
    let mut spans = vec![Span::raw(HINT_INDENT)];
    for (index, hint) in hints.split("  ").enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let (key, label) = hint.split_once(' ').unwrap_or((hint, ""));
        spans.push(Span::styled(
            key.to_owned(),
            Style::new().fg(theme::FOREGROUND),
        ));
        if !label.is_empty() {
            spans.push(Span::styled(format!(" {label}"), theme::muted()));
        }
    }
    Line::from(spans)
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
