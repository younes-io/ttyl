mod common;

use std::time::Instant;

use common::{answer, boot, buffer, commit, contains, press, runs, screen, secs, tap};
use devnull::api::{Call, Error, Reply};
use devnull::engine::{Effect, Engine, Event};
use devnull::model::{CommitId, Focus};
use devnull::sched::Ticket;
use devnull::theme;
use ratatui::crossterm::event::KeyCode;

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

/// Latest, with grace's commit selected: 27 stars, not starred, not ada's.
fn on_grace(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    engine
}

fn star(on: bool) -> Call {
    Call::Star { id: grace(), on }
}

fn follow(on: bool) -> Call {
    Call::Follow {
        username: "grace".to_owned(),
        on,
    }
}

/// What the server answers a star with: the commit as it now stands.
fn starred(on: bool, stars_count: u32) -> Reply {
    let page = commit(&grace()).unwrap();
    let mut card = page.commit;
    card.starred = on;
    card.stars_count = stars_count;
    Reply::Commit(Box::new(Focus {
        card,
        replies_count: page.replies_count,
    }))
}

fn refused(message: &str) -> Error {
    Error::Api {
        status: 403,
        code: Some("forbidden".to_owned()),
        message: message.to_owned(),
        retry_after: None,
    }
}

fn one_run(effects: &[Effect]) -> (Ticket, Call) {
    let [Effect::Run(ticket, call)] = effects else {
        panic!("one call goes out: {effects:?}");
    };
    (*ticket, call.clone())
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

/// The hint line without the spinner a call in flight draws at its end.
fn footer(engine: &mut Engine, at: Instant) -> String {
    let rows = screen(engine, 100, 30, at);
    rows[29]
        .trim_end_matches(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
        .trim_end()
        .to_owned()
}

/// The star of grace's row as drawn: its count and whether it is amber.
fn drawn_star(engine: &mut Engine, width: u16, height: u16, at: Instant) -> (String, bool) {
    let buffer = buffer(engine, width, height, at);
    let rows = screen(engine, width, height, at);
    let y = rows
        .iter()
        .position(|row| row.ends_with("0000201"))
        .unwrap_or_else(|| panic!("grace's row is on screen: {rows:#?}"));
    let star = (y..rows.len())
        .flat_map(|y| (0..width).map(move |x| (x, y as u16)))
        .find(|at| buffer[*at].symbol() == "★")
        .unwrap_or_else(|| panic!("grace's row has a star: {rows:#?}"));
    let count: String = (star.0 + 2..width)
        .map(|x| buffer[(x, star.1)].symbol().to_owned())
        .take_while(|symbol| symbol.chars().all(|c| c.is_ascii_digit()) && !symbol.is_empty())
        .collect();
    (count, buffer[star].fg == theme::AMBER)
}

#[test]
fn a_star_goes_out_at_once_and_turns_amber_with_the_new_count_on_the_same_frame() {
    for width in [100, 44] {
        let at = Instant::now();
        let mut engine = on_grace(at);
        assert_eq!(
            drawn_star(&mut engine, width, 30, at),
            ("27".to_owned(), false)
        );
        let effects = press(&mut engine, "s", at);
        assert_eq!(runs(&effects), [&star(true)]);
        assert_eq!(
            drawn_star(&mut engine, width, 30, at),
            ("28".to_owned(), true),
            "{width} columns"
        );
        let (ticket, _) = one_run(&effects);
        assert!(reply(&mut engine, ticket, Ok(starred(true, 28)), at).is_empty());
        assert_eq!(
            drawn_star(&mut engine, width, 30, at),
            ("28".to_owned(), true),
            "the answer carrying the new count does not count the star twice"
        );
    }
}

#[test]
fn stars_pressed_fast_keep_one_call_out_and_end_where_the_last_press_left_them() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = press(&mut engine, "sss", at);
    assert_eq!(runs(&effects), [&star(true)], "one call is out per commit");
    let (ticket, _) = one_run(&effects);
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("28".to_owned(), true)
    );

    let effects = press(&mut engine, "s", at);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("27".to_owned(), false)
    );

    let effects = reply(&mut engine, ticket, Ok(starred(true, 28)), at);
    assert_eq!(
        runs(&effects),
        [&star(false)],
        "the server is told the last press"
    );
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("27".to_owned(), false)
    );
    let (ticket, _) = one_run(&effects);
    assert!(reply(&mut engine, ticket, Ok(starred(false, 27)), at).is_empty());
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("27".to_owned(), false)
    );
}

#[test]
fn a_refused_star_goes_back_and_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, _) = one_run(&press(&mut engine, "s", at));
    assert!(
        reply(
            &mut engine,
            ticket,
            Err(refused("you can't star this commit")),
            at
        )
        .is_empty()
    );
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("27".to_owned(), false)
    );
    assert_eq!(footer(&mut engine, at), " you can't star this commit");
}

#[test]
fn a_star_answered_with_nonsense_goes_back_with_the_short_hash() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, _) = one_run(&press(&mut engine, "s", at));
    let nonsense = Err(Error::WrongShape("missing field `data`".to_owned()));
    reply(&mut engine, ticket, nonsense, at);
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("27".to_owned(), false)
    );
    assert_eq!(footer(&mut engine, at), " fatal: could not star 0000201");
}

#[test]
fn a_star_that_could_not_reach_the_server_stays_and_is_sent_once_it_is_back() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, _) = one_run(&press(&mut engine, "s", at));
    let offline = Err(Error::Http(ureq::Error::ConnectionFailed));
    assert!(reply(&mut engine, ticket, offline, at).is_empty());
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("28".to_owned(), true)
    );

    let effects = press(&mut engine, "R", at + secs(1));
    let [Effect::Run(page, Call::Page(log))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let effects = reply(
        &mut engine,
        *page,
        Ok(Reply::Page(common::page(*log))),
        at + secs(1),
    );
    assert_eq!(runs(&effects), [&star(true)]);
}

#[test]
fn follow_asks_who_the_user_follows_first_then_toggles_at_once() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, call) = one_run(&press(&mut engine, "f", at));
    assert_eq!(call, Call::User("grace".to_owned()));

    let effects = reply(
        &mut engine,
        ticket,
        Ok(Reply::User {
            is_following: false,
        }),
        at,
    );
    assert_eq!(runs(&effects), [&follow(true)]);
    assert_eq!(footer(&mut engine, at), " following @grace · f to undo");
    let (ticket, _) = one_run(&effects);
    assert!(reply(&mut engine, ticket, Ok(Reply::Done), at).is_empty());

    let effects = press(&mut engine, "f", at);
    assert_eq!(runs(&effects), [&follow(false)]);
    assert_eq!(footer(&mut engine, at), " unfollowed @grace · f to undo");
}

#[test]
fn a_refused_follow_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, _) = one_run(&press(&mut engine, "f", at));
    let effects = reply(
        &mut engine,
        ticket,
        Ok(Reply::User { is_following: true }),
        at,
    );
    assert_eq!(runs(&effects), [&follow(false)]);
    let (ticket, _) = one_run(&effects);
    reply(&mut engine, ticket, Err(refused("slow down")), at);
    assert_eq!(footer(&mut engine, at), " slow down");
    let effects = press(&mut engine, "f", at);
    assert_eq!(
        runs(&effects),
        [&follow(false)],
        "the refused unfollow went back to following"
    );
}

#[test]
fn star_and_follow_are_not_offered_on_your_own_commit() {
    let at = Instant::now();
    let mut engine = boot(at);
    assert!(press(&mut engine, "sf", at).is_empty());
    let rows = screen(&mut engine, 100, 30, at);
    assert_eq!(rows[29], " j/k move  enter open  ? help  Q quit");
    press(&mut engine, "?", at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "star or unstar"), "{rows:#?}");
    assert!(!contains(&rows, "follow or unfollow"), "{rows:#?}");

    let mut engine = on_grace(at);
    press(&mut engine, "?", at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "s  star or unstar"), "{rows:#?}");
    assert!(
        contains(&rows, "f  follow or unfollow the author"),
        "{rows:#?}"
    );
}

#[test]
fn a_star_in_a_thread_is_the_same_star_on_the_log() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    assert_eq!(engine.thread().unwrap().focus(), &grace());
    let effects = press(&mut engine, "s", at);
    assert_eq!(runs(&effects), [&star(true)]);
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("28".to_owned(), true)
    );
    tap(&mut engine, KeyCode::Esc, at);
    assert!(engine.thread().is_none());
    assert_eq!(
        drawn_star(&mut engine, 100, 30, at),
        ("28".to_owned(), true)
    );
}
