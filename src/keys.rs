use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_width::UnicodeWidthStr;

use crate::model::Log;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Down,
    Up,
    Top,
    Bottom,
    Expand,
    Show(Log),
    NextLog,
    Refresh,
    Help,
    Close,
    SignIn,
    StartOver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Always,
    Splash,
    Gate,
    Help,
    Log,
    List,
    Global,
}

pub const SPLASH_SCOPES: &[Scope] = &[Scope::Splash, Scope::Always];
pub const GATE_SCOPES: &[Scope] = &[Scope::Gate, Scope::Always];
pub const HELP_SCOPES: &[Scope] = &[Scope::Help, Scope::Always];
pub const LOG_SCOPES: &[Scope] = &[Scope::List, Scope::Log, Scope::Global, Scope::Always];
pub const UNIONS: &[&[Scope]] = &[SPLASH_SCOPES, GATE_SCOPES, HELP_SCOPES, LOG_SCOPES];

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
        chords: &[&[Key::ch(' ')]],
        action: Action::Expand,
        hint: Some("space expand"),
        help: "expand the selected commit",
    },
    Binding {
        scope: Scope::Log,
        chords: &[&[Key::ch('1')]],
        action: Action::Show(Log::Following),
        hint: Some("1-3 log"),
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
        scope: Scope::Log,
        chords: &[&[Key::ch('R')]],
        action: Action::Refresh,
        hint: Some("R refresh"),
        help: "refresh this log",
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
        chords: &[&[Key::ch('R')]],
        action: Action::Refresh,
        hint: Some("R retry now"),
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

/// Drops hints from the end until they fit `width`. `? help` is how a user finds every
/// other key, so it goes last; `Q quit` goes just before it.
pub fn fit(hints: &str, width: usize) -> String {
    let rank = |hint: &&str| match *hint {
        hint if hint.starts_with("? ") => 2,
        "Q quit" => 1,
        _ => 0,
    };
    let mut hints: Vec<&str> = hints.split("  ").collect();
    while hints.len() > 1 && hints.join("  ").width() > width {
        let lowest = hints.iter().map(rank).min().expect("at least two hints");
        let at = hints
            .iter()
            .rposition(|hint| rank(hint) == lowest)
            .expect("the lowest rank is held by a hint");
        hints.remove(at);
    }
    hints.join("  ")
}

pub fn help(scopes: &[Scope], can: &dyn Fn(Action) -> bool) -> Vec<(String, &'static str)> {
    bindings(scopes, can)
        .map(|binding| {
            let keys: Vec<String> = binding.chords.iter().map(|chord| label(chord)).collect();
            (keys.join("/"), binding.help)
        })
        .collect()
}
