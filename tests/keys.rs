use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::keys::{
    Act, Action, GATE_SCOPES, HELP_SCOPES, InputMode, Keymap, LOG_SCOPES, Resolve, Scope, TABLE,
    UNIONS, help, hints,
};
use ttyl::model::Log;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn all(_: Action) -> bool {
    true
}

#[test]
fn no_chord_is_a_prefix_of_another_chord_that_is_active_with_it() {
    for scopes in UNIONS {
        let chords: Vec<_> = TABLE
            .iter()
            .filter(|binding| scopes.contains(&binding.scope))
            .flat_map(|binding| {
                binding
                    .chords
                    .iter()
                    .map(move |chord| (chord, binding.action))
            })
            .collect();
        for (a, action_a) in &chords {
            for (b, action_b) in &chords {
                if std::ptr::eq(*a, *b) {
                    continue;
                }
                assert!(
                    !b.starts_with(a),
                    "{action_a:?} {a:?} is a prefix of {action_b:?} {b:?} in {scopes:?}"
                );
            }
        }
    }
}

#[test]
fn g_then_g_goes_to_the_top_and_g_then_another_key_cancels() {
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('g')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Pending
    );
    assert_eq!(
        hints(LOG_SCOPES, keymap.pending(), &all),
        "g-  g top · q open the quoted commit · n notifications · s starred commits"
    );
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('g')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Act(Action::Top)
    );
    assert!(keymap.pending().is_empty());

    keymap.resolve(
        key(KeyCode::Char('g')),
        InputMode::Commands,
        LOG_SCOPES,
        &all,
    );
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('x')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Ignored
    );
    assert!(keymap.pending().is_empty());
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('g')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Pending,
        "the cancelled chord does not count toward the next"
    );
}

#[test]
fn a_binding_the_screen_cannot_do_is_not_dispatched_listed_or_hinted() {
    let no_star = |action: Action| action != Action::Commit(Act::Star);
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('s')),
            InputMode::Commands,
            LOG_SCOPES,
            &no_star
        ),
        Resolve::Ignored
    );
    assert!(!hints(LOG_SCOPES, &[], &no_star).contains("s star"));
    assert!(hints(LOG_SCOPES, &[], &all).contains("s star"));
    let listed = help(LOG_SCOPES, &no_star);
    let live = |what: &str| listed.iter().find(|row| row.1 == what).map(|row| row.2);
    assert_eq!(live("star or unstar"), Some(false));
    assert_eq!(live("follow or unfollow the author"), Some(true));
}

#[test]
fn the_question_mark_opens_help_and_closes_it() {
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('?')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Act(Action::Help)
    );
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('?')),
            InputMode::Commands,
            HELP_SCOPES,
            &all
        ),
        Resolve::Act(Action::Close)
    );
    assert_eq!(
        keymap.resolve(key(KeyCode::Esc), InputMode::Commands, HELP_SCOPES, &all),
        Resolve::Act(Action::Close)
    );
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('j')),
            InputMode::Commands,
            HELP_SCOPES,
            &all
        ),
        Resolve::Act(Action::Down),
        "j scrolls the keys, not the list under them"
    );
}

#[test]
fn quit_works_everywhere_and_digits_pick_a_log() {
    let mut keymap = Keymap::default();
    for scopes in UNIONS {
        assert_eq!(
            keymap.resolve(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                InputMode::Commands,
                scopes,
                &all
            ),
            Resolve::Act(Action::Quit)
        );
    }
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Char('3')),
            InputMode::Commands,
            LOG_SCOPES,
            &all
        ),
        Resolve::Act(Action::Show(Log::Trending))
    );
    assert_eq!(
        keymap.resolve(key(KeyCode::Enter), InputMode::Commands, GATE_SCOPES, &all),
        Resolve::Act(Action::SignIn)
    );
    assert_eq!(
        keymap.resolve(
            key(KeyCode::Enter),
            InputMode::Commands,
            &[Scope::Always],
            &all
        ),
        Resolve::Ignored
    );
}

#[test]
fn an_unbound_key_reaches_a_text_field() {
    let mut keymap = Keymap::default();
    let event = key(KeyCode::Char('a'));
    assert_eq!(
        keymap.resolve(event, InputMode::Text, GATE_SCOPES, &all),
        Resolve::Text(event)
    );
    assert_eq!(
        keymap.resolve(event, InputMode::Commands, GATE_SCOPES, &all),
        Resolve::Ignored
    );
}
