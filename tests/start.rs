mod common;

use std::time::Instant;

use common::{account, contains, older, page, runs, screen};
use ttyl::api::{Call, Error, Reply};
use ttyl::cache::Start;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::log::LOGS;
use ttyl::model::{Log, LogPage};

fn launch(pages: fn(Log) -> LogPage, at: Instant) -> (Engine, Vec<Effect>) {
    let start = Start {
        account: account(3),
        pages: LOGS.map(pages).into(),
    };
    let mut engine = Engine::new("ttyl".to_owned(), at).with_start(start, at);
    let effects = engine.pump(at);
    (engine, effects)
}

/// Answers every call the engine makes until it asks for nothing more.
fn reply(
    engine: &mut Engine,
    mut effects: Vec<Effect>,
    at: Instant,
    answer: impl Fn(&Call) -> Result<Reply, Error>,
) {
    while !effects.is_empty() {
        for effect in std::mem::take(&mut effects) {
            if let Effect::Run(ticket, call) = effect {
                effects.extend(engine.update(Event::Reply(ticket, answer(&call)), at));
            }
        }
        effects.extend(engine.pump(at));
    }
}

fn stale(log: Log) -> LogPage {
    older(log, "stale", 3, None)
}

#[test]
fn the_last_timeline_shows_before_the_server_answers() {
    let at = Instant::now();
    let (mut engine, effects) = launch(page, at);

    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "e4f5a61"), "{rows:#?}");
    let calls = runs(&effects);
    assert!(calls.contains(&&Call::Me), "{calls:?}");
    assert!(
        calls.iter().any(|call| matches!(call, Call::Page(_))),
        "{calls:?}"
    );
}

#[test]
fn fresh_pages_replace_the_remembered_ones() {
    let at = Instant::now();
    let (mut engine, effects) = launch(stale, at);
    assert!(contains(&screen(&mut engine, 100, 30, at), "stale000"));

    reply(&mut engine, effects, at, |call| match call {
        Call::Page(log) => Ok(Reply::Page(page(*log))),
        _ => Ok(Reply::Account(account(3))),
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "stale000"), "{rows:#?}");
    assert!(contains(&rows, "e4f5a61"), "{rows:#?}");
}

#[test]
fn offline_keeps_the_remembered_timeline() {
    let at = Instant::now();
    let (mut engine, effects) = launch(page, at);

    reply(&mut engine, effects, at, |_| {
        Err(Error::Http(ureq::Error::ConnectionFailed))
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "e4f5a61"), "{rows:#?}");
}

#[test]
fn a_device_the_server_no_longer_knows_leaves_the_timeline_for_sign_in() {
    let at = Instant::now();
    let (mut engine, effects) = launch(page, at);

    reply(&mut engine, effects, at, |_| {
        Err(Error::Api {
            status: 401,
            code: Some("device_not_approved".to_owned()),
            message: "This key isn't approved".to_owned(),
            retry_after: None,
        })
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "Sign in to read your logs."), "{rows:#?}");
    assert!(!contains(&rows, "e4f5a61"), "{rows:#?}");
}
