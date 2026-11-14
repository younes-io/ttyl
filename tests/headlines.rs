mod common;

use std::time::Instant;

use common::{boot, buffer, contains, page, press, runs, screen, tap};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Modifier;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Headline, HeadlineId, HeadlinePage, Log, Row};
use ttyl::sched::Ticket;
use ttyl::theme;

const RECAP: &str = "Teams moving to cursor pagination report steady p99s once tables pass a few million rows, though some miss jumping to page 40.";

fn headline(id: &str, name: &str, summary: Option<&str>) -> Headline {
    Headline {
        id: HeadlineId(id.to_owned()),
        name: name.to_owned(),
        summary: summary.map(str::to_owned),
    }
}

fn hot() -> Vec<Headline> {
    vec![
        headline(
            "0199a000-0000-7000-8000-000000000001",
            "Keyset pagination goes mainstream",
            Some(RECAP),
        ),
        headline(
            "0199a000-0000-7000-8000-000000000002",
            "Postgres 18 async IO",
            Some("Early benchmarks show faster sequential scans."),
        ),
        headline(
            "0199a000-0000-7000-8000-000000000003",
            "Rust 2027 edition",
            None,
        ),
    ]
}

/// Grace's two commits, as a headline's page lists them.
fn grace() -> Vec<Row> {
    (page(Log::Latest).rows.into_iter())
        .filter(|row| row.commit.author.handle == "grace")
        .collect()
}

fn one_run(effects: &[Effect]) -> (Ticket, Call) {
    let [Effect::Run(ticket, call)] = effects else {
        panic!("one call goes out: {:#?}", runs(effects));
    };
    (*ticket, call.clone())
}

fn answer(engine: &mut Engine, ticket: Ticket, reply: Reply, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, Ok(reply)), at);
    effects.extend(engine.pump(at));
    effects
}

fn on_headlines(at: Instant, headlines: Vec<Headline>) -> Engine {
    let mut engine = boot(at);
    let (ticket, call) = one_run(&press(&mut engine, "gt", at));
    assert_eq!(call, Call::Headlines);
    answer(&mut engine, ticket, Reply::Headlines(headlines), at);
    engine
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is on screen: {rows:#?}"))
}

fn cell_at(buffer: &Buffer, y: usize, text: &str) -> ratatui::buffer::Cell {
    let row: String = (0..buffer.area.width)
        .map(|x| buffer[(x, y as u16)].symbol().to_owned())
        .collect();
    let x = row[..row.find(text).unwrap()].chars().count();
    buffer[(x as u16, y as u16)].clone()
}

#[test]
fn gt_lists_each_headline_in_bold_over_its_muted_recap() {
    let at = Instant::now();
    let mut engine = on_headlines(at, hot());
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("headlines"), "titled: {rows:#?}");
    let first = row_of(&rows, "Keyset pagination goes mainstream");
    let recap = row_of(&rows, "Teams moving to cursor pagination");
    let wrapped = row_of(&rows, "page 40.");
    let second = row_of(&rows, "Postgres 18 async IO");
    let third = row_of(&rows, "Rust 2027 edition");
    assert!(
        first < recap && recap < wrapped && wrapped < second && second < third,
        "name over its recap, wrapped, hottest first: {rows:#?}"
    );

    let buffer = buffer(&mut engine, 100, 30, at);
    let name = cell_at(&buffer, second, "Postgres");
    assert!(name.modifier.contains(Modifier::BOLD), "{name:?}");
    let recap = cell_at(&buffer, second + 1, "Early benchmarks");
    assert_eq!(recap.fg, theme::MUTED_FOREGROUND);
    assert!(!recap.modifier.contains(Modifier::BOLD), "{recap:?}");
}

#[test]
fn enter_opens_the_headlines_commits_under_its_recap() {
    let at = Instant::now();
    let mut engine = on_headlines(at, hot());
    let (ticket, call) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    assert_eq!(call, Call::Headline(hot()[0].id.clone()));
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains("Keyset pagination goes mainstream"),
        "titled with its name: {rows:#?}"
    );
    assert!(
        contains(&rows, "Teams moving to cursor pagination"),
        "the recap shows while the commits load: {rows:#?}"
    );

    let page = HeadlinePage {
        name: "Keyset pagination goes mainstream".to_owned(),
        summary: Some("Fresher recap: OFFSET is out.".to_owned()),
        rows: grace(),
    };
    answer(&mut engine, ticket, Reply::Headline(Box::new(page)), at);
    let rows = screen(&mut engine, 100, 30, at);
    let recap = row_of(&rows, "Fresher recap: OFFSET is out.");
    let commit = row_of(&rows, "today's footgun");
    assert!(
        recap < commit,
        "the recap sits above the commits: {rows:#?}"
    );
    assert!(!contains(&rows, "Teams moving"), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Commit(grace()[0].commit.id.clone())),
        "enter opens a commit's thread: {:#?}",
        runs(&effects)
    );
    tap(&mut engine, KeyCode::Esc, at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("headlines"), "esc walks back: {rows:#?}");
}

#[test]
fn a_headline_without_a_recap_lists_its_commits_alone() {
    let at = Instant::now();
    let mut engine = on_headlines(at, hot());
    press(&mut engine, "jj", at);
    let (ticket, call) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    assert_eq!(call, Call::Headline(hot()[2].id.clone()));
    let page = HeadlinePage {
        name: "Rust 2027 edition".to_owned(),
        summary: None,
        rows: grace(),
    };
    answer(&mut engine, ticket, Reply::Headline(Box::new(page)), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("Rust 2027 edition"), "{rows:#?}");
    assert!(
        rows[2].contains("Today") && rows[4].contains("Grace"),
        "newest first by day, straight under the title: {rows:#?}"
    );
}

#[test]
fn nothing_hot_or_nothing_under_a_headline_says_so() {
    let at = Instant::now();
    let mut engine = on_headlines(at, Vec::new());
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "Nothing is hot right now."), "{rows:#?}");

    let mut engine = on_headlines(at, hot());
    let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    let page = HeadlinePage {
        name: "Keyset pagination goes mainstream".to_owned(),
        summary: None,
        rows: Vec::new(),
    };
    answer(&mut engine, ticket, Reply::Headline(Box::new(page)), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "No commits under this headline from the past week."),
        "{rows:#?}"
    );
}

#[test]
fn gt_on_the_headlines_sends_nothing_more() {
    let at = Instant::now();
    let mut engine = on_headlines(at, hot());
    assert!(runs(&press(&mut engine, "gt", at)).is_empty());
}
