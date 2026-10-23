mod common;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use common::{
    account, boot_serving, contains, from_world, older, press, replies, runs, screen, tap,
    world_log,
};
use ttyl::api::{Call, Error, Reply};
use ttyl::clock::POLL;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitId, Log, LogPage, Shown};
use ratatui::crossterm::event::KeyCode;

const ADA: &str = "9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61";
const PAGED: &str = "4e5f6071-0000-4000-8000-000000000401";
const DEEPEST: &str = "7a000000-0000-4000-8000-000000000066";
const EARLIER: &str = "7a000000-0000-4000-8000-000000000051";
const GRACE: &str = "7a000000-0000-4000-8000-000000000001";
const DELETED: &str = "7a000000-0000-4000-8000-00000000000b";
const WIDTH: u16 = 100;
/// Wide enough for the log to show the selected commit's thread beside it.
const WIDE: u16 = 140;
const HEIGHT: u16 = 40;

fn id(id: &str) -> CommitId {
    CommitId(id.to_owned())
}

fn world_pages(log: Log) -> LogPage {
    LogPage {
        log,
        ..world_log(&[ADA, PAGED, DEEPEST])
    }
}

fn replies_of(owner: &str, cursor: Option<&str>) -> Call {
    Call::Replies {
        id: id(owner),
        cursor: cursor.map(str::to_owned),
    }
}

fn server_error() -> Error {
    Error::Api {
        status: 500,
        code: None,
        message: "boom".to_owned(),
        retry_after: None,
    }
}

fn world(call: &Call) -> Result<Reply, Error> {
    match call {
        Call::Page(log) => Ok(Reply::Page(world_pages(*log))),
        Call::Since { log, .. } => Ok(Reply::Page(older(*log, "new", 0, None))),
        Call::Me => Ok(Reply::Account(account(3))),
        call => from_world(call),
    }
}

fn answer_poll(engine: &mut Engine, effects: Vec<Effect>, at: Instant) -> Vec<Effect> {
    let mut rest = Vec::new();
    for effect in effects {
        match effect {
            Effect::Run(ticket, call @ (Call::Me | Call::Since { .. })) => {
                rest.extend(engine.update(Event::Reply(ticket, world(&call)), at));
            }
            other => rest.push(other),
        }
    }
    rest
}

fn settle(engine: &mut Engine, effects: Vec<Effect>, height: u16, at: Instant) -> Vec<Call> {
    settle_at(engine, effects, WIDTH, height, at)
}

fn settle_at(
    engine: &mut Engine,
    mut effects: Vec<Effect>,
    width: u16,
    height: u16,
    at: Instant,
) -> Vec<Call> {
    let mut calls = Vec::new();
    loop {
        screen(engine, width, height, at);
        effects.extend(engine.pump(at));
        if effects.is_empty() {
            return calls;
        }
        for effect in std::mem::take(&mut effects) {
            let Effect::Run(ticket, call) = effect else {
                panic!("only calls while a thread loads: {effect:?}");
            };
            let result = world(&call);
            calls.push(call);
            effects.extend(engine.update(Event::Reply(ticket, result), at));
        }
    }
}

fn reply(
    engine: &mut Engine,
    effects: Vec<Effect>,
    call: &Call,
    result: Result<Reply, Error>,
    at: Instant,
) -> Vec<Effect> {
    let mut result = Some(result);
    let mut rest = Vec::new();
    for effect in effects {
        match effect {
            Effect::Run(ticket, ref made) if made == call && result.is_some() => {
                let result = result.take().unwrap();
                rest.extend(engine.update(Event::Reply(ticket, result), at));
            }
            other => rest.push(other),
        }
    }
    assert!(result.is_none(), "{call:?} was made");
    rest
}

fn opened(keys: &str, height: u16) -> (Engine, Instant) {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let mut effects = press(&mut engine, keys, at);
    effects.extend(tap(&mut engine, KeyCode::Enter, at));
    settle(&mut engine, effects, height, at);
    (engine, at)
}

#[test]
fn enter_open_is_offered_only_on_a_commit_it_would_open() {
    let (mut engine, at) = opened("", HEIGHT);
    let hint = |engine: &mut Engine| screen(engine, WIDTH, HEIGHT, at)[HEIGHT as usize - 1].clone();
    assert_eq!(
        hint(&mut engine),
        " esc back  j/k move  c commit  ? help  Q quit"
    );
    assert!(tap(&mut engine, KeyCode::Enter, at).is_empty());
    press(&mut engine, "j", at);
    assert_eq!(
        hint(&mut engine),
        " esc back  j/k move  enter open  s star  c commit  ? help  Q quit",
        "a reply by someone else can be starred, the focused own commit cannot"
    );
}

#[test]
fn opening_asks_for_the_commit_and_its_replies_in_the_same_update() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&Call::Commit(id(ADA)), &replies_of(ADA, None)]
    );
}

#[test]
fn opening_spends_the_reserve_and_nested_lists_wait_for_it() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    for _ in 0..80 {
        let effects = tap(&mut engine, KeyCode::Char('R'), at);
        settle(&mut engine, effects, HEIGHT, at);
    }
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&Call::Commit(id(ADA)), &replies_of(ADA, None)]
    );
    let calls = settle(&mut engine, effects, HEIGHT, at);
    assert_eq!(calls, [Call::Commit(id(ADA)), replies_of(ADA, None)]);
    assert!(contains(
        &screen(&mut engine, WIDTH, HEIGHT, at),
        "⋯ 1 reply"
    ));

    let later = at + Duration::from_secs(61);
    let calls = settle(&mut engine, Vec::new(), HEIGHT, later);
    assert!(calls.contains(&replies_of(GRACE, None)), "{calls:?}");
}

#[test]
fn replies_that_land_after_going_back_wait_in_the_store() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    tap(&mut engine, KeyCode::Esc, at);
    let log = screen(&mut engine, WIDTH, HEIGHT, at);
    let footless = |rows: &[String]| rows[..rows.len() - 1].to_vec();
    for effect in effects {
        let Effect::Run(ticket, call) = effect else {
            panic!("only calls: {effect:?}");
        };
        let fired = engine.update(Event::Reply(ticket, from_world(&call)), at);
        assert!(fired.is_empty(), "{fired:?}");
    }
    assert_eq!(
        footless(&screen(&mut engine, WIDTH, HEIGHT, at)),
        footless(&log)
    );
    assert!(runs(&engine.pump(at)).is_empty());

    tap(&mut engine, KeyCode::Enter, at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(contains(&rows, "0000001"), "{rows:#?}");
    assert!(contains(&rows, "0000007"), "{rows:#?}");
}

#[test]
fn nested_lists_load_only_near_the_viewport() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let calls = settle(&mut engine, effects, 8, at);
    assert!(calls.contains(&replies_of(GRACE, None)), "{calls:?}");
    assert!(!calls.contains(&replies_of(DELETED, None)), "{calls:?}");

    let effects = press(&mut engine, "G", at);
    let calls = settle(&mut engine, effects, 8, at);
    assert!(!calls.contains(&replies_of(DELETED, None)), "{calls:?}");
    let mut calls = Vec::new();
    while *engine.thread().unwrap().selected() != id(DELETED) {
        let effects = press(&mut engine, "k", at);
        calls.extend(settle(&mut engine, effects, 8, at));
    }
    assert_eq!(calls, [replies_of(DELETED, None)]);
}

#[test]
fn a_long_list_continues_five_commits_before_its_end_one_call_at_a_time() {
    let (mut engine, at) = opened("j", HEIGHT);
    let first: Vec<CommitId> = replies(&id(PAGED), None)
        .replies
        .into_iter()
        .map(|shown| match shown {
            Shown::Live(reply) => reply.commit.id,
            Shown::Deleted(tombstone) => tombstone.id,
        })
        .collect();
    let more = replies_of(PAGED, Some("NTA"));
    let mut effects = Vec::new();
    while !runs(&effects).contains(&&more) {
        let selected = engine.thread().unwrap().selected().clone();
        assert_ne!(Some(&selected), first.last(), "the list never continued");
        effects = press(&mut engine, "j", at);
        screen(&mut engine, WIDTH, HEIGHT, at);
        effects.extend(engine.pump(at));
    }
    let selected = engine.thread().unwrap().selected().clone();
    assert_eq!(selected, first[first.len() - 6]);

    let again = press(&mut engine, "jGkkR", at);
    assert!(
        !runs(&again).iter().any(|call| call.key() == more.key()),
        "{again:?}"
    );
    let rest = reply(&mut engine, effects, &more, from_world(&more), at);
    let calls = settle(&mut engine, rest, HEIGHT, at);
    assert!(!calls.contains(&more), "{calls:?}");
}

#[test]
fn a_failed_list_retries_on_its_timer_and_clears_only_on_its_own_success() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let list = replies_of(ADA, None);
    let effects = reply(&mut engine, effects, &list, Err(server_error()), at);
    let commit = Call::Commit(id(ADA));
    reply(&mut engine, effects, &commit, from_world(&commit), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(
        contains(&rows, "   fatal: could not load replies · retrying in 30s"),
        "{rows:#?}"
    );
    assert!(contains(&rows, "e4f5a61"));

    let soon = at + Duration::from_secs(10);
    assert!(!runs(&engine.pump(soon)).contains(&&list));
    let rows = screen(&mut engine, WIDTH, HEIGHT, soon);
    assert!(contains(&rows, "retrying in 20s"), "{rows:#?}");

    let due = at + Duration::from_secs(30);
    let poll = engine.pump(due);
    let mut effects = answer_poll(&mut engine, poll, due);
    effects.extend(engine.pump(due));
    assert!(runs(&effects).contains(&&list), "{effects:?}");
    let rows = screen(&mut engine, WIDTH, HEIGHT, due);
    assert!(
        contains(&rows, "fatal: could not load replies"),
        "{rows:#?}"
    );
    assert!(!contains(&rows, "retrying in"), "{rows:#?}");

    reply(&mut engine, effects, &list, from_world(&list), due);
    let rows = screen(&mut engine, WIDTH, HEIGHT, due);
    assert!(!contains(&rows, "fatal"), "{rows:#?}");
    assert!(contains(&rows, "0000001"), "{rows:#?}");
}

#[test]
fn a_failed_reload_keeps_what_was_listed() {
    let (mut engine, at) = opened("", HEIGHT);
    let before = screen(&mut engine, WIDTH, HEIGHT, at);
    let effects = press(&mut engine, "R", at);
    let list = replies_of(ADA, None);
    let effects = reply(&mut engine, effects, &list, Err(server_error()), at);
    settle(&mut engine, effects, HEIGHT, at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(
        contains(&rows, "fatal: could not reload replies · retrying in 30s"),
        "{rows:#?}"
    );
    for hash in ["0000001", "000000e", "000000f"] {
        assert!(
            contains(&before, hash) && contains(&rows, hash),
            "{rows:#?}"
        );
    }
}

#[test]
fn refresh_reloads_the_commit_and_every_loaded_list() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let lists: HashSet<CommitId> = settle(&mut engine, effects, HEIGHT, at)
        .into_iter()
        .filter_map(|call| match call {
            Call::Replies { id, cursor: None } => Some(id),
            _ => None,
        })
        .collect();
    assert!(lists.len() > 3, "{lists:?}");
    let effects = press(&mut engine, "R", at);
    let calls = settle(&mut engine, effects, HEIGHT, at);
    assert!(calls.contains(&Call::Commit(id(ADA))));
    for owner in lists {
        let call = Call::Replies {
            id: owner,
            cursor: None,
        };
        assert!(calls.contains(&call), "{call:?} in {calls:?}");
    }
}

#[test]
fn a_commit_gone_on_open_is_not_asked_for_again() {
    let at = Instant::now();
    let mut engine = boot_serving(at, |log| older(log, "gone", 1, None));
    let effects = tap(&mut engine, KeyCode::Enter, at);
    settle(&mut engine, effects, HEIGHT, at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(contains(&rows, "fatal: bad object gone0"), "{rows:#?}");
    for later in [30, 90, 300] {
        let effects = engine.pump(at + Duration::from_secs(later));
        assert!(
            !runs(&effects)
                .iter()
                .any(|call| matches!(call, Call::Commit(_))),
            "{effects:?}"
        );
    }
}

#[test]
fn the_earlier_line_opens_the_commit_above_the_cut() {
    let (mut engine, at) = opened("jj", HEIGHT);
    let effects = press(&mut engine, "gg", at);
    settle(&mut engine, effects, HEIGHT, at);
    assert_eq!(*engine.thread().unwrap().selected(), id(EARLIER));
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(*engine.thread().unwrap().focus(), id(EARLIER));
    assert!(runs(&effects).contains(&&Call::Commit(id(EARLIER))));

    tap(&mut engine, KeyCode::Esc, at);
    let thread = engine.thread().unwrap();
    assert_eq!(*thread.focus(), id(DEEPEST));
    assert_eq!(*thread.selected(), id(EARLIER));
}

#[test]
fn refreshing_the_log_reloads_every_list_in_the_peeked_thread() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    settle_at(&mut engine, Vec::new(), WIDE, HEIGHT, at);
    let later = at + Duration::from_secs(5);
    let lists: HashSet<CommitId> = settle_at(&mut engine, Vec::new(), WIDE, HEIGHT, later)
        .into_iter()
        .filter_map(|call| match call {
            Call::Replies { id, cursor: None } => Some(id),
            _ => None,
        })
        .collect();
    assert!(lists.len() > 1, "{lists:?}");
    let effects = press(&mut engine, "R", later);
    let calls = settle_at(&mut engine, effects, WIDE, HEIGHT, later);
    assert!(calls.contains(&Call::Page(Log::Following)), "{calls:?}");
    assert!(calls.contains(&Call::Commit(id(ADA))), "{calls:?}");
    for owner in lists {
        let call = Call::Replies {
            id: owner,
            cursor: None,
        };
        assert!(calls.contains(&call), "{call:?} in {calls:?}");
    }
}

#[test]
fn a_new_notification_reloads_every_list_in_the_peeked_thread() {
    let at = Instant::now();
    let mut engine = boot_serving(at, world_pages);
    settle_at(&mut engine, Vec::new(), WIDE, HEIGHT, at);
    let later = at + Duration::from_secs(5);
    let lists: HashSet<CommitId> = settle_at(&mut engine, Vec::new(), WIDE, HEIGHT, later)
        .into_iter()
        .filter_map(|call| match call {
            Call::Replies { id, cursor: None } => Some(id),
            _ => None,
        })
        .collect();
    assert!(lists.len() > 1, "{lists:?}");
    let polled = later + POLL;
    let mut effects = Vec::new();
    for effect in engine.pump(polled) {
        match effect {
            Effect::Run(ticket, Call::Me) => {
                let reply = Ok(Reply::Account(account(4)));
                effects.extend(engine.update(Event::Reply(ticket, reply), polled));
            }
            other => effects.push(other),
        }
    }
    let calls = settle_at(&mut engine, effects, WIDE, HEIGHT, polled);
    assert!(calls.contains(&Call::Commit(id(ADA))), "{calls:?}");
    for owner in lists {
        let call = Call::Replies {
            id: owner,
            cursor: None,
        };
        assert!(calls.contains(&call), "{call:?} in {calls:?}");
    }
}
