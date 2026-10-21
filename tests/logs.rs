mod common;

use std::time::Instant;

use common::{account, boot, contains, cursor, older, page, press, runs, screen, secs};
use devnull::api::{Call, Error, Reply};
use devnull::engine::{Effect, Engine, Event};
use devnull::model::Log;
use devnull::sched::Ticket;

fn older_of(effects: &[Effect]) -> (Ticket, String) {
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

/// Runs the 30-second poll, answers its account check, and returns the check for new commits if one ran.
fn poll(engine: &mut Engine, at: Instant) -> Option<(Ticket, Call)> {
    let mut since = None;
    for effect in engine.pump(at) {
        match effect {
            Effect::Run(ticket, Call::Me) => {
                engine.update(Event::Reply(ticket, Ok(Reply::Account(account(0)))), at);
            }
            Effect::Run(ticket, call @ Call::Since { .. }) if since.is_none() => {
                since = Some((ticket, call));
            }
            other => panic!("{other:?}"),
        }
    }
    since
}

fn new_commits(engine: &mut Engine, ticket: Ticket, log: Log, count: u32, at: Instant) {
    let mut reply = page(log);
    reply.new_commits_count = Some(count);
    engine.update(Event::Reply(ticket, Ok(Reply::Page(reply))), at);
}

#[test]
fn only_the_visible_log_asks_for_new_commits_and_never_trending() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "3", at);
    assert!(poll(&mut engine, at + secs(30)).is_none());

    press(&mut engine, "2", at + secs(30));
    let (_, call) = poll(&mut engine, at + secs(60)).expect("latest asks");
    assert_eq!(
        call,
        Call::Since {
            log: Log::Latest,
            since: page(Log::Latest).rows[0].commit.id.clone()
        }
    );
}

#[test]
fn new_commits_show_a_pill_and_leave_the_feed_where_it_is() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    let before = screen(&mut engine, 100, 30, at);
    let (ticket, _) = poll(&mut engine, at + secs(30)).unwrap();
    new_commits(&mut engine, ticket, Log::Latest, 2, at + secs(30));

    let after = screen(&mut engine, 100, 30, at + secs(30));
    assert_eq!(after[1].trim(), "↑ 2 new commits · R to load");
    assert_eq!(after[2..], before[2..]);
    assert_eq!(selected(&engine), 2);
}

#[test]
fn r_loads_the_new_commits_above_and_keeps_the_cursor_on_its_commit() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2j", at);
    let (ticket, _) = poll(&mut engine, at + secs(30)).unwrap();
    new_commits(&mut engine, ticket, Log::Latest, 2, at + secs(30));
    let hash = page(Log::Latest).rows[1].commit.short_hash.clone();
    let line = |rows: &[String]| rows.iter().position(|row| row.contains(&hash));
    let before = screen(&mut engine, 100, 40, at + secs(30));

    let effects = press(&mut engine, "R", at + secs(30));
    let [Effect::Run(ticket, Call::Page(Log::Latest))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let mut first = older(Log::Latest, "new", 2, Some(&cursor(Log::Latest)));
    first.rows.extend(page(Log::Latest).rows);
    engine.update(Event::Reply(*ticket, Ok(Reply::Page(first))), at + secs(30));

    let after = screen(&mut engine, 100, 40, at + secs(30));
    assert_eq!(after[1], "", "the pill goes");
    assert_eq!(selected(&engine), 3);
    assert_eq!(line(&after), line(&before), "{after:#?}");
    press(&mut engine, "gg", at + secs(30));
    assert!(contains(
        &screen(&mut engine, 100, 40, at + secs(30)),
        "new001"
    ));
}

#[test]
fn each_log_keeps_its_own_count_across_tab_switches() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let (ticket, _) = poll(&mut engine, at + secs(30)).unwrap();
    new_commits(&mut engine, ticket, Log::Latest, 2, at + secs(30));
    press(&mut engine, "1", at + secs(30));
    let (ticket, call) = poll(&mut engine, at + secs(60)).unwrap();
    assert!(matches!(
        call,
        Call::Since {
            log: Log::Following,
            ..
        }
    ));
    new_commits(&mut engine, ticket, Log::Following, 1, at + secs(60));

    let pill = |engine: &mut Engine| screen(engine, 100, 20, at + secs(60))[1].trim().to_owned();
    assert_eq!(pill(&mut engine), "↑ 1 new commit · R to load");
    press(&mut engine, "2", at + secs(60));
    assert_eq!(pill(&mut engine), "↑ 2 new commits · R to load");
    press(&mut engine, "3", at + secs(60));
    assert_eq!(pill(&mut engine), "");
}
