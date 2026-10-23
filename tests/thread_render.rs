mod common;

use std::time::{Duration, Instant};

use common::{answer, boot_serving, buffer, contains, older, press, screen, tap, world_log};
use ttyl::engine::{Effect, Engine};
use ttyl::model::{Log, LogPage};
use ttyl::theme;
use ratatui::crossterm::event::KeyCode;

const ADA: &str = "9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61";
const DEEPEST: &str = "7a000000-0000-4000-8000-000000000066";
const UNDER_LONG: &str = "7a000000-0000-4000-8000-000000000012";
const HEIGHT: u16 = 40;
const WIDTHS: [u16; 2] = [100, 44];

fn pages(id: &str) -> fn(Log) -> LogPage {
    match id {
        ADA => |log| LogPage {
            log,
            ..world_log(&[ADA])
        },
        DEEPEST => |log| LogPage {
            log,
            ..world_log(&[DEEPEST])
        },
        UNDER_LONG => |log| LogPage {
            log,
            ..world_log(&[UNDER_LONG])
        },
        _ => |log| older(log, "gone", 1, None),
    }
}

fn settle(engine: &mut Engine, effects: Vec<Effect>, width: u16, at: Instant) {
    answer(engine, effects, at);
    loop {
        screen(engine, width, HEIGHT, at);
        let effects = engine.pump(at);
        if effects.is_empty() {
            return;
        }
        answer(engine, effects, at);
    }
}

fn opened(id: &str, width: u16) -> (Engine, Instant) {
    let at = Instant::now();
    let mut engine = boot_serving(at, pages(id));
    let effects = tap(&mut engine, KeyCode::Enter, at);
    settle(&mut engine, effects, width, at);
    (engine, at)
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is on screen: {rows:#?}"))
}

#[test]
fn ancestors_are_dimmed_and_cut_to_two_lines() {
    for width in WIDTHS {
        let (mut engine, at) = opened(UNDER_LONG, width);
        let rows = screen(&mut engine, width, HEIGHT, at);
        let header = row_of(&rows, "Margaret @margaret");
        assert_eq!(rows[header + 1], " │ notes from rewriting our scheduler.");
        assert_eq!(rows[header + 2], " │ …");
        assert_eq!(rows[header + 3], " │");
        assert!(rows[header + 4].starts_with(" ● Ada @ada"), "{rows:#?}");

        let cells = buffer(&mut engine, width, HEIGHT, at);
        let header = u16::try_from(header).unwrap();
        let focus = header + 4;
        assert_eq!(cells[(3, header)].fg, theme::MUTED_FOREGROUND);
        assert_eq!(cells[(3, header + 1)].fg, theme::MUTED_FOREGROUND);
        assert_ne!(cells[(3, focus)].fg, theme::MUTED_FOREGROUND);
    }
}

#[test]
fn a_cut_chain_starts_with_a_line_that_opens_earlier_commits() {
    for width in WIDTHS {
        let (mut engine, at) = opened(DEEPEST, width);
        assert!(!contains(
            &screen(&mut engine, width, HEIGHT, at),
            "earlier commits"
        ));
        press(&mut engine, "gg", at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert_eq!(rows[2], " ⋯ earlier commits · enter to open", "{rows:#?}");
        assert_eq!(rows[3], " │");
        assert!(rows[4].starts_with(" ● Margaret @margaret"), "{rows:#?}");
        assert_eq!(rows[5], " │ step 5 of 25 down this chain.");
        assert_eq!(rows[6], " │");
    }
}

#[test]
fn the_focused_commit_ends_with_its_local_time_and_hash() {
    for width in WIDTHS {
        let (mut engine, at) = opened(ADA, width);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(
            contains(&rows, " ┃ 9:42 AM · Oct 21, 2026 · commit e4f5a61"),
            "{rows:#?}"
        );
    }
}

#[test]
fn replies_indent_per_level_and_lie_flat_below_the_third() {
    for width in WIDTHS {
        let (mut engine, at) = opened(ADA, width);
        let rows = screen(&mut engine, width, HEIGHT, at);
        for (depth, hash) in [(1, "0000001"), (2, "000000e"), (3, "000000f")] {
            let row = &rows[row_of(&rows, hash)];
            assert!(
                row.starts_with(&format!(" {}● ", " ".repeat(2 * depth))),
                "{row}"
            );
        }
        for (hash, parent) in [("0000010", "@grace"), ("0000011", "@ada")] {
            let at = row_of(&rows, hash);
            assert!(rows[at].starts_with("       ● "), "{}", rows[at]);
            assert_eq!(rows[at + 1], format!("       │ ↳ replying to {parent}"));
        }
        assert!(!rows[row_of(&rows, "000000f") + 1].contains('↳'));
    }
}

#[test]
fn a_quote_is_a_reply_marked_quoted() {
    for width in WIDTHS {
        let (mut engine, at) = opened(ADA, width);
        press(&mut engine, "G", at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(
            rows[row_of(&rows, "000000a")].starts_with("   ● Ken @ken · now · quoted"),
            "{rows:#?}"
        );
        assert!(!rows[row_of(&rows, "0000009")].contains("quoted"));
    }
}

#[test]
fn a_deleted_reply_is_one_line_and_its_replies_stay() {
    for width in WIDTHS {
        let (mut engine, at) = opened(ADA, width);
        press(&mut engine, "G", at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        let gone = row_of(&rows, "fatal: bad object 000000b");
        assert_eq!(rows[gone], "   ○ fatal: bad object 000000b");
        assert_eq!(rows[gone + 1], "   │");
        assert!(
            rows[gone + 2].starts_with("     ● Grace @grace"),
            "{rows:#?}"
        );
    }
}

#[test]
fn a_deleted_focus_says_so_and_still_lists_its_replies() {
    for width in WIDTHS {
        let (mut engine, at) = opened(ADA, width);
        press(&mut engine, "Gkkk", at);
        assert!(engine.thread().unwrap().selected().0.ends_with("00b"));
        let effects = tap(&mut engine, KeyCode::Enter, at);
        settle(&mut engine, effects, width, at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert_eq!(rows[2], " ○ fatal: bad object 000000b", "{rows:#?}");
        assert_eq!(rows[3], " ┃ This commit is no longer available.");
        assert_eq!(rows[4], " │");
        assert!(rows[5].starts_with("   ● Grace @grace"), "{rows:#?}");
    }
}

#[test]
fn a_commit_gone_on_open_shows_only_the_fatal_lines() {
    for width in WIDTHS {
        let (mut engine, at) = opened("gone-0", width);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert_eq!(rows[2], " ○ fatal: bad object gone0", "{rows:#?}");
        assert_eq!(rows[3], " ┃ This commit is no longer available.");
        assert!(
            rows[4..rows.len() - 1].iter().all(String::is_empty),
            "{rows:#?}"
        );
    }
}

#[test]
fn the_loading_line_waits_three_hundred_milliseconds() {
    for width in WIDTHS {
        let at = Instant::now();
        let mut engine = boot_serving(at, pages(ADA));
        let effects = tap(&mut engine, KeyCode::Enter, at);
        for wait in [0, 100, 299] {
            let rows = screen(&mut engine, width, HEIGHT, at + Duration::from_millis(wait));
            assert!(rows[2].starts_with(" ● Ada @ada"), "{rows:#?}");
            assert!(!contains(&rows, "loading replies"), "{wait}ms: {rows:#?}");
        }
        let late = at + Duration::from_millis(300);
        let rows = screen(&mut engine, width, HEIGHT, late);
        assert!(contains(&rows, "   loading replies"), "{rows:#?}");

        answer(&mut engine, effects, late);
        let rows = screen(&mut engine, width, HEIGHT, late);
        assert!(!contains(&rows, "loading replies"), "{rows:#?}");
        assert!(contains(&rows, "Grace @grace"), "{rows:#?}");
    }
}

#[test]
fn a_nested_list_shows_its_count_until_it_lands() {
    for width in WIDTHS {
        let at = Instant::now();
        let mut engine = boot_serving(at, pages(ADA));
        let effects = tap(&mut engine, KeyCode::Enter, at);
        answer(&mut engine, effects, at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        let grace = row_of(&rows, "0000001");
        assert!(contains(&rows[grace..], "     ⋯ 1 reply"), "{rows:#?}");
        assert!(!contains(&rows, "000000e"));

        settle(&mut engine, Vec::new(), width, at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(!contains(&rows, "⋯ 1 reply"), "{rows:#?}");
        assert!(contains(&rows, "000000e"));
    }
}
