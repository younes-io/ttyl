use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use crate::model::{Log, Reason};

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
    Profile,
    NextFilter,
    PreviousFilter,
    Refresh,
    Help,
    Close,
    SignIn,
    StartOver,
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
    Attach,
    Site,
    Saver,
    Link(Link),
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
    Secret,
    Rules,
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
pub const SECRET_SCOPES: &[Scope] = &[Scope::Secret];
pub const RULES_SCOPES: &[Scope] = &[Scope::Rules];
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
    SECRET_SCOPES,
    RULES_SCOPES,
    LOG_SCOPES,
    THREAD_SCOPES,
    INBOX_SCOPES,
    GROUP_SCOPES,
    PEOPLE_SCOPES,
    PROFILE_SCOPES,
    PROFILE_PEOPLE_SCOPES,
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
        scope: Scope::Person,
        chords: &[&[Key::ch('f')]],
        action: Action::Commit(Act::Follow),
        hint: Some("f follow"),
        help: "follow or unfollow",
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
        scope: Scope::Commit,
        chords: &[&[Key::ch('f')]],
        action: Action::Commit(Act::Follow),
        hint: None,
        help: "follow or unfollow the author",
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
        chords: &[&[Key::ch('g'), Key::ch('q')]],
        action: Action::Commit(Act::OpenQuote),
        hint: Some("gq quoted"),
        help: "open the quoted commit",
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
        chords: &[&[Key::ctrl('v')]],
        action: Action::Attach,
        hint: Some("ctrl-v image"),
        help: "attach the image on the clipboard. dropping a file on the terminal works too",
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

/// Every key in `scopes`, each with whether it works right now, so help can show a key
/// like `v` dimmed instead of hiding it until a poll is selected.
pub fn help(scopes: &[Scope], can: &dyn Fn(Action) -> bool) -> Vec<(String, &'static str, bool)> {
    let mut listed: Vec<(String, &'static str, bool)> = Vec::new();
    let mut last: Option<&Binding> = None;
    for binding in TABLE
        .iter()
        .filter(|binding| scopes.contains(&binding.scope))
    {
        let keys: Vec<String> = binding.chords.iter().map(|chord| label(chord)).collect();
        let live = can(binding.action);
        // Neighbours that say the same thing share a row.
        match listed.last_mut() {
            Some((shown, _, on))
                if last.is_some_and(|l| l.scope == binding.scope && l.help == binding.help) =>
            {
                shown.push('/');
                shown.push_str(&keys.join("/"));
                *on |= live;
            }
            _ => listed.push((keys.join("/"), binding.help, live)),
        }
        last = Some(binding);
    }
    let mut shown: Vec<&'static str> = Vec::new();
    listed
        .iter()
        .filter(|(_, help, live)| {
            let twin = listed.iter().any(|(_, other, on)| other == help && *on);
            let keep = *live || (!twin && !shown.contains(help));
            if keep && !*live {
                shown.push(help);
            }
            keep
        })
        .cloned()
        .collect()
}
