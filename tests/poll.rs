mod common;

use std::collections::HashMap;
use std::time::Instant;

use common::{account, boot, contains, page, press, runs, screen, sign_in, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::Call;
use ttyl::attach::{Attachment, Duration, Poll};
use ttyl::draft::{Draft, Drafts, Target, Write};
use ttyl::engine::{Effect, Engine, Event};

const HEIGHT: u16 = 30;

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

/// A new commit asking `question`, with a poll open on its first option.
fn polling(question: &str, at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    press(&mut engine, question, at);
    ctrl(&mut engine, 'p', at);
    engine
}

/// Types each option, moving on to the next with enter.
fn options(engine: &mut Engine, options: &[&str], at: Instant) {
    for (at_option, option) in options.iter().enumerate() {
        if at_option > 0 {
            tap(engine, KeyCode::Enter, at);
        }
        press(engine, option, at);
    }
}

fn panel(engine: &mut Engine, at: Instant) -> Vec<String> {
    let rows = screen(engine, 100, HEIGHT, at);
    let title = rows
        .iter()
        .position(|row| row.starts_with(" commit ─"))
        .unwrap_or_else(|| panic!("the composer is open: {rows:#?}"));
    rows[title..]
        .iter()
        .map(|row| row.trim_end().to_owned())
        .collect()
}

fn posts(effects: &[Effect]) -> Vec<&Call> {
    runs(effects)
        .into_iter()
        .filter(|call| matches!(call, Call::Post { .. }))
        .collect()
}

#[test]
fn ctrl_p_opens_a_poll_under_the_question() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    assert_eq!(
        panel(&mut engine, at)[1..6],
        [
            " which editor?",
            "",
            " ○ option 1",
            " ○ option 2",
            "   lasts ‹ 1 day ›",
        ],
    );
}

#[test]
fn a_poll_goes_out_with_its_options_and_how_long_it_lasts() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    options(&mut engine, &["vim", " emacs "], at);
    tap(&mut engine, KeyCode::Tab, at);
    tap(&mut engine, KeyCode::Right, at);
    tap(&mut engine, KeyCode::Right, at);
    assert!(contains(&panel(&mut engine, at), "lasts ‹ 7 days ›"));
    let effects = ctrl(&mut engine, 's', at);
    assert_eq!(
        posts(&effects),
        [&Call::Post {
            target: Target::New,
            body: "which editor?".to_owned(),
            attachment: Attachment::Poll(Poll {
                options: vec!["vim".to_owned(), "emacs".to_owned()],
                duration: Duration::Week,
            }),
        }],
    );
}

#[test]
fn enter_adds_options_up_to_four_and_backspace_takes_an_empty_one_away() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    options(&mut engine, &["vim", "emacs", "helix", "zed"], at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "○ zed"), "{rows:#?}");
    tap(&mut engine, KeyCode::Enter, at);
    let rows = panel(&mut engine, at);
    assert!(!contains(&rows, "option 5"), "four is the most: {rows:#?}");
    assert!(contains(&rows, "← → change"), "enter moved on: {rows:#?}");
    tap(&mut engine, KeyCode::BackTab, at);
    for _ in 0.."zed".len() + 1 {
        tap(&mut engine, KeyCode::Backspace, at);
    }
    let rows = panel(&mut engine, at);
    assert!(!contains(&rows, "zed"), "{rows:#?}");
    assert!(!contains(&rows, "option 4"), "{rows:#?}");
    assert!(contains(&rows, "○ helix"), "{rows:#?}");
}

#[test]
fn a_poll_keeps_two_options_however_many_are_emptied() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    for _ in 0..3 {
        tap(&mut engine, KeyCode::Backspace, at);
    }
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "○ option 2"), "{rows:#?}");
}

#[test]
fn left_and_right_walk_the_durations_round() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    tap(&mut engine, KeyCode::Tab, at);
    tap(&mut engine, KeyCode::Tab, at);
    for (key, shown) in [
        (KeyCode::Left, "1 hour"),
        (KeyCode::Left, "7 days"),
        (KeyCode::Right, "1 hour"),
        (KeyCode::Char(' '), "1 day"),
        (KeyCode::Enter, "3 days"),
    ] {
        tap(&mut engine, key, at);
        let rows = panel(&mut engine, at);
        assert!(contains(&rows, &format!("lasts ‹ {shown} ›")), "{rows:#?}");
    }
}

#[test]
fn a_poll_the_server_would_refuse_never_leaves() {
    let at = Instant::now();
    for (typed, refusal) in [
        (&["vim"][..], "a poll needs at least two options."),
        (&["vim", "Vim "][..], "options 1 and 2 are the same."),
    ] {
        let mut engine = polling("which editor?", at);
        options(&mut engine, typed, at);
        let effects = ctrl(&mut engine, 's', at);
        assert!(posts(&effects).is_empty(), "{typed:?}: {effects:?}");
        let rows = panel(&mut engine, at);
        assert!(contains(&rows, refusal), "{typed:?}: {rows:#?}");
    }
}

#[test]
fn only_a_new_commit_can_carry_a_poll() {
    let at = Instant::now();
    for key in ["r", "q"] {
        let mut engine = boot(at);
        press(&mut engine, "2jj", at);
        press(&mut engine, key, at);
        ctrl(&mut engine, 'p', at);
        let rows = screen(&mut engine, 100, HEIGHT, at);
        assert!(!contains(&rows, "option 1"), "{key}: {rows:#?}");
    }
}

#[test]
fn ctrl_p_takes_the_poll_off_and_brings_the_same_one_back() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    options(&mut engine, &["vim", "emacs"], at);
    ctrl(&mut engine, 'p', at);
    let rows = panel(&mut engine, at);
    assert!(!contains(&rows, "○ vim"), "{rows:#?}");
    assert!(
        contains(&rows, "poll removed · ctrl-p brings it back"),
        "{rows:#?}"
    );
    press(&mut engine, "!", at);
    assert!(contains(&panel(&mut engine, at), " which editor?!"));
    ctrl(&mut engine, 'p', at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "○ vim"), "{rows:#?}");
    assert!(contains(&rows, "○ emacs"), "{rows:#?}");
}

#[test]
fn the_draft_keeps_the_poll_and_brings_it_back() {
    let at = Instant::now();
    let mut engine = polling("which editor?", at);
    let effects = press(&mut engine, "vim", at);
    let poll = Poll {
        options: vec!["vim".to_owned(), String::new()],
        duration: Duration::Day,
    };
    let draft = Draft {
        text: "which editor?".to_owned(),
        attachment: Attachment::Poll(poll),
    };
    assert!(
        effects.contains(&Effect::Draft(Write::Save(Target::New, draft.clone()))),
        "{effects:?}"
    );
    let saved = HashMap::from([(Target::New, draft)]);
    let engine = Engine::new("ttyl".to_owned(), at).with_drafts(Drafts::new(saved));
    let mut engine = sign_in(engine, at, account(3), page);
    press(&mut engine, "c", at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "○ vim"), "{rows:#?}");
    assert!(contains(&rows, "○ option 2"), "{rows:#?}");
}
