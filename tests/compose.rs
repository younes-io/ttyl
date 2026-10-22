mod common;

use std::collections::HashMap;
use std::time::Instant;

use common::{account, boot, contains, page, press, screen, sign_in, tap};
use devnull::draft::{Drafts, Target, Write};
use devnull::engine::{Effect, Engine, Event};
use devnull::keys;
use devnull::model::CommitId;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";
const WIDTHS: [u16; 2] = [100, 44];
const HEIGHT: u16 = 30;

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

fn on_grace(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    engine
}

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    )
}

fn saved(effects: &[Effect]) -> Vec<&Write> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Draft(write) => Some(write),
            _ => None,
        })
        .collect()
}

fn title_row(rows: &[String], title: &str) -> usize {
    let title = format!(" {title} ─");
    rows.iter()
        .position(|row| row.starts_with(&title))
        .unwrap_or_else(|| panic!("the panel is titled {title:?}: {rows:#?}"))
}

#[test]
fn c_r_and_q_open_the_panel_on_their_target_with_its_title() {
    let at = Instant::now();
    for (key, title, target) in [
        ("c", "commit", Target::New),
        ("r", "Reply to @grace", Target::Reply(grace())),
        ("q", "Quote @grace", Target::Quote(grace())),
    ] {
        let mut engine = on_grace(at);
        press(&mut engine, key, at);
        let effects = press(&mut engine, "x", at);
        assert_eq!(
            saved(&effects),
            [&Write::Save(target.clone(), "x".to_owned())],
            "{key}"
        );
        for width in WIDTHS {
            let rows = screen(&mut engine, width, HEIGHT, at);
            let panel = title_row(&rows, title);
            assert_eq!(rows[panel + 1], " x", "{key} at {width}: {rows:#?}");
            assert_eq!(
                usize::from(HEIGHT) - panel,
                6,
                "a title, 3 text lines, a status line, then the hint: {rows:#?}"
            );
            let grace_row = rows.iter().position(|row| row.contains("@grace"));
            assert!(
                grace_row.is_some_and(|row| row < panel),
                "{key} at {width} keeps the target above the panel: {rows:#?}"
            );
            assert_eq!(
                rows[usize::from(HEIGHT) - 1],
                " ctrl-s commit  ctrl-e editor  esc close",
                "{rows:#?}"
            );
        }
        assert_eq!(
            engine.log().unwrap().selected(),
            2,
            "{key} keeps the target selected"
        );
    }
}

#[test]
fn r_and_q_open_on_the_selected_commit_in_a_thread() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    common::answer(&mut engine, effects, at);
    let selected = engine.thread().unwrap().selected().clone();
    press(&mut engine, "r", at);
    let effects = press(&mut engine, "y", at);
    assert_eq!(
        saved(&effects),
        [&Write::Save(Target::Reply(selected), "y".to_owned())]
    );
    let rows = screen(&mut engine, 100, HEIGHT, at);
    title_row(&rows, "Reply to @grace");
}

#[test]
fn r_and_q_are_not_offered_on_a_commit_being_deleted() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "Dy", at);
    for key in ["r", "q"] {
        press(&mut engine, key, at);
        let rows = screen(&mut engine, 100, HEIGHT, at);
        assert!(
            !contains(&rows, "Reply to") && !contains(&rows, "Quote @"),
            "{rows:#?}"
        );
    }
    press(&mut engine, "c", at);
    title_row(&screen(&mut engine, 100, HEIGHT, at), "commit");
}

#[test]
fn every_letter_types_itself_and_each_edit_saves_the_draft_once() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    let mut text = String::new();
    for ch in "Qjk?gGRsD1 ".chars() {
        text.push(ch);
        let effects = press(&mut engine, &ch.to_string(), at);
        assert_eq!(
            effects,
            [Effect::Draft(Write::Save(Target::New, text.clone()))]
        );
    }
    assert!(engine.thread().is_none() && engine.log().unwrap().selected() == 2);
    for code in [KeyCode::Left, KeyCode::Home, KeyCode::Up, KeyCode::End] {
        assert!(
            tap(&mut engine, code, at).is_empty(),
            "{code:?} edits nothing"
        );
    }
    assert_eq!(
        tap(&mut engine, KeyCode::Enter, at),
        [Effect::Draft(Write::Save(Target::New, format!("{text}\n")))]
    );
    press(&mut engine, "two words", at);
    assert_eq!(
        ctrl(&mut engine, 'u', at),
        [Effect::Draft(Write::Save(Target::New, format!("{text}\n")))],
        "ctrl-u deletes to the start of the line, not past it"
    );
    assert!(ctrl(&mut engine, 'u', at).is_empty());
}

#[test]
fn a_draft_emptied_by_hand_is_deleted() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "cab", at);
    assert_eq!(
        ctrl(&mut engine, 'w', at),
        [Effect::Draft(Write::Drop(Target::New))]
    );
    assert_eq!(engine.draft(&Target::New), None);
    assert!(tap(&mut engine, KeyCode::Backspace, at).is_empty());
}

#[test]
fn esc_keeps_the_draft_and_reopening_restores_it_with_the_stash_line() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "rhello", at);
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(
        !contains(&rows, "Reply to"),
        "esc closes the panel: {rows:#?}"
    );
    assert_eq!(engine.draft(&Target::Reply(grace())), Some("hello"));

    press(&mut engine, "c", at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(!contains(&rows, "hello"), "a new commit has its own draft");
    assert!(!contains(&rows, "stash@{0}"));
    tap(&mut engine, KeyCode::Esc, at);

    press(&mut engine, "r", at);
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        let panel = title_row(&rows, "Reply to @grace");
        assert_eq!(rows[panel + 1], " hello");
        assert_eq!(
            rows[panel + 4],
            " stash@{0}: restored your unsent draft",
            "{rows:#?}"
        );
    }
    press(&mut engine, "!", at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, " hello!"));
    assert!(
        !contains(&rows, "stash@{0}"),
        "the stash line goes on the first edit"
    );
}

#[test]
fn a_draft_saved_on_disk_comes_back_on_its_target() {
    let at = Instant::now();
    let saved = HashMap::from([(Target::Quote(grace()), "from last time".to_owned())]);
    let engine = Engine::new("devnull-tui".to_owned(), at).with_drafts(Drafts::new(saved));
    let mut engine = sign_in(engine, at, account(3), page);
    press(&mut engine, "2jjq", at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    let panel = title_row(&rows, "Quote @grace");
    assert_eq!(rows[panel + 1], " from last time");
    assert!(contains(&rows, "stash@{0}: restored your unsent draft"));
}

#[test]
fn ctrl_c_quits_from_the_composer_and_q_types() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    assert_eq!(
        saved(&press(&mut engine, "Q", at)),
        [&Write::Save(Target::New, "Q".to_owned())]
    );
    assert_eq!(ctrl(&mut engine, 'c', at), [Effect::Quit]);
}

#[test]
fn a_paste_arrives_whole_and_the_panel_grows_then_scrolls_to_the_cursor() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    let pasted: Vec<String> = (1..=20).map(|line| format!("line {line}")).collect();
    let effects = engine.update(Event::Paste(pasted.join("\r\n")), at);
    assert_eq!(
        effects,
        [Effect::Draft(Write::Save(Target::New, pasted.join("\n")))]
    );
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        let panel = title_row(&rows, "commit");
        assert_eq!(
            usize::from(HEIGHT) - panel,
            HEIGHT as usize / 2 + 1,
            "the panel stops at half the screen: {rows:#?}"
        );
        assert_eq!(rows[usize::from(HEIGHT) - 3], " line 20", "{rows:#?}");
        assert!(!contains(&rows, "line 1\n") && !rows.iter().any(|row| row == " line 1"));
        assert!(contains(&rows[..panel], "@grace"), "{rows:#?}");
    }
    for _ in 0..19 {
        tap(&mut engine, KeyCode::Up, at);
    }
    let rows = screen(&mut engine, 100, HEIGHT, at);
    let panel = title_row(&rows, "commit");
    assert_eq!(
        rows[panel + 1],
        " line 1",
        "scrolled back to the cursor: {rows:#?}"
    );
}

#[test]
fn a_few_lines_grow_the_panel_one_row_each() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    engine.update(Event::Paste("a\nb\nc\nd\ne".to_owned()), at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    let panel = title_row(&rows, "commit");
    assert_eq!(usize::from(HEIGHT) - panel, 8, "{rows:#?}");
    assert_eq!(rows[panel + 1..panel + 6], [" a", " b", " c", " d", " e"]);
}

#[test]
fn help_on_the_log_and_a_thread_lists_c_r_and_q() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    for _ in 0..2 {
        press(&mut engine, "?", at);
        let rows = screen(&mut engine, 100, 50, at);
        for shown in ["c  write a commit", "r  reply", "q  quote"] {
            assert!(contains(&rows, shown), "{shown}: {rows:#?}");
        }
        press(&mut engine, "?", at);
        let effects = tap(&mut engine, KeyCode::Enter, at);
        common::answer(&mut engine, effects, at);
    }
}

#[test]
fn a_narrow_composer_hint_keeps_esc_close() {
    let hints = "ctrl-s commit  ctrl-e editor  esc close";
    assert_eq!(keys::fit(hints, 30), "ctrl-s commit  esc close");
    assert_eq!(keys::fit(hints, 12), "esc close");
}
