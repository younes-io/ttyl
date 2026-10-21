mod common;

use std::time::Instant;

use common::{boot, contains, page, press, screen, tap};
use devnull::engine::Engine;
use devnull::model::Log;
use ratatui::crossterm::event::KeyCode;

fn on_latest(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jjj", at);
    engine
}

#[test]
fn back_returns_to_the_same_tab_selection_and_scroll() {
    let at = Instant::now();
    let mut engine = on_latest(at);
    press(&mut engine, "jjjj", at);
    let before = screen(&mut engine, 100, 24, at);
    let selected = engine.log().unwrap().selected();

    for (open, back) in [
        (KeyCode::Enter, KeyCode::Esc),
        (KeyCode::Char('o'), KeyCode::Backspace),
    ] {
        tap(&mut engine, open, at);
        let thread = engine.thread().expect("enter opens a thread");
        let rows = page(Log::Latest).rows;
        assert_eq!(*thread.focus(), rows[selected].commit.id);
        assert_ne!(screen(&mut engine, 100, 24, at), before);

        tap(&mut engine, back, at);
        assert!(engine.thread().is_none());
        assert_eq!(engine.log().unwrap().active(), Log::Latest);
        assert_eq!(engine.log().unwrap().selected(), selected);
        assert_eq!(screen(&mut engine, 100, 24, at), before);
    }
}

#[test]
fn the_thread_hint_names_its_keys_and_keeps_help_when_narrow() {
    let at = Instant::now();
    let mut engine = on_latest(at);
    tap(&mut engine, KeyCode::Enter, at);
    let rows = screen(&mut engine, 100, 24, at);
    assert_eq!(rows[23], " j/k move  enter open  esc back  ? help  Q quit");
    let rows = screen(&mut engine, 30, 24, at);
    assert_eq!(rows[23], " j/k move  ? help  Q quit", "{rows:#?}");
    let rows = screen(&mut engine, 12, 24, at);
    assert_eq!(rows[23], " ? help", "{rows:#?}");
}

#[test]
fn help_on_a_thread_lists_the_thread_keys() {
    let at = Instant::now();
    let mut engine = on_latest(at);
    tap(&mut engine, KeyCode::Enter, at);
    press(&mut engine, "?", at);
    let rows = screen(&mut engine, 100, 40, at);
    for shown in [
        "move down",
        "open this commit's thread",
        "esc/backspace  back",
        "following",
        "next log",
        "these keys",
        "quit",
    ] {
        assert!(contains(&rows, shown), "{shown}: {rows:#?}");
    }
    for hidden in ["open the thread", "refresh this log"] {
        assert!(!contains(&rows, hidden), "{hidden}: {rows:#?}");
    }
    press(&mut engine, "?", at);
    assert!(engine.thread().is_some(), "closing help keeps the thread");
}

#[test]
fn log_keys_on_a_thread_go_back_to_the_log_and_switch() {
    let at = Instant::now();
    for (keys, log) in [
        ("1", Log::Following),
        ("2", Log::Latest),
        ("3", Log::Trending),
        ("\t", Log::Trending),
    ] {
        let mut engine = on_latest(at);
        tap(&mut engine, KeyCode::Enter, at);
        match keys {
            "\t" => tap(&mut engine, KeyCode::Tab, at),
            keys => press(&mut engine, keys, at),
        };
        assert!(engine.thread().is_none(), "{keys:?}");
        assert_eq!(engine.log().unwrap().active(), log, "{keys:?}");
        press(&mut engine, "2", at);
        assert_eq!(engine.log().unwrap().selected(), 3, "{keys:?}");
    }
}
