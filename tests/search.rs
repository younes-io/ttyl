mod common;

use std::time::Instant;

use common::{boot, contains, page, press, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Log, PeoplePage, Person, Row, RowPage};
use ttyl::sched::Ticket;

fn search(engine: &mut Engine, query: &str, at: Instant) -> Vec<Effect> {
    press(engine, &format!("/{query}"), at);
    tap(engine, KeyCode::Enter, at)
}

fn one_run(effects: &[Effect]) -> (Ticket, &Call) {
    let [Effect::Run(ticket, call)] = effects else {
        panic!("one call goes out: {:#?}", runs(effects));
    };
    (*ticket, call)
}

fn answer(
    engine: &mut Engine,
    ticket: Ticket,
    reply: Result<Reply, Error>,
    at: Instant,
) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, reply), at);
    effects.extend(engine.pump(at));
    effects
}

/// Grace's two commits, as a search for them answers.
fn found() -> Vec<Row> {
    let rows: Vec<Row> = page(Log::Latest)
        .rows
        .into_iter()
        .filter(|row| row.commit.author.handle == "grace")
        .rev()
        .collect();
    assert_eq!(rows.len(), 2);
    rows
}

fn grace() -> Person {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/log-latest.json"
    ))
    .unwrap();
    let log: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = log["data"].as_array().unwrap();
    let row = rows.iter().find(|row| row["author"]["handle"] == "grace");
    let mut person = row.unwrap()["author"].clone();
    person["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
    person["is_followed"] = serde_json::json!(false);
    person["follows_you"] = serde_json::json!(true);
    serde_json::from_value(person).unwrap()
}

#[test]
fn slash_searches_commits_and_lists_them_closest_first() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "/offsets", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert_eq!(rows[29].trim_end(), " /offsets█");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    let (ticket, call) = one_run(&effects);
    assert_eq!(*call, Call::SearchCommits("offsets".to_owned()));
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains("/offsets"),
        "the query is the title: {rows:#?}"
    );

    let reply = Reply::Rows(RowPage {
        rows: found(),
        next_cursor: None,
    });
    answer(&mut engine, ticket, Ok(reply), at);
    let rows = screen(&mut engine, 100, 30, at);
    let case = rows
        .iter()
        .position(|row| row.contains("the case against OFFSET"));
    let footgun = rows.iter().position(|row| row.contains("today's footgun"));
    assert!(
        case.is_some() && case < footgun,
        "in the order given: {rows:#?}"
    );

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Commit(id) if id.0.ends_with("0000601"))),
        "enter opens the thread: {:#?}",
        runs(&effects)
    );
}

#[test]
fn at_searches_people_and_enter_opens_a_profile() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = search(&mut engine, "@gra", at);
    let (ticket, call) = one_run(&effects);
    assert_eq!(*call, Call::SearchPeople("gra".to_owned()));

    let reply = Reply::People(PeoplePage {
        people: vec![grace()],
        next_cursor: None,
    });
    answer(&mut engine, ticket, Ok(reply), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "@grace"), "{rows:#?}");
    assert!(rows[29].contains("f follow"), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::User("grace".to_owned())),
        "enter opens the profile: {:#?}",
        runs(&effects)
    );
}

#[test]
fn the_next_search_starts_from_the_last_and_esc_sends_nothing() {
    let at = Instant::now();
    let mut engine = boot(at);
    search(&mut engine, "offsets", at);
    tap(&mut engine, KeyCode::Esc, at);
    press(&mut engine, "/", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[29].starts_with(" /offsets█ "), "{}", rows[29]);
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    assert!(!contains(&screen(&mut engine, 100, 30, at), "/offsets"));
}

#[test]
fn searching_from_results_replaces_them_so_esc_goes_straight_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    search(&mut engine, "offsets", at);
    press(&mut engine, "/", at);
    for _ in 0.."offsets".len() {
        tap(&mut engine, KeyCode::Backspace, at);
    }
    press(&mut engine, "@gra", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&Call::SearchPeople("gra".to_owned())]);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[1].contains("/following"), "back on the log: {rows:#?}");
}

#[test]
fn nothing_found_says_so_and_a_refusal_says_why() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = one_run(&search(&mut engine, "zebras", at));
    let none = Reply::Rows(RowPage {
        rows: Vec::new(),
        next_cursor: None,
    });
    answer(&mut engine, ticket, Ok(none), at);
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "No commits about \"zebras\"."
    ));

    let (ticket, _) = one_run(&search(&mut engine, "ab", at));
    let short = Error::Api {
        status: 422,
        code: None,
        message: "The q field must be at least 3 characters.".to_owned(),
        retry_after: None,
    };
    answer(&mut engine, ticket, Err(short), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "The q field must be at least 3 characters."),
        "{rows:#?}"
    );
}
