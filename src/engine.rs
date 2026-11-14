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
use crate::cache::Start;
use crate::clock::{
    FADE, FOCUS_RELOAD, FRAME, GLOW, MINUTE, POLL, RETRY, SECOND, SPARK_FRAME, Spark, TOAST, Timer,
    Timers, UNDO,
};
use crate::compose::{Composer, Prompt};
use crate::draft::{self, Drafts, Target};
use crate::fatal;
use crate::inbox::{self, GroupList, Inbox, Opens};
use crate::keys::{
    self, Act, Action, COMPOSER_SCOPES, CONFIRM_SCOPES, DETAILS_SCOPES, GATE_SCOPES, GROUP_SCOPES,
    HELP_SCOPES, INBOX_SCOPES, InputMode, Keymap, LIST_PEOPLE_SCOPES, LIST_SCOPES, LOG_SCOPES,
    PEOPLE_SCOPES, PIPE_SCOPES, PREVIEW_SCOPES, PROFILE_PEOPLE_SCOPES, PROFILE_SCOPES,
    REPORT_SCOPES, RULES_SCOPES, Resolve, SEARCH_SCOPES, SECRET_SCOPES, SPLASH_SCOPES, Scope,
    THREAD_SCOPES, VIEWER_SCOPES, VOTE_SCOPES,
};
use crate::list::{ListScreen, Source};
use crate::log::{self, LOGS, LogScreen, Retry};
use crate::model::{
    self, Commit, CommitId, Filter, Kind, Log, NotificationId, Reason, Row, Shown, Status,
    Translation, Untranslated, User,
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
use crate::viewer::Gallery;
use crate::welcome::{self, Step, Wait};
use ratatui_image::picker::Picker;

pub const WORKERS: usize = 2;
pub const AHEAD: usize = 5;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_CELLS: usize = 2;
const HINT_INDENT: &str = " ";
pub const PENDING: &str =
    "Your commit is pending review. Everyone will see it once it is approved.";
const EDITOR_FAILED: &str = "editor exited with an error. draft unchanged.";
const EDITOR_FAILED_SHORT: &str = "editor failed. draft unchanged.";
pub const NO_ANSWER: &str = "fatal: no answer from dev-null.sh. it may have gone through. check your log before sending again.";
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
    /// An image fetched for the viewer, or why it couldn't be.
    Fetched(String, Result<image::DynamicImage, String>),
    /// The terminal came back into focus.
    Focused,
    /// A pipe to `command` ended, or why it failed.
    Piped(String, Result<(), String>),
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
    /// Fetches an image for the viewer.
    Fetch(String),
    /// Remembers whether the viewer picked plain symbols over Nerd Font icons.
    Icons(bool),
    /// Hands the terminal to `command` with `input` on its stdin.
    Pipe {
        command: String,
        input: String,
    },
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
    /// The shell command the last pipe ran, which the next one starts from.
    piped: String,
    /// What the last search asked for, which the next one starts from.
    searched: String,
    /// The commit deleted moments ago, which `u` can still bring back.
    undoable: Option<CommitId>,
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
    List(Box<ListScreen>),
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
    /// Looking at image `at` of the ones in `id`.
    Viewer {
        id: CommitId,
        at: usize,
    },
    /// Typing the shell command that a commit's text is piped to.
    Pipe {
        input: String,
        command: String,
    },
    /// Typing what to search for: words for commits, `@name` for people.
    Search {
        query: String,
    },
}

const SERVER_DETAILS_MAX: usize = 500;

impl Session {
    /// Where the selected card on screen was last drawn.
    fn card(&self) -> Option<Rect> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => thread.card(),
            Some(Screen::Profile(profile)) => profile.card(),
            Some(Screen::List(list)) => list.card(),
            Some(Screen::Inbox | Screen::Group(_)) => None,
            None => self.log.card(),
        }
    }

    fn thread(&self) -> Option<&ThreadScreen> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => Some(thread),
            Some(Screen::Inbox | Screen::Group(_) | Screen::Profile(_) | Screen::List(_))
            | None => None,
        }
    }

    /// The thread on screen, opened or peeked beside the log.
    fn shown_thread(&self, now: Instant) -> Option<&ThreadScreen> {
        match self.stack.last() {
            Some(Screen::Thread(thread)) => Some(thread),
            Some(Screen::Inbox | Screen::Group(_) | Screen::Profile(_) | Screen::List(_)) => None,
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
    fn profiles_mut(&mut self, handle: &str) -> impl Iterator<Item = &mut ProfileScreen> {
        self.stack
            .iter_mut()
            .filter_map(move |screen| match screen {
                Screen::Profile(profile) if profile.handle() == handle => Some(profile.as_mut()),
                _ => None,
            })
    }

    /// Whose profile p opens: the selected commit's author or the selected person, unless already shown.
    fn profile_target(&self) -> Option<String> {
        let handle = match self.selected()? {
            Selected::Row(row) => row.commit.author.handle.clone(),
            Selected::Note(commit) => commit.author.handle.clone(),
            Selected::Person(user) => user.handle.clone(),
        };
        (self.profile().map(ProfileScreen::handle) != Some(&handle)).then_some(handle)
    }

    fn on_inbox(&self) -> bool {
        matches!(self.stack.last(), Some(Screen::Inbox))
    }

    /// Every open list filled from `source`, wherever it sits in the stack.
    fn lists_mut<'a>(&'a mut self, source: &'a Source) -> impl Iterator<Item = &'a mut ListScreen> {
        self.stack
            .iter_mut()
            .filter_map(move |screen| match screen {
                Screen::List(list) if list.source() == source => Some(list.as_mut()),
                _ => None,
            })
    }

    fn list(&self) -> Option<&ListScreen> {
        match self.stack.last() {
            Some(Screen::List(list)) => Some(list),
            _ => None,
        }
    }

    fn on_starred(&self) -> bool {
        self.list()
            .is_some_and(|list| *list.source() == Source::Starred)
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
            Some(Screen::List(list)) if list.people() => LIST_PEOPLE_SCOPES,
            Some(Screen::List(_)) => LIST_SCOPES,
        }
    }

    fn screen_allows(&self, action: Action) -> bool {
        if let Action::Commit(act) = action {
            return self.allows(act);
        }
        if action == Action::Profile {
            return self.profile_target().is_some();
        }
        matches!(
            action,
            Action::Quit | Action::Help | Action::Saver | Action::Search
        ) || (action == Action::Undo && self.undoable.is_some())
            || (matches!(action, Action::Compose | Action::Inbox) && !self.on_inbox())
            || (action == Action::Starred && !self.on_starred())
            || match self.stack.last() {
                Some(Screen::Thread(thread)) => thread.can(action, &self.store),
                Some(Screen::Inbox) => self.inbox.can(action, &self.store),
                Some(Screen::Group(group)) => group.can(action),
                Some(Screen::Profile(profile)) => profile.can(action),
                Some(Screen::List(list)) => list.can(action),
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
            Some(Screen::List(list)) => match list.selected()? {
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
        let me = &self.store.me.handle;
        match self.selected() {
            Some(Selected::Row(row)) => {
                let own = row.commit.author.handle == *me;
                row.deletion.is_none()
                    && match act {
                        Act::Star | Act::Follow | Act::Mute => !own,
                        Act::Pastel => !own && self.store.me.pasteis_mode,
                        Act::Copy | Act::Browse | Act::Pipe => true,
                        Act::Delete => own,
                        Act::Report => !own,
                        Act::Reply | Act::Quote => true,
                        Act::Vote => {
                            !own && (row.media.poll.as_ref())
                                .is_some_and(|poll| !poll.is_closed && !poll.has_voted)
                        }
                        Act::Images => !row.media.images.is_empty(),
                        Act::OpenQuote => matches!(row.quoted_commit, Some(Shown::Live(_))),
                        Act::Translate => !row.commit.body.trim().is_empty(),
                    }
            }
            // A notification's commit carries no star state, so its star and pastel wait for the thread.
            Some(Selected::Note(commit)) => match act {
                Act::Follow | Act::Mute => commit.author.handle != *me,
                Act::Copy | Act::Browse | Act::Reply | Act::Quote | Act::Pipe => true,
                Act::Star
                | Act::Pastel
                | Act::Delete
                | Act::Report
                | Act::Vote
                | Act::Images
                | Act::OpenQuote
                | Act::Translate => false,
            },
            Some(Selected::Person(user)) => act == Act::Follow && user.handle != *me,
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
            (Some(Screen::List(list)), action) => list.act(action),
            (Some(Screen::Inbox), action) => self.inbox.act(action, &self.store),
            (Some(Screen::Group(group)), action) => group.act(action),
            (Some(Screen::Profile(profile)), action) => {
                profile.act(action, now);
            }
            (None, _) => {}
        }
        None
    }

    fn images(&self, id: &CommitId) -> &[model::Image] {
        (self.store.row(id)).map_or(&[], |row| row.media.images.as_slice())
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
        if let Some(Overlay::Viewer { id, .. }) = &self.overlay {
            return match action {
                Action::Next | Action::Previous => self.images(id).len() > 1,
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
            Some(Overlay::Viewer { .. }) => VIEWER_SCOPES,
            Some(Overlay::Details { .. }) => DETAILS_SCOPES,
            Some(Overlay::Pipe { .. }) => PIPE_SCOPES,
            Some(Overlay::Search { .. }) => SEARCH_SCOPES,
            None => match self.composer.as_ref().map(Composer::prompt) {
                Some(Some(Prompt::Secret)) => SECRET_SCOPES,
                Some(Some(Prompt::Rules)) => RULES_SCOPES,
                Some(None) if self.composer.as_ref().is_some_and(Composer::previewing) => {
                    PREVIEW_SCOPES
                }
                Some(None) => COMPOSER_SCOPES,
                None => self.screen_scopes(),
            },
        }
    }

    fn input_mode(&self) -> InputMode {
        match self.overlay {
            Some(
                Overlay::Delete(_)
                | Overlay::Details { .. }
                | Overlay::Pipe { .. }
                | Overlay::Search { .. },
            ) => InputMode::Text,
            None if self
                .composer
                .as_ref()
                .is_some_and(|composer| match composer.prompt() {
                    Some(prompt) => prompt != Prompt::Rules,
                    None => !composer.previewing(),
                }) =>
            {
                InputMode::Text
            }
            Some(
                Overlay::Help { .. }
                | Overlay::Reason { .. }
                | Overlay::Vote { .. }
                | Overlay::Viewer { .. },
            )
            | None => InputMode::Commands,
        }
    }

    fn pasted(&mut self, pasted: &str) {
        let (text, max) = match &mut self.overlay {
            Some(Overlay::Details { text, .. }) => (text, SERVER_DETAILS_MAX),
            Some(Overlay::Pipe { command: text, .. } | Overlay::Search { query: text }) => {
                (text, usize::MAX)
            }
            _ => return,
        };
        let room = max.saturating_sub(text.chars().count());
        let flat = pasted
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch });
        text.extend(flat.take(room));
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
            Some(Overlay::Pipe { command: text, .. } | Overlay::Search { query: text }) => {
                match event.code {
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(ch)
                        if !event
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        text.push(ch);
                    }
                    _ => {}
                }
            }
            Some(
                Overlay::Help { .. }
                | Overlay::Reason { .. }
                | Overlay::Vote { .. }
                | Overlay::Viewer { .. },
            )
            | None => {}
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
            (Overlay::Viewer { id, at }, Action::Next | Action::Previous) => {
                let count = self.images(&id).len().max(1);
                let by = if action == Action::Next { 1 } else { count - 1 };
                (
                    Some(Overlay::Viewer {
                        at: (at + by) % count,
                        id,
                    }),
                    None,
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

    /// The question or the typing that takes the bottom row, and how it looks.
    fn prompt(&self, room: usize) -> Option<(String, Style)> {
        match &self.overlay {
            Some(Overlay::Delete(id)) => {
                let short = id.short_hash();
                let warned = format!("delete {short}? u undoes it for 30s [y/N]");
                let prompt = if warned.len() <= room {
                    warned
                } else {
                    format!("delete {short}? [y/N]")
                };
                Some((prompt, Style::new().fg(theme::DESTRUCTIVE)))
            }
            Some(Overlay::Pipe { command, .. }) => {
                Some((format!("| {command}█"), Style::new().fg(theme::FOREGROUND)))
            }
            Some(Overlay::Search { query }) if query.is_empty() => Some((
                "/█  words to find commits, or @name for people".to_owned(),
                Style::new().fg(theme::FOREGROUND),
            )),
            Some(Overlay::Search { query }) => {
                Some((format!("/{query}█"), Style::new().fg(theme::FOREGROUND)))
            }
            Some(
                Overlay::Help { .. }
                | Overlay::Reason { .. }
                | Overlay::Vote { .. }
                | Overlay::Details { .. }
                | Overlay::Viewer { .. },
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

fn open_profile(sched: &mut Scheduler, session: &mut Session, handle: String, now: Instant) {
    let profile = ProfileScreen::new(handle, now);
    sched.send(Call::User(profile.handle().to_owned()), Priority::Now);
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

fn followed(handle: &str, on: bool) -> String {
    let done = if on { "following" } else { "unfollowed" };
    format!("{done} @{handle} · f to undo")
}

fn muted(handle: &str, on: bool) -> String {
    match on {
        true => format!("muted @{handle} · m to undo"),
        false => format!("unmuted @{handle}"),
    }
}

fn untranslated(why: &Untranslated) -> String {
    match why {
        Untranslated::Already(language) => format!("already in {language}"),
        Untranslated::NoLanguage => "nothing to translate".to_owned(),
    }
}

/// Counts what came in without naming it, since reading the notifications would mark them read.
fn notified(new: u32) -> String {
    let what = if new == 1 {
        "notification"
    } else {
        "notifications"
    };
    format!("{new} new {what} · gn to read")
}

fn can(phase: &Phase, health: Option<&Failure>, sending: bool, action: Action) -> bool {
    match (phase, action) {
        (_, Action::Quit) => true,
        (_, Action::Send | Action::Editor | Action::Poll | Action::Attach | Action::Preview)
            if sending =>
        {
            false
        }
        (Phase::Splash, Action::Refresh) => health.is_some(),
        (Phase::Splash, _) => false,
        (Phase::SignIn(step), Action::SignIn) => matches!(step, Step::Ready(_)),
        (Phase::SignIn(step), Action::StartOver) => matches!(step, Step::Browser(_)),
        (Phase::SignIn(step), Action::Icons) => matches!(step, Step::Ready(_)),
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
    gallery: Gallery,
    reloaded_at: Option<Instant>,
    /// What the last frame had selected, so a new selection can lift into place.
    lifted: Option<String>,
    /// The language the device reads in, which commits translate into unless the user picked one.
    device_language: Option<String>,
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
            gallery: Gallery::new(Picker::halfblocks()),
            reloaded_at: None,
            lifted: None,
            device_language: None,
        }
    }

    /// Draws images the way this terminal knows best, found by asking it.
    pub fn with_picker(self, picker: Picker) -> Self {
        Engine {
            gallery: Gallery::new(picker),
            ..self
        }
    }

    pub fn with_drafts(self, drafts: Drafts) -> Self {
        Engine { drafts, ..self }
    }

    /// Translates into `device`, as the device's locale names it, unless the user picked a language on
    /// dev-null.sh.
    pub fn with_language(self, device: Option<String>) -> Self {
        Engine {
            device_language: device,
            ..self
        }
    }

    /// Opens on what the last run saw while the server is asked again, so the timeline shows at once
    /// and refreshes in place.
    pub fn with_start(mut self, start: Start, now: Instant) -> Self {
        self.open(start.account, now);
        if let Phase::Ready(session) = &mut self.phase {
            for page in start.pages {
                session.store.leave_off(&page);
                session
                    .store
                    .apply(&Call::Page(page.log), Reply::Page(page));
            }
        }
        self
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
            Event::Focused => {
                if self
                    .reloaded_at
                    .is_none_or(|at| now.duration_since(at) >= FOCUS_RELOAD)
                {
                    self.reload(false, now);
                }
                Vec::new()
            }
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
            Event::Fetched(url, image) => {
                self.gallery.fetched(url, image);
                Vec::new()
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
            Event::Piped(command, outcome) => {
                let toast = match outcome {
                    Ok(()) => format!("piped to {command}"),
                    Err(why) => format!("fatal: {command}: {why}"),
                };
                self.toast(toast, now);
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
                Timer::Glow | Timer::Spark(..) | Timer::Lift => {}
                Timer::Undo => {
                    if let Phase::Ready(session) = &mut self.phase {
                        session.undoable = None;
                    }
                }
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
        self.refill_logs();
        self.load_older();
        self.load_older_inbox();
        self.load_older_group();
        self.load_older_list();
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
                .any(|timer| matches!(timer, Timer::Spark(..) | Timer::Lift))
                .then(|| now + SPARK_FRAME),
            self.toast_fading(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Walks each reloaded log back to the depth it was read to.
    fn refill_logs(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        for log in LOGS {
            if let Some(cursor) = session.store.refilling(log)
                && !self.timers.is_set(&Timer::Older(log))
            {
                let cursor = cursor.to_owned();
                let priority = if log == session.log.active() {
                    Priority::Now
                } else {
                    Priority::Later
                };
                self.sched.send(Call::Older { log, cursor }, priority);
            }
        }
    }

    fn load_older(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        let log = session.log.active();
        let Slot::Loaded(listing) = session.store.slot(log) else {
            return;
        };
        if session.store.refilling(log).is_some() {
            return;
        }
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

    fn load_older_list(&mut self) {
        let Phase::Ready(session) = &self.phase else {
            return;
        };
        let call = match session.stack.last() {
            Some(Screen::Profile(profile)) => profile.wants_older(),
            Some(Screen::List(list)) => list.wants_older(),
            _ => None,
        };
        if let Some(call) = call {
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
            Some(Screen::List(list)) => list.placeholder_at(),
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
            (Phase::Ready(session), Action::Commit(Act::Browse))
                if let Some(Overlay::Viewer { id, at }) = &session.overlay =>
            {
                let url = session.images(id).get(*at).map(|image| image.url.clone());
                return url.map(Effect::Open).into_iter().collect();
            }
            (Phase::Ready(session), Action::Confirm)
                if matches!(session.overlay, Some(Overlay::Pipe { .. })) =>
            {
                if let Some(Overlay::Pipe { input, command }) = session.overlay.take() {
                    let command = command.trim().to_owned();
                    if !command.is_empty() {
                        session.piped = command.clone();
                        return vec![Effect::Pipe { command, input }];
                    }
                }
            }
            (Phase::Ready(session), Action::Confirm)
                if matches!(session.overlay, Some(Overlay::Search { .. })) =>
            {
                if let Some(Overlay::Search { query }) = session.overlay.take() {
                    let query = query.trim().to_owned();
                    if !query.is_empty() && query != "@" {
                        session.searched = query.clone();
                        let list = ListScreen::new(Source::search(&query), now);
                        self.sched.send(list.call(), Priority::Now);
                        // A search from results refines it, so back still leads where the first began.
                        if session
                            .list()
                            .is_some_and(|list| *list.source() != Source::Starred)
                        {
                            session.stack.pop();
                        }
                        session.stack.push(Screen::List(Box::new(list)));
                    }
                }
            }
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
            (Phase::Ready(session), Action::Undo) => {
                if let Some(id) = session.undoable.take() {
                    self.timers.clear(&Timer::Undo);
                    self.sched.send(Call::Restore(id), Priority::Now);
                }
            }
            (Phase::Ready(session), Action::Editor) => {
                if let Some(composer) = &session.composer {
                    return vec![Effect::Edit(composer.text().to_owned())];
                }
            }
            (Phase::Ready(_), Action::Poll) => return self.edit(Composer::toggle_poll),
            (Phase::Ready(session), Action::Preview) => {
                if let Some(composer) = &mut session.composer
                    && let Some(text) = composer.toggle_preview()
                {
                    self.sched.send(Call::Preview(text), Priority::Now);
                }
            }
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
            (Phase::SignIn(_), Action::Icons) => {
                let plain = !theme::plain_icons();
                theme::set_plain_icons(plain);
                return vec![Effect::Icons(plain)];
            }
            (Phase::Ready(session), Action::Inbox) => {
                if !session.on_inbox() {
                    session.stack.push(Screen::Inbox);
                    fetch_inbox(&mut self.sched, &mut self.timers, &mut session.inbox, now);
                }
            }
            (Phase::Ready(session), Action::Starred) => {
                if !session.on_starred() {
                    let list = ListScreen::new(Source::Starred, now);
                    self.sched.send(list.call(), Priority::Now);
                    session.stack.push(Screen::List(Box::new(list)));
                }
            }
            (Phase::Ready(session), Action::Search) => {
                session.overlay = Some(Overlay::Search {
                    query: session.searched.clone(),
                });
            }
            (Phase::Ready(_), Action::Refresh) => self.reload(true, now),
            (Phase::Ready(session), Action::NextFilter | Action::PreviousFilter)
                if session.on_inbox() =>
            {
                session.inbox.act(action, &session.store);
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
                if let Some(handle) = session.profile_target() {
                    open_profile(&mut self.sched, session, handle, now);
                }
            }
            (Phase::Ready(session), Action::Open) if session.group().is_some() => {
                let opens = session.group().and_then(GroupList::opens);
                self.open_row(opens, now);
            }
            (Phase::Ready(session), Action::Open) if session.list().is_some() => {
                let opens = session.list().and_then(ListScreen::opens);
                self.open_row(opens, now);
            }
            (Phase::Ready(session), Action::Open) if session.profile().is_some() => {
                let opens = session.profile().and_then(ProfileScreen::opens);
                self.open_row(opens, now);
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
            (Phase::Ready(session), action) => session.log.act(action, &session.store),
            (Phase::Splash | Phase::SignIn(_), _) => {}
        }
        Vec::new()
    }

    /// Reloads all the session shows: every log, the peek, and each screen on the stack.
    /// The inbox reloads only on screen, since reading it marks it read. `asked` toasts when the screen shown lands.
    fn reload(&mut self, asked: bool, now: Instant) {
        let Phase::Ready(session) = &mut self.phase else {
            return;
        };
        self.reloaded_at = Some(now);
        let shown = session.log.active();
        for log in LOGS {
            self.timers.clear(&Timer::Reload(log));
            self.timers.clear(&Timer::Older(log));
            let priority = if log == shown && session.stack.is_empty() {
                Priority::Now
            } else {
                Priority::Later
            };
            self.sched.send(Call::Page(log), priority);
        }
        if asked && session.stack.is_empty() {
            session.asked_to_refresh.insert(Refresh::Log(shown));
        }
        if let Some(peek) = session.log.peek(&session.store, now) {
            refresh_thread(&mut self.sched, &mut self.timers, peek, &session.store);
        }
        if session.on_inbox() {
            if asked {
                session
                    .asked_to_refresh
                    .insert(Refresh::Inbox(session.inbox.filter()));
            }
            fetch_inbox(&mut self.sched, &mut self.timers, &mut session.inbox, now);
        }
        for screen in &mut session.stack {
            match screen {
                Screen::Thread(thread) => {
                    refresh_thread(&mut self.sched, &mut self.timers, thread, &session.store);
                }
                Screen::Group(group) => fetch_group(&mut self.sched, group),
                Screen::Profile(profile) => {
                    profile.reload(now);
                    self.sched
                        .send(Call::User(profile.handle().to_owned()), Priority::Now);
                    self.sched.send(profile.tab_call(), Priority::Now);
                }
                Screen::List(list) => {
                    list.reload(now);
                    self.sched.send(list.call(), Priority::Now);
                }
                Screen::Inbox => {}
            }
        }
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
            Some(profile::Opens::Profile(handle)) => {
                open_profile(&mut self.sched, session, handle, now);
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
                let author = user.handle.clone();
                if let Some(on) = session.store.follow(&author) {
                    self.toast(followed(&author, on), now);
                }
                return Vec::new();
            }
            None => return Vec::new(),
        };
        let (id, author, url) = (commit.id, commit.author.handle, commit.html_url);
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
            Act::Mute => {
                let on = session.store.mute(&author);
                (Some(muted(&author, on)), Vec::new())
            }
            Act::Translate => (
                session.store.translate(&id).map(|why| untranslated(&why)),
                Vec::new(),
            ),
            Act::Copy => (Some(format!("copied {url}")), vec![Effect::Copy(url)]),
            Act::Browse => (
                Some("opened in the browser".to_owned()),
                vec![Effect::Open(url)],
            ),
            Act::Images => {
                let urls: Vec<String> = (session.images(&id).iter())
                    .map(|image| image.url.clone())
                    .collect();
                session.overlay = Some(Overlay::Viewer { id, at: 0 });
                (
                    None,
                    self.gallery
                        .want(&urls)
                        .into_iter()
                        .map(Effect::Fetch)
                        .collect(),
                )
            }
            Act::OpenQuote => {
                if let Some(Shown::Live(quote)) =
                    (session.store.row(&id)).and_then(|row| row.quoted_commit.as_ref())
                {
                    let id = quote.commit.id.clone();
                    session
                        .stack
                        .push(Screen::Thread(Box::new(ThreadScreen::new(id.clone(), now))));
                    fetch_thread(&mut self.sched, &mut self.timers, id);
                }
                (None, Vec::new())
            }
            Act::Delete => {
                session.overlay = Some(Overlay::Delete(id));
                (None, Vec::new())
            }
            Act::Pipe => {
                session.overlay = Some(Overlay::Pipe {
                    input: commit.body,
                    command: session.piped.clone(),
                });
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
        if let (Call::Preview(text), Reply::Preview(segments)) = (call, &reply) {
            if let Some(composer) = &mut session.composer {
                composer.rendered(text, Some(segments.clone()));
            }
            return Vec::new();
        }
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
            Call::Commits { .. }
            | Call::People { .. }
            | Call::Starred { .. }
            | Call::SearchCommits(_)
            | Call::SearchPeople(_) => {
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
                if let Some((handle, tab, cursor)) = Tab::of(call) {
                    for profile in session.profiles_mut(handle) {
                        profile.land(tab, cursor, items.clone(), next.clone());
                    }
                } else if let Some((source, cursor)) = Source::of(call) {
                    for list in session.lists_mut(&source) {
                        list.land(cursor, items.clone(), next.clone());
                    }
                }
                return Vec::new();
            }
            Call::User(handle) => {
                if let Reply::User(profile) = &reply {
                    for screen in session.profiles_mut(handle) {
                        screen.set_profile(profile.as_ref().clone());
                    }
                }
            }
            _ => {}
        }
        let (Call::Page(log) | Call::Older { log, .. }) = call else {
            let answered = match (call, &reply) {
                (Call::User(handle), Reply::User(profile)) => Some((handle, profile.is_following)),
                _ => None,
            };
            let said = match (call, &reply) {
                (_, Reply::Pastel { commit, remaining }) => Some(format!(
                    "gave a pastel to @{} · {remaining} left today",
                    commit.card.commit.author.handle
                )),
                (Call::Delete(id), Reply::Done) => {
                    session.undoable = Some(id.clone());
                    self.timers.set(Timer::Undo, now + UNDO);
                    Some(format!("deleted {} · u to undo", id.short_hash()))
                }
                (Call::Restore(id), Reply::Commit(_)) => {
                    Some(format!("restored {}", id.short_hash()))
                }
                (Call::Report { id, .. }, Reply::Done) => {
                    Some(format!("reported {}. thanks.", id.short_hash()))
                }
                (Call::Translate { .. }, Reply::Translation(translation)) => match &**translation {
                    Translation::Skipped(why) => Some(untranslated(why)),
                    Translation::Done(_) => None,
                },
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
            let mut news = None;
            if session.store.unread > unread {
                self.timers.set(Timer::Glow, now + GLOW);
                if !session.on_inbox() {
                    news = Some(notified(session.store.unread - unread));
                }
                // A new notification may be a reply in the thread on screen.
                if let Some(thread) = session.shown_thread(now) {
                    refresh_thread(&mut self.sched, &mut self.timers, thread, &session.store);
                }
            }
            let toast = said.or_else(|| {
                let (handle, server) = answered?;
                let want = session.store.following(handle)?;
                (want != server).then(|| followed(handle, want))
            });
            if let Some(text) = toast.or(news) {
                self.toast(text, now);
            }
            return Vec::new();
        };
        let selected = session.log.selected_id(*log, &session.store).cloned();
        session.store.apply(call, reply);
        session
            .log
            .reselect(*log, selected.as_ref(), &session.store);
        if session.store.refilling(*log).is_none()
            && session.asked_to_refresh.remove(&Refresh::Log(*log))
        {
            self.toast(format!("{} refreshed", log.slug()), now);
        }
        Vec::new()
    }

    fn open(&mut self, account: Account, now: Instant) {
        self.phase = Phase::Ready(Box::new(Session {
            store: Store::new(account).with_device_language(self.device_language.clone()),
            log: LogScreen::default(),
            inbox: Inbox::default(),
            stack: Vec::new(),
            overlay: None,
            composer: None,
            asked_to_refresh: HashSet::new(),
            filled: HashSet::new(),
            piped: String::new(),
            searched: String::new(),
            undoable: None,
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
        if let Call::Preview(text) = &call
            && !error.needs_sign_in()
        {
            if let Phase::Ready(session) = &mut self.phase
                && let Some(composer) = &mut session.composer
            {
                composer.rendered(text, None);
            }
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
                if let Call::User(handle) = &call {
                    for profile in session.profiles_mut(handle) {
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
                        if let Some((handle, tab, _)) = Tab::of(&call) {
                            for profile in session.profiles_mut(handle) {
                                profile.fail_tab(tab, failure.clone());
                            }
                        }
                        offline
                    }
                    Call::Starred { .. } | Call::SearchCommits(_) | Call::SearchPeople(_) => {
                        if let Some((source, _)) = Source::of(&call) {
                            for list in session.lists_mut(&source) {
                                list.fail(failure.clone());
                            }
                        }
                        offline
                    }
                    Call::Star { .. } | Call::Follow { .. } | Call::Mute { .. } | Call::User(_)
                        if !matches!(failure, Failure::Refused(_)) =>
                    {
                        true
                    }
                    Call::Star { .. }
                    | Call::Follow { .. }
                    | Call::Mute { .. }
                    | Call::User(_)
                    | Call::Delete(_)
                    | Call::Restore(_)
                    | Call::Pastel(_)
                    | Call::Vote { .. }
                    | Call::Report { .. }
                    | Call::Translate { .. } => {
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
                                Call::Follow { handle, .. } | Call::User(handle),
                            ) => format!("fatal: could not follow @{handle}"),
                            _ => error.to_string(),
                        };
                        self.toast = Some(text);
                        self.timers.set(Timer::Toast, now + TOAST);
                        offline
                    }
                    Call::Settings => {
                        session.store.refused(&call);
                        offline
                    }
                    Call::Me | Call::Redeem { .. } | Call::Since { .. } => true,
                    Call::Post { .. } | Call::Preview(_) => false,
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

    /// When the toast next needs a frame to fade, at the start and end of its time.
    fn toast_fading(&self, now: Instant) -> Option<Instant> {
        let left = self.timers.remaining(&Timer::Toast, now)?;
        let fading = TOAST.saturating_sub(left) < FADE || left < FADE;
        theme::motion().then(|| {
            if fading {
                now + SPARK_FRAME
            } else {
                now + left - FADE
            }
        })
    }

    /// How much of the toast shows, rising as it arrives and falling away before it goes.
    fn toast_shown(&self, now: Instant) -> f32 {
        match self.timers.remaining(&Timer::Toast, now) {
            Some(left) if theme::motion() => {
                let edge = TOAST.saturating_sub(left).min(left);
                edge.as_secs_f32() / FADE.as_secs_f32()
            }
            _ => 1.0,
        }
    }

    pub fn draw<Tz: TimeZone>(&mut self, frame: &mut Frame, now: Instant, wall: &DateTime<Tz>) {
        let selected = match &self.phase {
            Phase::Ready(session) => {
                theme::set_pasteis(session.store.me.pasteis_mode);
                session.selected().map(|selected| match selected {
                    Selected::Row(row) => row.commit.id.0.clone(),
                    Selected::Note(commit) => commit.id.0.clone(),
                    Selected::Person(user) => user.handle.clone(),
                })
            }
            _ => None,
        };
        if selected != self.lifted {
            // Only a move from one card to another lifts. The first frame lands whole.
            if theme::motion() && self.lifted.is_some() && selected.is_some() {
                self.timers.set(Timer::Lift, now + FADE);
            }
            self.lifted = selected;
        }
        theme::set_lift(
            self.timers
                .remaining(&Timer::Lift, now)
                .map_or(1.0, |left| 1.0 - left.as_secs_f32() / FADE.as_secs_f32()),
        );
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
        let toast_shown = self.toast_shown(now);
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
                    .map(|profile| format!("@{}", profile.handle()));
                let title = match session.stack.last() {
                    Some(Screen::Inbox | Screen::Group(_)) => Some(inbox::TITLE),
                    Some(Screen::List(list)) => Some(list.title()),
                    _ => handle.as_deref(),
                };
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
                let framed = log::boxes(inset(body));
                if session.on_inbox() && !framed {
                    let banner = inset(banner);
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
                    (Some((prompt, style)), _) => {
                        Line::styled(format!("{HINT_INDENT}{prompt}"), style)
                    }
                    (None, Some(error)) => Line::from(vec![
                        Span::styled(
                            format!("{HINT_INDENT}{error}"),
                            Style::new().fg(theme::DESTRUCTIVE),
                        ),
                        Span::styled(help, theme::muted()),
                    ]),
                    (None, None) => match &toast {
                        Some(toast) => Line::styled(
                            format!("{HINT_INDENT}{toast}"),
                            Style::new().fg(theme::blend(
                                theme::MUTED_FOREGROUND,
                                theme::MUTED,
                                toast_shown,
                            )),
                        ),
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
                        let area = framed_as(frame, body, framed, "thread");
                        thread.draw(frame, area, &session.store, &self.timers, now, wall)
                    }
                    Some(Screen::Inbox) => {
                        let retry = Retry {
                            page: self.timers.remaining(&Timer::Inbox(filter), now),
                            older: self.timers.remaining(&Timer::OlderInbox(filter), now),
                        };
                        let area = if framed {
                            session.inbox.draw_box(frame, log::column(inset(body)))
                        } else {
                            inset(body)
                        };
                        session.inbox.draw(frame, area, retry, now, wall);
                    }
                    Some(Screen::Group(group)) => {
                        let area = framed_as(frame, body, framed, "");
                        group.draw(frame, area, now, wall)
                    }
                    Some(Screen::Profile(profile)) => profile.draw(
                        frame,
                        log::column(inset(body)),
                        &session.store,
                        &self.timers,
                        now,
                        wall,
                    ),
                    Some(Screen::List(list)) => {
                        let area = framed_as(frame, body, framed, "");
                        list.draw(frame, area, &session.store, &self.timers, now, wall)
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
                    Some(Overlay::Viewer { id, at }) => {
                        popup::dim(frame.buffer_mut(), screen, None);
                        (self.gallery).draw(frame, screen, id, session.images(id), *at);
                    }
                    Some(
                        Overlay::Delete(_)
                        | Overlay::Vote { .. }
                        | Overlay::Pipe { .. }
                        | Overlay::Search { .. },
                    )
                    | None => {}
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
    format!("↑ {count} new {noun} · ctrl-r to load")
}

/// A line of help: a group's title, one of its keys, or the gap before the next group.
enum HelpRow {
    Title(&'static str),
    Key(keys::Entry),
    Gap,
}

type Section = (&'static str, Vec<keys::Entry>);

/// The keys for this screen in their groups, and the composer's apart from them.
fn help_sections(session: &Session) -> (Vec<Section>, Section) {
    let allows = |action| session.screen_allows(action);
    let mut entries = keys::help(session.screen_scopes(), &allows);
    entries.sort_by_key(|entry| entry.group);
    let mut sections: Vec<Section> = Vec::new();
    for entry in entries {
        match sections.last_mut() {
            Some((title, list)) if *title == entry.group.label() => list.push(entry),
            _ => sections.push((entry.group.label(), vec![entry])),
        }
    }
    // `?` types itself in the composer, so its keys are listed here instead.
    let writing = keys::help(COMPOSER_SCOPES, &|action| action != Action::Quit)
        .into_iter()
        .filter(|entry| entry.live)
        .collect();
    (sections, ("while writing", writing))
}

fn help_rows(sections: &[Section]) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for (title, entries) in sections {
        if !rows.is_empty() {
            rows.push(HelpRow::Gap);
        }
        rows.push(HelpRow::Title(title));
        rows.extend(entries.iter().cloned().map(HelpRow::Key));
    }
    rows
}

/// How wide a column's keys are, and the whole column.
fn help_widths(rows: &[HelpRow]) -> (usize, usize) {
    let keys = rows
        .iter()
        .filter_map(|row| match row {
            HelpRow::Key(entry) => Some(entry.keys.width()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let width = rows
        .iter()
        .map(|row| match row {
            HelpRow::Key(entry) => keys + 2 + entry.help.width(),
            HelpRow::Title(title) => keys + 2 + title.width(),
            HelpRow::Gap => 0,
        })
        .max()
        .unwrap_or(0);
    (keys, width)
}

const HELP_GUTTER: u16 = 6;

/// Help laid out in bands, top to bottom, each band columns side by side.
type HelpBands = Vec<Vec<Vec<HelpRow>>>;

/// The bands help shows, the box they go in, and the furthest row the list can scroll to.
/// The screen's groups split into two even columns with the composer's keys across
/// the foot when that fits; otherwise everything is one column that scrolls.
fn help_layout(screen: Rect, session: &Session) -> (HelpBands, Rect, usize) {
    let (sections, writing) = help_sections(session);
    let size = |bands: &HelpBands| {
        let width = bands
            .iter()
            .map(|band| {
                band.iter().map(|rows| help_widths(rows).1).sum::<usize>()
                    + usize::from(HELP_GUTTER) * (band.len() - 1)
            })
            .max()
            .unwrap_or(0)
            + 6;
        let height = bands
            .iter()
            .map(|band| band.iter().map(Vec::len).max().unwrap_or(0))
            .sum::<usize>()
            + bands.len()
            - 1
            + 4;
        (
            u16::try_from(width).unwrap_or(u16::MAX),
            u16::try_from(height).unwrap_or(u16::MAX),
        )
    };
    let columns = (1..sections.len())
        .map(|at| {
            let (left, right) = sections.split_at(at);
            vec![help_rows(left), help_rows(right)]
        })
        .min_by_key(|columns| columns.iter().map(Vec::len).max());
    if let Some(columns) = columns {
        let bands = vec![columns, vec![help_rows(std::slice::from_ref(&writing))]];
        let (width, height) = size(&bands);
        // Like a scrolling list, the columns give up their blank rows before they give up.
        if width < screen.width && height - 2 < screen.height {
            return (bands, popup::place(screen, width, height, None), 0);
        }
    }
    let mut all = sections;
    all.push(writing);
    let bands = vec![vec![help_rows(&all)]];
    let (width, height) = size(&bands);
    let area = popup::place(screen, width, height, None);
    let last = if area.height < height {
        bands[0][0]
            .len()
            .saturating_sub(usize::from(area.height.saturating_sub(2)))
    } else {
        0
    };
    (bands, area, last)
}

fn help_line(row: HelpRow, key_width: usize) -> Line<'static> {
    match row {
        HelpRow::Title(title) => Line::from(vec![
            Span::raw(" ".repeat(key_width + 2)),
            Span::styled(title, theme::muted().add_modifier(Modifier::BOLD)),
        ]),
        HelpRow::Key(entry) => {
            let (key_style, help_style) = if entry.live {
                (
                    Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    Style::new().fg(theme::FOREGROUND),
                )
            } else {
                (Style::new().fg(theme::FAINT), Style::new().fg(theme::FAINT))
            };
            Line::from(vec![
                Span::styled(format!("{:>key_width$}", entry.keys), key_style),
                Span::raw("  "),
                Span::styled(entry.help, help_style),
            ])
        }
        HelpRow::Gap => Line::default(),
    }
}

/// Draws the keys from row `top`, which the caller has already held within reach.
fn draw_help(frame: &mut Frame, screen: Rect, session: &Session, top: usize) {
    let (bands, area, last) = help_layout(screen, session);
    let block = Block::bordered().title(Span::styled(
        " keys ",
        Style::new()
            .fg(theme::FOREGROUND)
            .add_modifier(Modifier::BOLD),
    ));
    let content: usize = bands
        .iter()
        .map(|band| band.iter().map(Vec::len).max().unwrap_or(0) + 1)
        .sum();
    // A list too tall for the screen gives up its blank rows before it scrolls.
    let block = if usize::from(area.height) < content + 3 {
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
    let mut y = inner.y;
    for band in bands {
        let height = band.iter().map(Vec::len).max().unwrap_or(0);
        let mut x = inner.x;
        for rows in band {
            let (key_width, width) = help_widths(&rows);
            let lines: Vec<Line> = rows
                .into_iter()
                .skip(top)
                .map(|row| help_line(row, key_width))
                .collect();
            let column = Rect {
                x,
                y,
                width: u16::try_from(width).unwrap_or(u16::MAX),
                height: inner.bottom().saturating_sub(y),
            }
            .intersection(inner);
            frame.render_widget(Paragraph::new(lines), column);
            x = column.right().saturating_add(HELP_GUTTER);
        }
        y = y.saturating_add(u16::try_from(height + 1).unwrap_or(u16::MAX));
    }
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

/// The body as a box titled `title` when `framed`, else as a bare inset, returning where its
/// screen draws.
fn framed_as(frame: &mut Frame, body: Rect, framed: bool, title: &str) -> Rect {
    if framed {
        log::draw_box(frame, log::column(inset(body)), title, true)
    } else {
        inset(body)
    }
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
