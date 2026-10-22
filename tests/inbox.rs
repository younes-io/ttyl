mod common;

use std::time::Instant;

use common::{boot, contains, press, screen, tap};
use ratatui::crossterm::event::KeyCode;

#[test]
fn gn_opens_the_inbox_and_esc_returns_to_the_same_place() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("notifications"), "{rows:#?}");
    assert!(!rows[0].contains("latest"), "{rows:#?}");
    assert_eq!(rows[1], " all  replies  quotes  stars  mentions  follows");
    assert_eq!(rows[29], " tab filter  esc back  ? help  Q quit");

    press(&mut engine, "gn", at);
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(
        screen(&mut engine, 100, 30, at),
        before,
        "one esc undoes gn"
    );
}

#[test]
fn esc_from_the_inbox_returns_to_the_thread_it_was_opened_from() {
    let at = Instant::now();
    let mut engine = boot(at);
    common::serve(&mut engine, at);
    tap(&mut engine, KeyCode::Enter, at);
    common::serve(&mut engine, at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    assert!(engine.thread().is_none());
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(screen(&mut engine, 100, 30, at), before);
}

#[test]
fn tab_and_shift_tab_cycle_the_filters_and_wrap() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let bold = |engine: &mut devnull::engine::Engine| {
        let buffer = common::buffer(engine, 100, 30, at);
        (0..100u16)
            .filter(|x| {
                buffer[(*x, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
            .map(|x| buffer[(x, 1)].symbol().to_owned())
            .collect::<String>()
    };
    assert_eq!(bold(&mut engine), "all");
    tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(bold(&mut engine), "replies");
    tap(&mut engine, KeyCode::BackTab, at);
    tap(&mut engine, KeyCode::BackTab, at);
    assert_eq!(bold(&mut engine), "follows");
}

#[test]
fn the_narrow_filter_bar_and_hints_fit_44_columns() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 44, 20, at);
    assert_eq!(rows[1], " all replies quotes stars mentions follows");
    assert!(contains(&rows[19..], "? help"), "{rows:#?}");
}
