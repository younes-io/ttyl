use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use crate::model::{Hush, Log, Reason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Down,
    Up,
    Top,
    Bottom,
    Expand,
    Open,
    Back,
    Show(Log),
    NextLog,
    Inbox,
    /// Opens the list of conversations.
    Messages,
    Starred,
    /// Lists what dev-null.sh is talking about right now.
    Headlines,
    /// Asks what to search for.
    Search,
    Profile,
    NextFilter,
    PreviousFilter,
    Refresh,
    Help,
    Close,
    SignIn,
    StartOver,
    Icons,
    Commit(Act),
    Confirm,
    Pick(Reason),
    Choose(usize),
    Next,
    Previous,
    Compose,
    Send,
    Editor,
    Poll,
    /// Scrolls a conversation back a page, or forward.
    PageUp,
    PageDown,
    Attach,
    Preview,
    Site,
    Saver,
    Link(Link),
    /// Brings back the commit deleted moments ago.
    Undo,
    /// Asks when the commit being written goes out.
    Schedule,
    /// Changes whom the commit being written goes out as: you, or an organization you're in.
    PostAs,
    /// Lists the commits scheduled to go out later.
    Later,
    /// Cancels the selected scheduled commit.
    Unschedule,
    /// Commits the selected scheduled commit now.
    SendNow,
    /// Asks for a name to narrow the list of conversations to, which also finds people to start
    /// one with.
    Find,
    /// Acts on the selected conversation in the list.
    Keep(Keep),
    /// Mutes the conversation asked about for this long.
    Hush(Hush),
}

/// What a key does to the selected conversation, as mutt's keys flag and save a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Pin,
    Stash,
    /// Asks how long to mute it for, or unmutes it.
    Mute,
    /// Merges a request, so the user can reply.
    Merge,
    /// Asks before dropping a request for good.
    Drop,
}

/// A place of the person a profile is about, out in the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Website,
    GitHub,
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Star,
    Follow,
    Copy,
    Browse,
    Pastel,
    Delete,
    Report,
    Reply,
    Quote,
    Vote,
    /// Shows the commit's images.
    Images,
    /// Opens the thread of the commit it quotes.
    OpenQuote,
    /// Sends the commit's text to a shell command.
    Pipe,
    /// Mutes the author, or unmutes them.
    Mute,
    /// Swaps the body for its translation, or back to the original.
    Translate,
    /// Lists the commits nearest in meaning to this one.
    Related,
    /// Asks before blocking the author, or unblocks them.
    Block,
    /// Opens the conversation with the author or the person, starting it if there is none.
    Message,
    /// Pins the user's commit to their profile, or unpins it.
    Pin,
}

/// Where a key sits in help: what it does, not where it works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Move,
    Read,
    Act,
    Write,
    Go,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Move => "move",
            Group::Read => "read",
            Group::Act => "act",
            Group::Write => "write",
            Group::Go => "go",
        }
    }
}

impl Action {
    pub fn group(self) -> Group {
        match self {
            Action::Down
            | Action::Up
            | Action::Top
            | Action::Bottom
            | Action::Next
            | Action::Previous
            | Action::NextFilter
            | Action::PreviousFilter
            | Action::PageUp
            | Action::PageDown
            | Action::Choose(_) => Group::Move,
            Action::Open
            | Action::Expand
            | Action::Back
            | Action::Profile
            | Action::Preview
            | Action::Link(_)
            | Action::Commit(Act::Images | Act::OpenQuote | Act::Translate | Act::Related) => {
                Group::Read
            }
            Action::Compose
            | Action::Send
            | Action::Editor
            | Action::Poll
            | Action::Attach
            | Action::Schedule
            | Action::PostAs
            | Action::Commit(Act::Reply | Act::Quote | Act::Message) => Group::Write,
            Action::Show(_)
            | Action::NextLog
            | Action::Inbox
            | Action::Messages
            | Action::Starred
            | Action::Headlines
            | Action::Later
            | Action::Search
            | Action::Find
            | Action::Refresh
            | Action::Help
            | Action::Close
            | Action::Saver
            | Action::Quit => Group::Go,
            Action::Commit(_)
            | Action::Confirm
            | Action::Pick(_)
            | Action::Site
            | Action::SignIn
            | Action::StartOver
            | Action::Icons
            | Action::Unschedule
            | Action::SendNow
            | Action::Keep(_)
            | Action::Hush(_)
            | Action::Undo => Group::Act,
        }
    }
}

/// One row of help: its keys, what they do, whether they work right now, and its group.
#[derive(Debug, Clone)]
pub struct Entry {
    pub keys: String,
    pub help: &'static str,
    pub live: bool,
    pub group: Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Always,
    Splash,
    Gate,
    Help,
    Log,
    Thread,
    Inbox,
    Group,
    Profile,
    Person,
    Listed,
    List,
    Commit,
    People,
    Global,
    Confirm,
    Report,
    Details,
    Vote,
    Viewer,
    Composer,
    Preview,
    Secret,
    Rules,
    Pipe,
    Search,
    At,
    Queue,
    Messages,
    Conversation,
    /// Asking how long to mute a conversation for.
    Mute,
}

pub const SPLASH_SCOPES: &[Scope] = &[Scope::Splash, Scope::Always];
pub const GATE_SCOPES: &[Scope] = &[Scope::Gate, Scope::Always];
pub const HELP_SCOPES: &[Scope] = &[Scope::Help, Scope::Always];
pub const CONFIRM_SCOPES: &[Scope] = &[Scope::Confirm, Scope::Always];
pub const REPORT_SCOPES: &[Scope] = &[Scope::Report, Scope::Always];
pub const VOTE_SCOPES: &[Scope] = &[Scope::Vote, Scope::Always];
pub const VIEWER_SCOPES: &[Scope] = &[Scope::Viewer, Scope::Always];
pub const DETAILS_SCOPES: &[Scope] = &[Scope::Details];
pub const COMPOSER_SCOPES: &[Scope] = &[Scope::Composer];
pub const PREVIEW_SCOPES: &[Scope] = &[Scope::Preview];
pub const SECRET_SCOPES: &[Scope] = &[Scope::Secret];
pub const RULES_SCOPES: &[Scope] = &[Scope::Rules];
pub const PIPE_SCOPES: &[Scope] = &[Scope::Pipe];
pub const SEARCH_SCOPES: &[Scope] = &[Scope::Search];
pub const AT_SCOPES: &[Scope] = &[Scope::At];
pub const CONVERSATION_SCOPES: &[Scope] = &[Scope::Conversation];
pub const MUTE_SCOPES: &[Scope] = &[Scope::Mute, Scope::Always];
pub const LOG_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Log,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const THREAD_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Thread,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const INBOX_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Inbox,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const GROUP_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Group,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const PEOPLE_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Group,
    Scope::People,
    Scope::Global,
    Scope::Always,
];
pub const PROFILE_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Profile,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const PROFILE_PEOPLE_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Profile,
    Scope::Person,
    Scope::Global,
    Scope::Always,
];
pub const LIST_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Listed,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
];
pub const LIST_PEOPLE_SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Listed,
    Scope::Person,
    Scope::Global,
    Scope::Always,
];
pub const QUEUE_SCOPES: &[Scope] = &[Scope::List, Scope::Queue, Scope::Global, Scope::Always];
pub const MESSAGES_SCOPES: &[Scope] = &[Scope::List, Scope::Messages, Scope::Global, Scope::Always];
pub const UNIONS: &[&[Scope]] = &[
    SPLASH_SCOPES,
    GATE_SCOPES,
    HELP_SCOPES,
    CONFIRM_SCOPES,
    REPORT_SCOPES,
    VOTE_SCOPES,
    VIEWER_SCOPES,
    DETAILS_SCOPES,
    COMPOSER_SCOPES,
    PREVIEW_SCOPES,
    SECRET_SCOPES,
    RULES_SCOPES,
    PIPE_SCOPES,
    SEARCH_SCOPES,
    LOG_SCOPES,
    THREAD_SCOPES,
    INBOX_SCOPES,
    GROUP_SCOPES,
    PEOPLE_SCOPES,
    PROFILE_SCOPES,
    PROFILE_PEOPLE_SCOPES,
    LIST_SCOPES,
    LIST_PEOPLE_SCOPES,
    AT_SCOPES,
    QUEUE_SCOPES,
    MESSAGES_SCOPES,
    CONVERSATION_SCOPES,
    MUTE_SCOPES,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Commands,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    code: KeyCode,
    ctrl: bool,
}

impl Key {
    const fn ch(ch: char) -> Self {
        Key {
            code: KeyCode::Char(ch),
            ctrl: false,
        }
    }

    const fn code(code: KeyCode) -> Self {
        Key { code, ctrl: false }
    }

    const fn ctrl(ch: char) -> Self {
        Key {
            code: KeyCode::Char(ch),
            ctrl: true,
        }
    }

    pub fn label(self) -> String {
        let code = match self.code {
            KeyCode::Char(' ') => "space".to_owned(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::Enter => "enter".to_owned(),
            KeyCode::Esc => "esc".to_owned(),
            KeyCode::Tab => "tab".to_owned(),
            KeyCode::BackTab => "shift-tab".to_owned(),
            KeyCode::Up => "↑".to_owned(),
            KeyCode::Down => "↓".to_owned(),
            KeyCode::Home => "home".to_owned(),
            KeyCode::PageUp => "pgup".to_owned(),
            KeyCode::PageDown => "pgdn".to_owned(),
            KeyCode::End => "end".to_owned(),
            other => format!("{other:?}").to_lowercase(),
        };
        if self.ctrl {
            format!("ctrl-{code}")
        } else {
            code
        }
    }
}

impl From<KeyEvent> for Key {
    fn from(event: KeyEvent) -> Self {
        Key {
            code: event.code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
        }
    }
}

pub struct Binding {
    pub scope: Scope,
    pub chords: &'static [&'static [Key]],
    pub action: Action,
    pub hint: Option<&'static str>,
    pub help: &'static str,
}

pub const TABLE: &[Binding] = &[
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::List,
        chords: &[&[Key::ch('j')], &[Key::code(KeyCode::Down)]],
        action: Action::Down,
        hint: Some("j/k move"),
        help: "move down",
    },
    Binding {
        scope: Scope::List,
        chords: &[&[Key::ch('k')], &[Key::code(KeyCode::Up)]],
        action: Action::Up,
        hint: None,
        help: "move up",
    },
    Binding {
        scope: Scope::List,
        chords: &[&[Key::ch('g'), Key::ch('g')], &[Key::code(KeyCode::Home)]],
        action: Action::Top,
        hint: None,
        help: "top",
    },
    Binding {
        scope: Scope::List,
        chords: &[&[Key::ch('G')], &[Key::code(KeyCode::End)]],
        action: Action::Bottom,
        hint: None,
        help: "bottom",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open the thread",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::ch(' ')]],
        action: Action::Expand,
        hint: None,
        help: "expand the selected commit",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::ch('1')]],
        action: Action::Show(Log::Following),
        hint: None,
        help: "following",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::ch('2')]],
        action: Action::Show(Log::Latest),
        hint: None,
        help: "latest",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::ch('3')]],
        action: Action::Show(Log::Trending),
        hint: None,
        help: "trending",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::code(KeyCode::Tab)]],
        action: Action::NextLog,
        hint: None,
        help: "next log",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open this commit's thread",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::ch(' ')]],
        action: Action::Expand,
        hint: None,
        help: "expand the selected commit",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::ch('1')]],
        action: Action::Show(Log::Following),
        hint: None,
        help: "following",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::ch('2')]],
        action: Action::Show(Log::Latest),
        hint: None,
        help: "latest",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::ch('3')]],
        action: Action::Show(Log::Trending),
        hint: None,
        help: "trending",
    },
    Binding {
        scope: Scope::Thread,
        chords: &[&[Key::code(KeyCode::Tab)]],
        action: Action::NextLog,
        hint: None,
        help: "next log",
    },
    Binding {
        scope: Scope::Inbox,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open",
    },
    Binding {
        scope: Scope::Inbox,
        chords: &[&[Key::code(KeyCode::Tab)]],
        action: Action::NextFilter,
        hint: Some("tab filter"),
        help: "next filter",
    },
    Binding {
        scope: Scope::Inbox,
        chords: &[&[Key::code(KeyCode::BackTab)]],
        action: Action::PreviousFilter,
        hint: None,
        help: "previous filter",
    },
    Binding {
        scope: Scope::Inbox,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::Group,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open this commit's thread",
    },
    Binding {
        scope: Scope::Group,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::ch(' ')]],
        action: Action::Expand,
        hint: None,
        help: "expand the selected commit",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::code(KeyCode::Tab)]],
        action: Action::NextFilter,
        hint: Some("tab tabs"),
        help: "next tab",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::code(KeyCode::BackTab)]],
        action: Action::PreviousFilter,
        hint: None,
        help: "previous tab",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::ch('g'), Key::ch('w')]],
        action: Action::Link(Link::Website),
        hint: None,
        help: "open their website",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::ch('g'), Key::ch('h')]],
        action: Action::Link(Link::GitHub),
        hint: None,
        help: "open their GitHub",
    },
    Binding {
        scope: Scope::Profile,
        chords: &[&[Key::ch('g'), Key::ch('d')]],
        action: Action::Link(Link::Page),
        hint: None,
        help: "open their dev-null page",
    },
    Binding {
        scope: Scope::Listed,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open the thread, or the profile",
    },
    Binding {
        scope: Scope::Listed,
        chords: &[&[Key::ch(' ')]],
        action: Action::Expand,
        hint: None,
        help: "expand the selected commit",
    },
    Binding {
        scope: Scope::Listed,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::Queue,
        chords: &[&[Key::ch('S')]],
        action: Action::SendNow,
        hint: Some("S commit now"),
        help: "commit it now",
    },
    Binding {
        scope: Scope::Queue,
        chords: &[&[Key::ch('D')]],
        action: Action::Unschedule,
        hint: Some("D cancel"),
        help: "cancel it",
    },
    Binding {
        scope: Scope::Queue,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back",
    },
    Binding {
        scope: Scope::Person,
        chords: &[&[Key::ch('f')]],
        action: Action::Commit(Act::Follow),
        hint: Some("f follow"),
        help: "follow or unfollow",
    },
    Binding {
        scope: Scope::Person,
        chords: &[&[Key::ch('B')]],
        action: Action::Commit(Act::Block),
        hint: None,
        help: "block or unblock",
    },
    Binding {
        scope: Scope::Person,
        chords: &[&[Key::ch('M')]],
        action: Action::Commit(Act::Message),
        hint: None,
        help: "message them",
    },
    Binding {
        scope: Scope::Person,
        chords: &[&[Key::ch('p')]],
        action: Action::Profile,
        hint: None,
        help: "open their profile",
    },
    Binding {
        scope: Scope::People,
        chords: &[&[Key::ch('p')]],
        action: Action::Profile,
        hint: None,
        help: "open their profile",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('p')]],
        action: Action::Profile,
        hint: None,
        help: "open the author's profile",
    },
    Binding {
        scope: Scope::People,
        chords: &[&[Key::ch('f')]],
        action: Action::Commit(Act::Follow),
        hint: Some("f follow back"),
        help: "follow back",
    },
    Binding {
        scope: Scope::People,
        chords: &[&[Key::ch('B')]],
        action: Action::Commit(Act::Block),
        hint: None,
        help: "block or unblock",
    },
    Binding {
        scope: Scope::People,
        chords: &[&[Key::ch('M')]],
        action: Action::Commit(Act::Message),
        hint: None,
        help: "message them",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('s')]],
        action: Action::Commit(Act::Star),
        hint: Some("s star"),
        help: "star or unstar",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('c')]],
        action: Action::Compose,
        hint: Some("c commit"),
        help: "write a commit",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('u')]],
        action: Action::Undo,
        hint: Some("u undo"),
        help: "undo the delete, for 30 seconds",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('f')]],
        action: Action::Commit(Act::Follow),
        hint: None,
        help: "follow or unfollow the author",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('m')]],
        action: Action::Commit(Act::Mute),
        hint: None,
        help: "mute or unmute the author",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('B')]],
        action: Action::Commit(Act::Block),
        hint: None,
        help: "block or unblock the author",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('M')]],
        action: Action::Commit(Act::Message),
        hint: None,
        help: "message the author",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('^')]],
        action: Action::Commit(Act::Pin),
        hint: None,
        help: "pin to your profile, or unpin",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('y')]],
        action: Action::Commit(Act::Copy),
        hint: None,
        help: "copy the link",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('O')]],
        action: Action::Commit(Act::Browse),
        hint: None,
        help: "open in the browser",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('|')]],
        action: Action::Commit(Act::Pipe),
        hint: None,
        help: "pipe the text to a shell command",
    },
    Binding {
        scope: Scope::Pipe,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter run"),
        help: "run",
    },
    Binding {
        scope: Scope::Pipe,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Pipe,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Search,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter search"),
        help: "search",
    },
    Binding {
        scope: Scope::Search,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Search,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::At,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter schedule"),
        help: "schedule it",
    },
    Binding {
        scope: Scope::At,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc back"),
        help: "back to the draft",
    },
    Binding {
        scope: Scope::At,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('P')]],
        action: Action::Commit(Act::Pastel),
        hint: None,
        help: "give a pastel",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('D')]],
        action: Action::Commit(Act::Delete),
        hint: None,
        help: "delete",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('!')]],
        action: Action::Commit(Act::Report),
        hint: None,
        help: "report",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('r')]],
        action: Action::Commit(Act::Reply),
        hint: None,
        help: "reply",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('q')]],
        action: Action::Commit(Act::Quote),
        hint: None,
        help: "quote",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('v')]],
        action: Action::Commit(Act::Vote),
        hint: Some("v vote"),
        help: "vote in the poll",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('i')]],
        action: Action::Commit(Act::Images),
        hint: Some("i view"),
        help: "view the images",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('t')]],
        action: Action::Commit(Act::Translate),
        hint: None,
        help: "translate, or show the original",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('g'), Key::ch('q')]],
        action: Action::Commit(Act::OpenQuote),
        hint: Some("gq quoted"),
        help: "open the quoted commit",
    },
    Binding {
        scope: Scope::Commit,
        chords: &[&[Key::ch('g'), Key::ch('r')]],
        action: Action::Commit(Act::Related),
        hint: None,
        help: "commits like this one",
    },
    Binding {
        scope: Scope::Confirm,
        chords: &[&[Key::ch('y')]],
        action: Action::Confirm,
        hint: None,
        help: "yes",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('1')]],
        action: Action::Pick(Reason::Spam),
        hint: Some("1-4 pick"),
        help: "spam",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('2')]],
        action: Action::Pick(Reason::Harassment),
        hint: None,
        help: "harassment",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('3')]],
        action: Action::Pick(Reason::Unsafe),
        hint: None,
        help: "unsafe",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('4')]],
        action: Action::Pick(Reason::Other),
        hint: None,
        help: "other",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('j')], &[Key::code(KeyCode::Down)]],
        action: Action::Down,
        hint: None,
        help: "move down",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::ch('k')], &[Key::code(KeyCode::Up)]],
        action: Action::Up,
        hint: None,
        help: "move up",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter next"),
        help: "next",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('1')]],
        action: Action::Choose(0),
        hint: Some("1-4 pick"),
        help: "option 1",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('2')]],
        action: Action::Choose(1),
        hint: None,
        help: "option 2",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('3')]],
        action: Action::Choose(2),
        hint: None,
        help: "option 3",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('4')]],
        action: Action::Choose(3),
        hint: None,
        help: "option 4",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('j')], &[Key::code(KeyCode::Down)]],
        action: Action::Down,
        hint: None,
        help: "move down",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::ch('k')], &[Key::code(KeyCode::Up)]],
        action: Action::Up,
        hint: None,
        help: "move up",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter vote"),
        help: "vote, which is final",
    },
    Binding {
        scope: Scope::Vote,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Viewer,
        chords: &[&[Key::ch('l')], &[Key::code(KeyCode::Right)]],
        action: Action::Next,
        hint: Some("h/l flip"),
        help: "next image",
    },
    Binding {
        scope: Scope::Viewer,
        chords: &[&[Key::ch('h')], &[Key::code(KeyCode::Left)]],
        action: Action::Previous,
        hint: None,
        help: "previous image",
    },
    Binding {
        scope: Scope::Viewer,
        chords: &[&[Key::ch('O')]],
        action: Action::Commit(Act::Browse),
        hint: Some("O browser"),
        help: "open the image in the browser",
    },
    Binding {
        scope: Scope::Viewer,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::ch('q')], &[Key::ch('i')]],
        action: Action::Close,
        hint: Some("esc close"),
        help: "close",
    },
    Binding {
        scope: Scope::Details,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Confirm,
        hint: Some("enter send"),
        help: "send",
    },
    Binding {
        scope: Scope::Report,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Details,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Details,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[
            &[Key::ctrl('s')],
            &[Key {
                code: KeyCode::Enter,
                ctrl: true,
            }],
        ],
        action: Action::Send,
        hint: Some("ctrl-s commit"),
        help: "commit",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('e')]],
        action: Action::Editor,
        hint: Some("ctrl-e editor"),
        help: "write it in $VISUAL or $EDITOR",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('p')]],
        action: Action::Poll,
        hint: Some("ctrl-p poll"),
        help: "add a poll, or take it off",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('t')]],
        action: Action::Schedule,
        hint: Some("ctrl-t later"),
        help: "send it later, at a time you type",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('x')]],
        action: Action::PostAs,
        hint: Some("ctrl-x as"),
        help: "post as you or, in turn, each organization you're a member of",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('v')]],
        action: Action::Attach,
        hint: Some("ctrl-v image"),
        help: "attach a copied image, or drop a file on the terminal",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('r')]],
        action: Action::Preview,
        hint: Some("ctrl-r preview"),
        help: "preview it as the commit it would become",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc close"),
        help: "close, keeping the draft",
    },
    Binding {
        scope: Scope::Composer,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Preview,
        chords: &[
            &[Key::ctrl('s')],
            &[Key {
                code: KeyCode::Enter,
                ctrl: true,
            }],
        ],
        action: Action::Send,
        hint: Some("ctrl-s commit"),
        help: "commit",
    },
    Binding {
        scope: Scope::Preview,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::ctrl('r')]],
        action: Action::Close,
        hint: Some("esc write"),
        help: "back to writing",
    },
    Binding {
        scope: Scope::Preview,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::code(KeyCode::Enter)], &[Key::ch('o')]],
        action: Action::Open,
        hint: Some("enter open"),
        help: "open the conversation, or one with the person",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('a')]],
        action: Action::Keep(Keep::Merge),
        hint: Some("a merge"),
        help: "merge the request, so you can reply",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('D')]],
        action: Action::Keep(Keep::Drop),
        hint: Some("D drop"),
        help: "drop the request, without telling them",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('s')]],
        action: Action::Keep(Keep::Stash),
        hint: Some("s stash"),
        help: "stash it until someone writes, or unstash it",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('m')]],
        action: Action::Keep(Keep::Mute),
        hint: Some("m mute"),
        help: "mute it for a while, or unmute it",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('^')]],
        action: Action::Keep(Keep::Pin),
        hint: None,
        help: "pin it to the top, or unpin it",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::code(KeyCode::Tab)]],
        action: Action::NextFilter,
        hint: Some("tab folder"),
        help: "next folder: open, requests, stashed",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::code(KeyCode::BackTab)]],
        action: Action::PreviousFilter,
        hint: None,
        help: "previous folder",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::ch('/')]],
        action: Action::Find,
        hint: Some("/ find"),
        help: "find a conversation, or someone to write to",
    },
    Binding {
        scope: Scope::Messages,
        chords: &[&[Key::code(KeyCode::Esc)], &[Key::code(KeyCode::Backspace)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back, or out of a find",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::Send,
        hint: Some("enter send"),
        help: "send",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::code(KeyCode::PageUp)]],
        action: Action::PageUp,
        hint: Some("pgup/pgdn scroll"),
        help: "scroll back a page, loading older messages at the top",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::code(KeyCode::PageDown)]],
        action: Action::PageDown,
        hint: None,
        help: "scroll forward a page",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::ctrl('e')]],
        action: Action::Editor,
        hint: Some("ctrl-e editor"),
        help: "write it in $VISUAL or $EDITOR",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Back,
        hint: Some("esc back"),
        help: "back, keeping what you typed",
    },
    Binding {
        scope: Scope::Conversation,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Mute,
        chords: &[&[Key::ch('h')]],
        action: Action::Hush(Hush::Hour),
        hint: None,
        help: "mute for an hour",
    },
    Binding {
        scope: Scope::Mute,
        chords: &[&[Key::ch('t')]],
        action: Action::Hush(Hush::Tomorrow),
        hint: None,
        help: "mute till tomorrow",
    },
    Binding {
        scope: Scope::Mute,
        chords: &[&[Key::ch('w')]],
        action: Action::Hush(Hush::Week),
        hint: None,
        help: "mute for a week",
    },
    Binding {
        scope: Scope::Mute,
        chords: &[&[Key::ch('f')]],
        action: Action::Hush(Hush::Forever),
        hint: None,
        help: "mute until you unmute it",
    },
    Binding {
        scope: Scope::Mute,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("esc cancel"),
        help: "cancel",
    },
    Binding {
        scope: Scope::Secret,
        chords: &[&[Key::ch('y')]],
        action: Action::Confirm,
        hint: Some("y commit anyway"),
        help: "commit anyway",
    },
    Binding {
        scope: Scope::Secret,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Rules,
        chords: &[&[Key::ch('O')]],
        action: Action::Site,
        hint: Some("O open dev-null.sh"),
        help: "open dev-null.sh to read the commit rules",
    },
    Binding {
        scope: Scope::Rules,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::Back,
        hint: Some("esc back to the draft"),
        help: "back to the draft",
    },
    Binding {
        scope: Scope::Rules,
        chords: &[&[Key::ctrl('c')]],
        action: Action::Quit,
        hint: None,
        help: "quit",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('g'), Key::ch('n')]],
        action: Action::Inbox,
        hint: None,
        help: "notifications",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('g'), Key::ch('s')]],
        action: Action::Starred,
        hint: None,
        help: "starred commits",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('g'), Key::ch('t')]],
        action: Action::Headlines,
        hint: None,
        help: "hot topics",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('g'), Key::ch('l')]],
        action: Action::Later,
        hint: None,
        help: "scheduled commits",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('g'), Key::ch('m')]],
        action: Action::Messages,
        hint: None,
        help: "messages",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('/')]],
        action: Action::Search,
        hint: None,
        help: "search commits, or @people",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ctrl('r')]],
        action: Action::Refresh,
        hint: None,
        help: "reload everything",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('z')]],
        action: Action::Saver,
        hint: None,
        help: "screensaver",
    },
    Binding {
        scope: Scope::Global,
        chords: &[&[Key::ch('?')]],
        action: Action::Help,
        hint: Some("? help"),
        help: "these keys",
    },
    Binding {
        scope: Scope::Help,
        chords: &[&[Key::ch('?')], &[Key::code(KeyCode::Esc)]],
        action: Action::Close,
        hint: Some("? close"),
        help: "close",
    },
    Binding {
        scope: Scope::Help,
        chords: &[&[Key::ch('j')], &[Key::code(KeyCode::Down)]],
        action: Action::Down,
        hint: Some("j/k scroll"),
        help: "scroll down",
    },
    Binding {
        scope: Scope::Help,
        chords: &[&[Key::ch('k')], &[Key::code(KeyCode::Up)]],
        action: Action::Up,
        hint: None,
        help: "scroll up",
    },
    Binding {
        scope: Scope::Gate,
        chords: &[&[Key::code(KeyCode::Enter)]],
        action: Action::SignIn,
        hint: Some("enter sign in"),
        help: "sign in",
    },
    Binding {
        scope: Scope::Gate,
        chords: &[&[Key::code(KeyCode::Esc)]],
        action: Action::StartOver,
        hint: Some("esc start over"),
        help: "start over",
    },
    Binding {
        scope: Scope::Gate,
        chords: &[&[Key::ch('i')]],
        action: Action::Icons,
        hint: Some("i switch icons"),
        help: "switch between Nerd Font icons and plain symbols",
    },
    Binding {
        scope: Scope::Splash,
        chords: &[&[Key::ctrl('r')]],
        action: Action::Refresh,
        hint: Some("ctrl-r retry now"),
        help: "retry now",
    },
    Binding {
        scope: Scope::Always,
        chords: &[&[Key::ch('Q')], &[Key::ctrl('c')]],
        action: Action::Quit,
        hint: Some("Q quit"),
        help: "quit",
    },
];

pub fn bindings<'a>(
    scopes: &'a [Scope],
    can: &'a dyn Fn(Action) -> bool,
) -> impl Iterator<Item = &'static Binding> + 'a {
    TABLE
        .iter()
        .filter(move |binding| scopes.contains(&binding.scope) && can(binding.action))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolve {
    Act(Action),
    Pending,
    Text(KeyEvent),
    Ignored,
}

#[derive(Default)]
pub struct Keymap {
    pending: Vec<Key>,
}

impl Keymap {
    pub fn pending(&self) -> &[Key] {
        &self.pending
    }

    pub fn resolve(
        &mut self,
        event: KeyEvent,
        mode: InputMode,
        scopes: &[Scope],
        can: &dyn Fn(Action) -> bool,
    ) -> Resolve {
        if event.kind != KeyEventKind::Press {
            return Resolve::Ignored;
        }
        let mut chord = std::mem::take(&mut self.pending);
        chord.push(Key::from(event));
        let mut longer = false;
        for binding in bindings(scopes, can) {
            for candidate in binding.chords {
                if *candidate == chord.as_slice() {
                    return Resolve::Act(binding.action);
                }
                longer |= candidate.starts_with(&chord);
            }
        }
        if longer {
            self.pending = chord;
            return Resolve::Pending;
        }
        if mode == InputMode::Text && chord.len() == 1 {
            return Resolve::Text(event);
        }
        Resolve::Ignored
    }
}

/// The first chord that does `action`, as help shows it.
pub fn key_for(action: Action) -> Option<String> {
    let binding = TABLE.iter().find(|binding| binding.action == action)?;
    Some(label(binding.chords.first()?))
}

pub fn label(chord: &[Key]) -> String {
    chord.iter().map(|key| key.label()).collect()
}

pub fn hints(scopes: &[Scope], pending: &[Key], can: &dyn Fn(Action) -> bool) -> String {
    if !pending.is_empty() {
        let rest: Vec<String> = bindings(scopes, can)
            .flat_map(|binding| {
                binding
                    .chords
                    .iter()
                    .filter(|chord| chord.len() > pending.len() && chord.starts_with(pending))
                    .map(|chord| format!("{} {}", label(&chord[pending.len()..]), binding.help))
            })
            .collect();
        return format!("{}-  {}", label(pending), rest.join(" · "));
    }
    bindings(scopes, can)
        .filter_map(|binding| binding.hint)
        .collect::<Vec<_>>()
        .join("  ")
}

const NEVER_DROPPED: [&str; 3] = ["? help", "? close", "esc close"];
const DROPPED_LAST: &str = "Q quit";

pub fn fit(hints: &str, width: usize) -> String {
    let mut hints: Vec<&str> = hints.split("  ").collect();
    while hints.join("  ").width() > width {
        let ordinary = |hint: &&str| !NEVER_DROPPED.contains(hint) && *hint != DROPPED_LAST;
        let Some(at) = hints
            .iter()
            .rposition(ordinary)
            .or_else(|| hints.iter().rposition(|hint| *hint == DROPPED_LAST))
        else {
            break;
        };
        hints.remove(at);
    }
    hints.join("  ")
}

/// Fits a pending chord's line, `g-  g top · r commits like this one · …`, by dropping whole
/// continuations from the end, so the line never ends mid-word.
pub fn fit_chords(hints: &str, width: usize) -> String {
    let Some((pending, rest)) = hints.split_once("-  ") else {
        return hints.to_owned();
    };
    let mut rest: Vec<&str> = rest.split(" · ").collect();
    let line = |rest: &[&str]| format!("{pending}-  {}", rest.join(" · "));
    while rest.len() > 1 && line(&rest).width() > width {
        rest.pop();
    }
    line(&rest)
}

/// Every key in `scopes`, each with whether it works right now, so help can show a key
/// like `v` dimmed instead of hiding it until a poll is selected.
pub fn help(scopes: &[Scope], can: &dyn Fn(Action) -> bool) -> Vec<Entry> {
    let mut listed: Vec<Entry> = Vec::new();
    let mut last: Option<&Binding> = None;
    for binding in TABLE
        .iter()
        .filter(|binding| scopes.contains(&binding.scope))
    {
        let keys: Vec<String> = binding.chords.iter().map(|chord| label(chord)).collect();
        let live = can(binding.action);
        // Neighbours that say the same thing share a row.
        match listed.last_mut() {
            Some(entry)
                if last.is_some_and(|l| l.scope == binding.scope && l.help == binding.help) =>
            {
                entry.keys.push('/');
                entry.keys.push_str(&keys.join("/"));
                entry.live |= live;
            }
            _ => listed.push(Entry {
                keys: keys.join("/"),
                help: binding.help,
                live,
                group: binding.action.group(),
            }),
        }
        last = Some(binding);
    }
    let mut shown: Vec<&'static str> = Vec::new();
    listed
        .iter()
        .filter(|entry| {
            let twin = listed
                .iter()
                .any(|other| other.help == entry.help && other.live);
            let keep = entry.live || (!twin && !shown.contains(&entry.help));
            if keep && !entry.live {
                shown.push(entry.help);
            }
            keep
        })
        .cloned()
        .collect()
}
