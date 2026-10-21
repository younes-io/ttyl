mod common;

use std::io;
use std::time::Instant;

use common::{account, boot, contains, key, open_browser, page, press, screen, secs, serve};
use devnull::api::{Call, Error, Reply};
use devnull::engine::{Effect, Engine, Event};
use devnull::model::Log;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn fresh(at: Instant) -> (Engine, devnull::sched::Ticket) {
    let mut engine = Engine::new("devnull-tui".to_owned(), at);
    let effects = engine.pump(at);
    let [Effect::Run(ticket, Call::Me)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    (engine, *ticket)
}

fn maintenance() -> Error {
    Error::Unexpected(200)
}

fn refused(status: u16, code: &str) -> Error {
    Error::Api {
        status,
        code: Some(code.to_owned()),
        message: format!("{code} from the server"),
        retry_after: None,
    }
}

fn runs(effects: &[Effect]) -> Vec<&Call> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Run(_, call) => Some(call),
            Effect::StartSignIn | Effect::Quit => None,
        })
        .collect()
}

#[test]
fn keys_answer_while_a_page_is_still_loading() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(Event::Reply(me, Ok(Reply::Account(account(0)))), at);
    let effects = engine.pump(at);
    assert_eq!(
        runs(&effects),
        [&Call::Page(Log::Following), &Call::Page(Log::Latest)],
        "two workers, two pages at once"
    );

    assert!(press(&mut engine, "2", at).is_empty());
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "Loading…"), "{rows:#?}");
    assert!(rows[0].ends_with("@ada"), "{:?}", rows[0]);

    let Effect::Run(latest, _) = &effects[1] else {
        unreachable!()
    };
    engine.update(
        Event::Reply(*latest, Ok(Reply::Page(page(Log::Latest)))),
        at,
    );
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "┆ Today"), "{rows:#?}");
    assert!(contains(&rows, "@ada · 18m"), "{rows:#?}");
}

#[test]
fn the_me_poll_lands_in_the_store_wherever_the_user_went() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "3", at);
    assert!(engine.pump(at + secs(29)).is_empty());
    let effects = engine.pump(at + secs(30));
    let [Effect::Run(ticket, Call::Me)] = effects.as_slice() else {
        panic!("the poll asks for /me: {effects:?}");
    };
    engine.update(
        Event::Reply(*ticket, Ok(Reply::Account(account(5)))),
        at + secs(31),
    );

    let rows = screen(&mut engine, 80, 40, at + secs(31));
    assert!(rows[0].contains("3 trending"), "{:?}", rows[0]);
    assert!(rows[0].ends_with("● 5  @ada"), "{:?}", rows[0]);
    assert!(
        contains(&rows, "@margaret"),
        "trending stays on screen: {rows:#?}"
    );
}

#[test]
fn g_twice_goes_to_the_top_and_g_then_an_unbound_key_does_nothing() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jjj", at);
    assert_eq!(engine.log().unwrap().selected(), 3);

    press(&mut engine, "g", at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "g-  g top"), "{rows:#?}");
    press(&mut engine, "g", at);
    assert_eq!(engine.log().unwrap().selected(), 0);

    press(&mut engine, "jj", at);
    assert!(press(&mut engine, "gx", at).is_empty());
    assert_eq!(engine.log().unwrap().selected(), 2);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(!contains(&rows, "g-"), "{rows:#?}");
    assert!(contains(&rows, "j/k move"), "{rows:#?}");
}

#[test]
fn help_lists_exactly_the_keys_that_work_now() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "?", at);
    let rows = screen(&mut engine, 80, 30, at);
    assert!(contains(&rows, " keys "), "{rows:#?}");
    for shown in ["following", "refresh this log", "quit", "these keys"] {
        assert!(contains(&rows, shown), "{shown}: {rows:#?}");
    }
    for hidden in ["move down", "expand the selected commit"] {
        assert!(
            !contains(&rows, hidden),
            "{hidden} shown for an empty log: {rows:#?}"
        );
    }
    assert!(contains(&rows, "? close"), "{rows:#?}");

    press(&mut engine, "j", at);
    assert_eq!(engine.log().unwrap().active(), Log::Following);
    press(&mut engine, "?", at);
    let rows = screen(&mut engine, 80, 30, at);
    assert!(!contains(&rows, " keys "), "{rows:#?}");

    press(&mut engine, "2?", at);
    let rows = screen(&mut engine, 80, 30, at);
    assert!(contains(&rows, "move down"), "{rows:#?}");
    assert!(
        !contains(&rows, "expand the selected commit"),
        "the first commit is short: {rows:#?}"
    );
    press(&mut engine, "?jjjj?", at);
    let rows = screen(&mut engine, 80, 30, at);
    assert!(contains(&rows, "expand the selected commit"), "{rows:#?}");
    engine.update(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        at,
    );
    let rows = screen(&mut engine, 80, 30, at);
    assert!(!contains(&rows, " keys "), "{rows:#?}");
}

#[test]
fn a_429_holds_every_call_until_retry_after_passes() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "R", at);
    let [Effect::Run(ticket, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(
            *ticket,
            Err(Error::Api {
                status: 429,
                code: Some("rate_limited".to_owned()),
                message: "slow down".to_owned(),
                retry_after: Some(secs(7)),
            }),
        ),
        at,
    );
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "rate limited, retrying in 7s"), "{rows:#?}");

    assert!(press(&mut engine, "2R", at + secs(1)).is_empty());
    assert!(engine.pump(at + secs(6)).is_empty());
    assert_eq!(engine.next_wake(at + secs(6)), Some(at + secs(7)));
    assert_eq!(
        runs(&engine.pump(at + secs(7))),
        [&Call::Page(Log::Following), &Call::Page(Log::Latest)]
    );
}

#[test]
fn maintenance_before_the_first_load_fills_the_screen_and_retries() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "Loading…"), "{rows:#?}");

    engine.update(Event::Reply(me, Err(maintenance())), at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "the site is in maintenance"), "{rows:#?}");
    assert!(contains(&rows, "Retrying in 30s."), "{rows:#?}");
    assert!(contains(&rows, "R retry now  Q quit"), "{rows:#?}");
    assert!(!contains(&rows, "following"), "no tab bar: {rows:#?}");

    assert_eq!(
        engine.next_wake(at),
        Some(at + secs(1)),
        "the countdown redraws every second"
    );
    assert!(engine.pump(at + secs(29)).is_empty());
    assert_eq!(runs(&engine.pump(at + secs(30))), [&Call::Me]);

    let (mut engine, me) = fresh(at);
    engine.update(Event::Reply(me, Err(maintenance())), at);
    assert_eq!(runs(&press(&mut engine, "R", at)), [&Call::Me]);
    assert!(
        engine.pump(at + secs(30)).is_empty(),
        "the retry the user took replaces the timer"
    );
}

#[test]
fn maintenance_after_the_first_load_is_a_banner_over_a_working_screen() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let effects = engine.pump(at + secs(30));
    let [Effect::Run(ticket, Call::Me)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(maintenance())), at + secs(30));

    let rows = screen(&mut engine, 80, 40, at + secs(30));
    assert_eq!(
        rows[1],
        " fatal: the site is in maintenance · your log is still here"
    );
    assert!(contains(&rows, "@ada · 18m"), "{rows:#?}");
    press(&mut engine, "j", at + secs(30));
    assert_eq!(engine.log().unwrap().selected(), 1);

    let effects = engine.pump(at + secs(60));
    let [Effect::Run(ticket, Call::Me)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*ticket, Ok(Reply::Account(account(1)))),
        at + secs(60),
    );
    let rows = screen(&mut engine, 80, 40, at + secs(60));
    assert_eq!(rows[1], "", "the banner clears once the server answers");
}

#[test]
fn a_page_that_fails_waits_its_turn_instead_of_refiring_on_every_key() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(Event::Reply(me, Ok(Reply::Account(account(0)))), at);
    let effects = engine.pump(at);
    let [
        Effect::Run(following, Call::Page(Log::Following)),
        Effect::Run(latest, _),
    ] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*latest, Ok(Reply::Page(page(Log::Latest)))),
        at,
    );
    engine.update(Event::Reply(*following, Err(Error::Unexpected(502))), at);
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        contains(&rows, "fatal: /dev/null answered 502"),
        "{rows:#?}"
    );
    assert!(
        contains(&rows, "retrying in 30s · R retry now"),
        "{rows:#?}"
    );
    assert!(
        contains(
            &rows,
            "fatal: the site is in maintenance · your log is still here"
        ),
        "{rows:#?}"
    );
    serve(&mut engine, at);
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        !contains(&rows, "your log is still here"),
        "the next good answer clears the banner: {rows:#?}"
    );
    assert!(
        contains(&rows, "fatal: /dev/null answered 502"),
        "{rows:#?}"
    );

    assert!(press(&mut engine, "jk", at + secs(1)).is_empty());
    assert!(engine.update(Event::Tick, at + secs(2)).is_empty());
    assert!(engine.pump(at + secs(29)).is_empty());
    let effects = engine.pump(at + secs(30));
    let mut released = runs(&effects);
    released.sort_by_key(|call| format!("{call:?}"));
    assert_eq!(
        released,
        [&Call::Me, &Call::Page(Log::Following)],
        "the poll and the retry share the window"
    );
}

#[test]
fn a_device_the_server_does_not_know_is_sent_to_sign_in() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(
        Event::Reply(me, Err(refused(401, "device_not_approved"))),
        at,
    );
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "Sign in to read your logs."), "{rows:#?}");
    assert!(contains(&rows, "enter sign in  Q quit"), "{rows:#?}");

    assert!(press(&mut engine, "j", at).is_empty());
    let effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        at,
    );
    assert_eq!(effects, [Effect::StartSignIn]);
    engine.update(Event::SignIn(Err(io::Error::other("no browser here"))), at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "no browser here"), "{rows:#?}");
    assert!(contains(&rows, "enter sign in  Q quit"), "{rows:#?}");
}

#[test]
fn a_device_revoked_while_the_app_runs_returns_to_sign_in() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    let effects = engine.pump(at + secs(30));
    let [Effect::Run(poll, Call::Me)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*poll, Err(refused(401, "device_not_approved"))),
        at + secs(30),
    );
    let rows = screen(&mut engine, 80, 20, at + secs(30));
    assert!(contains(&rows, "Sign in to read your logs."), "{rows:#?}");
    assert!(contains(&rows, "enter sign in  Q quit"), "{rows:#?}");
    assert!(engine.log().is_none());
    assert!(press(&mut engine, "j", at + secs(30)).is_empty());
    assert!(
        engine.pump(at + secs(120)).is_empty(),
        "no poll runs for a device the server threw out"
    );
}

/// Both workers are busy and a third refresh waits when the server revokes the device through one of them.
fn revoked_mid_flight(at: Instant) -> (Engine, devnull::sched::Ticket) {
    let mut engine = boot(at);
    let effects = engine.pump(at + secs(30));
    let [Effect::Run(poll, Call::Me)] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let effects = press(&mut engine, "R", at + secs(30));
    let [Effect::Run(following, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert!(press(&mut engine, "2R", at + secs(30)).is_empty());
    engine.update(
        Event::Reply(*following, Err(refused(401, "device_not_approved"))),
        at + secs(30),
    );
    (engine, *poll)
}

#[test]
fn sign_out_drops_queued_calls_and_ignores_replies_to_calls_sent_before_it() {
    let at = Instant::now();
    let (mut engine, stale) = revoked_mid_flight(at);
    assert!(
        engine.pump(at + secs(30)).is_empty(),
        "the refresh that was waiting is dropped"
    );
    open_browser(&mut engine, at + secs(31));
    engine.update(
        Event::Reply(stale, Ok(Reply::Account(account(0)))),
        at + secs(32),
    );
    let rows = screen(&mut engine, 80, 20, at + secs(32));
    assert!(
        contains(&rows, "Waiting for you in the browser."),
        "a stale account is not a sign-in: {rows:#?}"
    );
    assert!(engine.log().is_none());

    let (mut engine, stale) = revoked_mid_flight(at);
    open_browser(&mut engine, at + secs(31));
    engine.update(Event::Reply(stale, Err(maintenance())), at + secs(32));
    let rows = screen(&mut engine, 80, 20, at + secs(32));
    assert!(
        contains(&rows, "Waiting for you in the browser."),
        "a stale failure does not cancel the browser: {rows:#?}"
    );
}

#[test]
fn refresh_toasts_then_the_hints_come_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "R", at);
    let [Effect::Run(ticket, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*ticket, Ok(Reply::Page(page(Log::Following)))),
        at + secs(1),
    );
    let rows = screen(&mut engine, 80, 20, at + secs(1));
    assert_eq!(rows[19], " following refreshed");

    press(&mut engine, "2g", at + secs(2));
    let rows = screen(&mut engine, 80, 20, at + secs(2));
    assert_eq!(rows[19], " g-  g top", "a pending chord outranks the toast");
    press(&mut engine, "g", at + secs(2));

    engine.pump(at + secs(5));
    let rows = screen(&mut engine, 80, 20, at + secs(5));
    assert_eq!(rows[19], " j/k move  1-3 log  R refresh  ? help  Q quit");
}

#[test]
fn quit_works_in_every_phase() {
    let at = Instant::now();
    let (mut engine, _) = fresh(at);
    assert_eq!(engine.update(Event::Key(key('Q')), at), [Effect::Quit]);
    let mut engine = boot(at);
    press(&mut engine, "?", at);
    assert_eq!(engine.update(Event::Key(key('Q')), at), [Effect::Quit]);
}
