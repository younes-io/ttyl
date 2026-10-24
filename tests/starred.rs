mod common;

use std::time::Instant;

use common::{boot, contains, page, press, reload, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Log, Row, RowPage};
use ttyl::sched::Ticket;

fn ticket(effects: &[Effect], want: impl Fn(&Call) -> bool) -> Ticket {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(ticket, call) if want(call) => Some(*ticket),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no such call in {:#?}", runs(effects)))
}

fn first_page(effects: &[Effect]) -> Ticket {
    ticket(effects, |call| *call == Call::Starred { cursor: None })
}

/// Commits by others, since your own cannot be starred.
fn stars(count: usize) -> Vec<Row> {
    let rows: Vec<Row> = page(Log::Latest)
        .rows
        .into_iter()
        .filter(|row| row.commit.author.username != "ada")
        .take(count)
        .map(|row| Row {
            starred: true,
            ..row
        })
        .collect();
    assert_eq!(
        rows.len(),
        count,
        "the fixture has {count} commits by others"
    );
    rows
}

fn land(engine: &mut Engine, ticket: Ticket, rows: Vec<Row>, next: Option<&str>, at: Instant) {
    let reply = Reply::Rows(RowPage {
        rows,
        next_cursor: next.map(str::to_owned),
    });
    engine.update(Event::Reply(ticket, Ok(reply)), at);
}

/// Opens the starred screen and answers it with `rows`.
fn on_starred(at: Instant, rows: Vec<Row>, next: Option<&str>) -> Engine {
    let mut engine = boot(at);
    let effects = press(&mut engine, "gs", at);
    land(&mut engine, first_page(&effects), rows, next, at);
    engine
}

fn short_id(row: &Row) -> String {
    row.commit.id.0[row.commit.id.0.len() - 7..].to_owned()
}

#[test]
fn gs_asks_for_the_first_page_and_shows_loading() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gs", at);
    first_page(&effects);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "starred"), "{rows:#?}");
}

#[test]
fn starred_commits_draw_as_cards_in_the_order_given() {
    let at = Instant::now();
    let given = stars(3);
    let mut engine = on_starred(at, given.clone(), None);
    let rows = screen(&mut engine, 100, 40, at);
    let lines: Vec<usize> = given
        .iter()
        .map(|row| {
            let id = short_id(row);
            rows.iter()
                .position(|line| line.contains(&id))
                .unwrap_or_else(|| panic!("{id} is on screen: {rows:#?}"))
        })
        .collect();
    assert!(lines.is_sorted(), "{lines:?} {rows:#?}");
}

#[test]
fn nothing_starred_says_how_to_star() {
    let at = Instant::now();
    let mut engine = on_starred(at, Vec::new(), None);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(
            &rows,
            "Nothing starred yet. Press s on a commit to star it."
        ),
        "{rows:#?}"
    );
}

#[test]
fn enter_opens_the_selected_commits_thread() {
    let at = Instant::now();
    let given = stars(3);
    let mut engine = on_starred(at, given.clone(), None);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Commit(given[0].commit.id.clone())),
        "{:#?}",
        runs(&effects)
    );
}

#[test]
fn nearing_the_end_asks_for_the_older_page() {
    let at = Instant::now();
    let mut engine = on_starred(at, stars(6), Some("older"));
    let effects = press(&mut engine, "G", at);
    assert!(
        runs(&effects).contains(&&Call::Starred {
            cursor: Some("older".to_owned()),
        }),
        "{:#?}",
        runs(&effects)
    );
}

#[test]
fn esc_goes_back_to_the_log() {
    let at = Instant::now();
    let mut engine = on_starred(at, stars(3), None);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "starred"), "{rows:#?}");
}

#[test]
fn an_unstarred_row_stays_until_the_next_reload() {
    let at = Instant::now();
    let given = stars(3);
    let mut engine = on_starred(at, given.clone(), None);
    let gone = short_id(&given[0]);
    let effects = press(&mut engine, "s", at);
    assert!(
        runs(&effects).contains(&&Call::Star {
            id: given[0].commit.id.clone(),
            on: false,
        }),
        "{:#?}",
        runs(&effects)
    );
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, &gone), "{rows:#?}");
    let effects = reload(&mut engine, at);
    land(
        &mut engine,
        first_page(&effects),
        given[1..].to_vec(),
        None,
        at,
    );
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, &gone), "{rows:#?}");
    assert!(contains(&rows, &short_id(&given[1])), "{rows:#?}");
}
