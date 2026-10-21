mod common;

use std::time::Instant;

use common::{boot, contains, cursor, older, press, runs, screen, secs};
use devnull::api::{Call, Error, Reply};
use devnull::engine::{Effect, Engine, Event};
use devnull::model::Log;

fn older_of(effects: &[Effect]) -> (devnull::sched::Ticket, String) {
    let [
        Effect::Run(
            ticket,
            Call::Older {
                log: Log::Latest,
                cursor,
            },
        ),
    ] = effects
    else {
        panic!("one older page of latest: {effects:?}");
    };
    (*ticket, cursor.clone())
}

fn selected(engine: &Engine) -> usize {
    engine.log().unwrap().selected()
}

#[test]
fn the_next_page_is_asked_for_once_the_cursor_is_five_commits_from_the_end() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);

    assert!(
        press(&mut engine, "jjjj", at).is_empty(),
        "six of the eleven commits are still below"
    );
    let (_, asked) = older_of(&press(&mut engine, "j", at));
    assert_eq!(asked, cursor(Log::Latest));
    assert!(
        press(&mut engine, "jjGkG", at).is_empty(),
        "one older page in flight per log"
    );
}

#[test]
fn an_older_page_is_appended_after_the_rows_already_listed_without_repeating_them() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = older_of(&press(&mut engine, "2G", at));
    let mut next = older(Log::Latest, "older", 20, Some("page-3"));
    next.rows[0] = common::page(Log::Latest).rows[10].clone();
    engine.update(Event::Reply(ticket, Ok(Reply::Page(next))), at);
    assert!(
        engine.pump(at).is_empty(),
        "the cursor is now far from the end"
    );

    press(&mut engine, "G", at);
    assert_eq!(
        selected(&engine),
        11 + 19 - 1,
        "the repeated row is listed once"
    );
}

#[test]
fn the_last_page_ends_on_the_root_commit() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = older_of(&press(&mut engine, "2G", at));
    engine.update(
        Event::Reply(
            ticket,
            Ok(Reply::Page(older(Log::Latest, "older", 3, None))),
        ),
        at,
    );
    assert!(
        press(&mut engine, "G", at).is_empty(),
        "there is nothing before the root"
    );

    let rows = screen(&mut engine, 100, 20, at);
    assert!(contains(&rows, "older002"), "{rows:#?}");
    assert_eq!(
        rows[rows.len() - 2].trim(),
        "○ That's the root commit. There's nothing before it.",
        "{rows:#?}"
    );
}

#[test]
fn a_failed_older_page_says_so_at_the_end_and_waits_for_its_retry() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = older_of(&press(&mut engine, "2G", at));
    engine.update(Event::Reply(ticket, Err(Error::Unexpected(502))), at);

    let rows = screen(&mut engine, 100, 20, at + secs(12));
    assert_eq!(
        rows[rows.len() - 2].trim(),
        "fatal: could not load older commits · retrying in 18s",
        "{rows:#?}"
    );
    assert!(
        runs(&press(&mut engine, "kjkjG", at + secs(1))).is_empty(),
        "keys never refire it"
    );
    assert!(engine.pump(at + secs(29)).is_empty());
    let effects = engine.pump(at + secs(30));
    assert!(
        runs(&effects).contains(&&Call::Older {
            log: Log::Latest,
            cursor: cursor(Log::Latest)
        }),
        "{effects:?}"
    );
}
