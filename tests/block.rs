mod common;

use std::time::Instant;

use common::{boot, contains, press, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Filter, NotificationPage, PeoplePage, Person, RowPage};
use ttyl::sched::Ticket;

fn block(on: bool) -> Call {
    Call::Block {
        handle: "grace".to_owned(),
        on,
    }
}

fn on_grace(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    engine
}

fn one_run(effects: &[Effect]) -> Ticket {
    let [Effect::Run(ticket, _)] = effects else {
        panic!("one call goes out: {effects:?}");
    };
    *ticket
}

fn reply(
    engine: &mut Engine,
    ticket: Ticket,
    result: Result<Reply, Error>,
    at: Instant,
) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, result), at);
    effects.extend(engine.pump(at));
    effects
}

fn folded(rows: &[String]) -> usize {
    rows.iter()
        .filter(|row| row.contains("blocked @grace · B to unblock"))
        .count()
}

#[test]
fn b_asks_first_and_y_blocks_and_folds_every_commit_of_the_author() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    assert!(runs(&press(&mut engine, "B", at)).is_empty(), "asks first");
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(
        rows[39].trim_end(),
        " block @grace? this ends follows both ways [y/N]"
    );
    assert!(contains(&rows, "today's footgun"), "{rows:#?}");

    let effects = press(&mut engine, "y", at);
    assert_eq!(runs(&effects), [&block(true)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(
        rows[39].starts_with(" blocked @grace · B to undo"),
        "{}",
        rows[39]
    );
    assert_eq!(folded(&rows), 2, "both of grace's commits fold: {rows:#?}");
    assert!(!contains(&rows, "today's footgun"), "{rows:#?}");
    assert!(
        contains(&rows, "switched the feed"),
        "others stay: {rows:#?}"
    );
}

#[test]
fn any_other_key_than_y_keeps_them_unblocked() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    assert!(runs(&press(&mut engine, "n", at)).is_empty());
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(folded(&rows), 0, "{rows:#?}");
    assert!(!contains(&rows, "block @grace?"), "{rows:#?}");
}

#[test]
fn b_on_a_folded_line_unblocks_without_asking() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    let ticket = one_run(&press(&mut engine, "y", at));
    reply(&mut engine, ticket, Ok(Reply::Done), at);
    assert!(
        press(&mut engine, "f", at).is_empty(),
        "a folded line takes only B"
    );
    let effects = press(&mut engine, "B", at);
    assert_eq!(runs(&effects), [&block(false)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(
        rows[39].starts_with(" unblocked @grace · the follows it ended don't come back"),
        "{}",
        rows[39]
    );
    assert_eq!(folded(&rows), 0, "{rows:#?}");
    assert!(contains(&rows, "today's footgun"), "{rows:#?}");
}

#[test]
fn a_refused_block_unfolds_and_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    let ticket = one_run(&press(&mut engine, "y", at));
    let refused = Error::Api {
        status: 403,
        code: None,
        message: "This action is unauthorized.".to_owned(),
        retry_after: None,
    };
    reply(&mut engine, ticket, Err(refused), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(rows[39].trim_end(), " This action is unauthorized.");
    assert_eq!(folded(&rows), 0, "{rows:#?}");
}

#[test]
fn you_cannot_block_yourself() {
    let at = Instant::now();
    let mut engine = boot(at);
    assert!(press(&mut engine, "B", at).is_empty());
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "block @ada?"), "{rows:#?}");
}

#[test]
fn f_on_an_empty_profile_of_someone_you_blocked_sends_nothing() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    let ticket = one_run(&press(&mut engine, "y", at));
    reply(&mut engine, ticket, Ok(Reply::Done), at);
    let effects = press(&mut engine, "p", at);
    for (ticket, call) in effects.iter().filter_map(|effect| match effect {
        Effect::Run(ticket, call) => Some((*ticket, call)),
        _ => None,
    }) {
        let answer = match call {
            Call::User(handle) => common::profile(handle, false),
            Call::Commits { .. } => Reply::Rows(RowPage {
                rows: Vec::new(),
                next_cursor: None,
            }),
            other => panic!("opening a profile asks for nothing else: {other:#?}"),
        };
        reply(&mut engine, ticket, Ok(answer), at);
    }
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "@grace"), "on her profile: {rows:#?}");
    let effects = press(&mut engine, "f", at);
    assert!(runs(&effects).is_empty(), "{:#?}", runs(&effects));
}

fn grace(is_followed: bool) -> Person {
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
    person["is_followed"] = serde_json::json!(is_followed);
    person["follows_you"] = serde_json::json!(true);
    serde_json::from_value(person).unwrap()
}

#[test]
fn blocking_a_person_from_search_ends_the_follows_both_ways_for_good() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "/@grace", at);
    let asked = one_run(&tap(&mut engine, KeyCode::Enter, at));
    let found = Reply::People(PeoplePage {
        people: vec![grace(true)],
        next_cursor: None,
    });
    reply(&mut engine, asked, Ok(found), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "@grace · follows you · following"),
        "{rows:#?}"
    );

    press(&mut engine, "B", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[29].contains("block @grace? this ends follows both ways"));
    let ticket = one_run(&press(&mut engine, "y", at));
    let rows = screen(&mut engine, 100, 30, at);
    assert_eq!(folded(&rows), 1, "{rows:#?}");
    assert!(
        press(&mut engine, "f", at).is_empty(),
        "no follow across a block"
    );
    reply(&mut engine, ticket, Ok(Reply::Done), at);

    assert_eq!(runs(&press(&mut engine, "B", at)), [&block(false)]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "@grace"), "{rows:#?}");
    assert!(!contains(&rows, "following"), "{rows:#?}");
    assert!(!contains(&rows, "follows you"), "{rows:#?}");
}

#[test]
fn y_blocks_even_when_a_refused_unblock_lands_while_it_asks() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    let blocked = one_run(&press(&mut engine, "y", at));
    reply(&mut engine, blocked, Ok(Reply::Done), at);
    let unblock = one_run(&press(&mut engine, "B", at));
    press(&mut engine, "B", at);
    let refused = Error::Api {
        status: 403,
        code: None,
        message: "This action is unauthorized.".to_owned(),
        retry_after: None,
    };
    reply(&mut engine, unblock, Err(refused), at);

    let effects = press(&mut engine, "y", at);
    assert!(runs(&effects).is_empty(), "{:?}", runs(&effects));
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(folded(&rows), 2, "{rows:#?}");
}

fn notifications() -> NotificationPage {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications.json"
    ))
    .unwrap();
    NotificationPage::parse(&json, Filter::All).unwrap()
}

#[test]
fn a_blocked_reply_folds_in_the_inbox_and_takes_only_b() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "B", at);
    let blocked = one_run(&press(&mut engine, "y", at));
    reply(&mut engine, blocked, Ok(Reply::Done), at);

    open_inbox(&mut engine, notifications(), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(folded(&rows), 1, "grace's reply folds: {rows:#?}");
    assert!(!contains(&rows, "replied to your commit"), "{rows:#?}");
    assert!(
        contains(&rows, "starred your commit"),
        "a star on your own commit stays: {rows:#?}"
    );

    assert!(
        press(&mut engine, "f", at).is_empty(),
        "no follow across a block"
    );
    assert_eq!(runs(&press(&mut engine, "B", at)), [&block(false)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(folded(&rows), 0, "{rows:#?}");
    assert!(contains(&rows, "replied to your commit"), "{rows:#?}");
}

/// Opens notifications and answers with `page`.
fn open_inbox(engine: &mut Engine, page: NotificationPage, at: Instant) {
    let effects = press(engine, "gn", at);
    let Some(&Effect::Run(listed, _)) = effects
        .iter()
        .find(|effect| matches!(effect, Effect::Run(_, Call::Notifications { .. })))
    else {
        panic!("{:?}", runs(&effects));
    };
    engine.update(Event::Reply(listed, Ok(Reply::Notifications(page))), at);
}

#[test]
fn a_group_of_several_stays_open_and_keeps_b_and_f_off_when_its_newest_is_blocked() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jjjB", at);
    let blocked = one_run(&press(&mut engine, "y", at));
    reply(&mut engine, blocked, Ok(Reply::Done), at);

    let mut page = notifications();
    let mentions = &mut page.groups[4];
    assert_eq!(mentions.event.actor.handle, "linus");
    (mentions.actors_count, mentions.event_count) = (2, 2);
    open_inbox(&mut engine, page, at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(
        !contains(&rows, "blocked @linus · B to unblock"),
        "{rows:#?}"
    );
    assert!(contains(&rows, "Linus mentioned you"), "{rows:#?}");

    press(&mut engine, "jjjj", at);
    assert!(press(&mut engine, "B", at).is_empty(), "no unseen unblock");
    assert!(press(&mut engine, "f", at).is_empty(), "no refused follow");
}
