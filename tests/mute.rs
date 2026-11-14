mod common;

use std::time::Instant;

use common::{boot, contains, press, runs, screen};
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::sched::Ticket;

fn mute(on: bool) -> Call {
    Call::Mute {
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

#[test]
fn m_folds_every_commit_of_the_author_away_at_once() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = press(&mut engine, "m", at);
    assert_eq!(runs(&effects), [&mute(true)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(
        rows[39].starts_with(" muted @grace · m to undo"),
        "{}",
        rows[39]
    );
    let folded = rows
        .iter()
        .filter(|row| row.contains("muted @grace · m to unmute"))
        .count();
    assert_eq!(folded, 2, "both of grace's commits fold: {rows:#?}");
    assert!(!contains(&rows, "today's footgun"), "{rows:#?}");
    assert!(
        contains(&rows, "switched the feed"),
        "others stay: {rows:#?}"
    );
}

#[test]
fn m_again_unmutes_once_the_first_answer_is_in() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let ticket = one_run(&press(&mut engine, "m", at));
    assert!(
        runs(&press(&mut engine, "m", at)).is_empty(),
        "one mute call at a time"
    );
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "today's footgun"), "{rows:#?}");
    assert_eq!(
        runs(&reply(&mut engine, ticket, Ok(Reply::Done), at)),
        [&mute(false)]
    );
}

#[test]
fn a_refused_mute_unfolds_and_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let ticket = one_run(&press(&mut engine, "m", at));
    let refused = Error::Api {
        status: 404,
        code: None,
        message: "That user does not exist.".to_owned(),
        retry_after: None,
    };
    reply(&mut engine, ticket, Err(refused), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(rows[39], " That user does not exist.");
    assert!(contains(&rows, "today's footgun"), "{rows:#?}");
}

#[test]
fn you_cannot_mute_yourself() {
    let at = Instant::now();
    let mut engine = boot(at);
    assert!(press(&mut engine, "m", at).is_empty());
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "muted"), "{rows:#?}");
}
