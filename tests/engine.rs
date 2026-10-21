mod common;

use std::io;
use std::time::Instant;

use common::{
    account, boot, boot_serving, contains, following_nobody, key, open_browser, page, press, runs,
    screen, secs, serve,
};
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
    let mut engine = boot_serving(at, |log| match log {
        Log::Following => following_nobody(),
        other => page(other),
    });
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
    assert!(contains(&rows, "retrying in 30s"), "{rows:#?}");
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
fn maintenance_after_the_first_load_takes_the_hint_line_over_a_working_screen() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let effects = engine.pump(at + secs(30));
    let [
        Effect::Run(ticket, Call::Me),
        Effect::Run(_, Call::Since { .. }),
    ] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(maintenance())), at + secs(30));

    let rows = screen(&mut engine, 80, 40, at + secs(30));
    assert!(
        rows[39].starts_with(
            " fatal: the site is in maintenance · your log is still here · retrying in 30s"
        ),
        "{:?}",
        rows[39]
    );
    assert!(contains(&rows, "@ada · 18m"), "{rows:#?}");
    press(&mut engine, "j", at + secs(30));
    assert_eq!(engine.log().unwrap().selected(), 1);

    let effects = engine.pump(at + secs(60));
    let [Effect::Run(ticket, Call::Me), ..] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*ticket, Ok(Reply::Account(account(1)))),
        at + secs(60),
    );
    let rows = screen(&mut engine, 80, 40, at + secs(60));
    assert!(
        !rows[39].contains("fatal"),
        "the hints come back once the server answers: {:?}",
        rows[39]
    );
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
    assert!(contains(&rows, "the site is in maintenance"), "{rows:#?}");
    assert!(contains(&rows, "retrying in 30s"), "{rows:#?}");
    assert!(
        !contains(&rows, "your log is still here"),
        "the screen says it once: {rows:#?}"
    );
    let effects = press(&mut engine, "2", at);
    let [Effect::Run(trending, Call::Page(Log::Trending))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        rows[19].starts_with(" fatal: the site is in maintenance · your log is still here"),
        "a loaded log keeps its rows and says it in the hint line: {:?}",
        rows[19]
    );
    engine.update(
        Event::Reply(*trending, Ok(Reply::Page(page(Log::Trending)))),
        at,
    );
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        !contains(&rows, "your log is still here"),
        "the next good answer clears the hint: {rows:#?}"
    );
    press(&mut engine, "1", at);
    let rows = screen(&mut engine, 100, 20, at);
    assert!(contains(&rows, "the site is in maintenance"), "{rows:#?}");

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

fn following_fails_with(error: Error, at: Instant) -> Engine {
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
    engine.update(Event::Reply(*following, Err(error)), at);
    engine
}

#[test]
fn offline_fills_an_empty_log_and_takes_the_hint_line_of_a_loaded_one() {
    let at = Instant::now();
    let mut engine = following_fails_with(Error::Http(ureq::Error::HostNotFound), at);
    let rows = screen(&mut engine, 80, 20, at);
    assert_eq!(
        rows[2..6],
        [
            " fatal: unable to access 'https://dev-null.sh/':",
            "        Could not resolve host: dev-null.sh",
            "",
            " retrying in 30s",
        ],
        "{rows:#?}"
    );

    press(&mut engine, "2", at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "@ada · 18m"), "{rows:#?}");
    assert!(
        rows[19].starts_with(" fatal: Could not resolve host: dev-null.sh · retrying in 30s"),
        "too narrow to also say the log is still here: {:?}",
        rows[19]
    );
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        rows[19].starts_with(
            " fatal: Could not resolve host: dev-null.sh · your log is still here · retrying in 30s"
        ),
        "{:?}",
        rows[19]
    );
}

#[test]
fn a_refusal_says_what_the_server_said() {
    let at = Instant::now();
    let mut engine = following_fails_with(refused(500, "server_error"), at);
    let rows = screen(&mut engine, 80, 20, at);
    assert_eq!(rows[2], " fatal: server_error from the server", "{rows:#?}");
    assert!(!contains(&rows, "unable to access"), "{rows:#?}");
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
    let [Effect::Run(poll, Call::Me), ..] = effects.as_slice() else {
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

fn revoked_mid_flight(at: Instant) -> (Engine, devnull::sched::Ticket) {
    let mut engine = boot(at);
    let effects = engine.pump(at + secs(30));
    let [
        Effect::Run(poll, Call::Me),
        Effect::Run(since, Call::Since { .. }),
    ] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*since, Ok(Reply::Page(page(Log::Following)))),
        at + secs(30),
    );
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
fn an_approved_link_redeems_the_code_and_the_app_opens_without_a_restart() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(
        Event::Reply(me, Err(refused(401, "device_not_approved"))),
        at,
    );
    let browser = open_browser(&mut engine, at);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(
        contains(&rows, "Waiting for you in the browser."),
        "{rows:#?}"
    );
    assert!(contains(&rows, "esc start over  Q quit"), "{rows:#?}");

    engine.update(
        Event::Link(Ok("devnull://signed-in?code=stale&state=other".into())),
        at,
    );
    assert!(
        engine.pump(at).is_empty(),
        "a link for another attempt is noise"
    );
    engine.update(Event::Link(Ok(browser.redirect("code=c0de"))), at);
    let effects = engine.pump(at);
    let [Effect::Run(redeem, Call::Redeem { code, verifier })] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(code, "c0de");
    assert_eq!(*verifier, browser.verifier);
    let rows = screen(&mut engine, 80, 20, at);
    assert!(contains(&rows, "Signing in."), "{rows:#?}");

    engine.update(
        Event::Reply(*redeem, Ok(Reply::Account(account(2)))),
        at + secs(1),
    );
    serve(&mut engine, at + secs(1));
    let rows = screen(&mut engine, 80, 20, at + secs(1));
    assert!(rows[0].contains("1 following"), "{:?}", rows[0]);
    assert!(rows[0].ends_with("● 2  @ada"), "{:?}", rows[0]);
    press(&mut engine, "2", at + secs(1));
    let rows = screen(&mut engine, 80, 20, at + secs(1));
    assert!(contains(&rows, "@ada · 18m"), "{rows:#?}");
}

#[test]
fn a_denied_link_says_so_and_offers_to_try_again() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(
        Event::Reply(me, Err(refused(401, "device_not_approved"))),
        at,
    );
    let browser = open_browser(&mut engine, at);
    engine.update(Event::Link(Ok(browser.redirect("error=access_denied"))), at);
    assert!(engine.pump(at).is_empty(), "nothing to redeem");
    let rows = screen(&mut engine, 80, 20, at);
    assert!(
        contains(&rows, "You cancelled in the browser."),
        "{rows:#?}"
    );
    assert!(contains(&rows, "enter sign in  Q quit"), "{rows:#?}");
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
fn a_refresh_folded_into_a_call_in_flight_still_answers_the_user() {
    let at = Instant::now();
    let (mut engine, me) = fresh(at);
    engine.update(Event::Reply(me, Ok(Reply::Account(account(3)))), at);
    let effects = engine.pump(at);
    let [
        Effect::Run(following, Call::Page(Log::Following)),
        Effect::Run(latest, Call::Page(Log::Latest)),
    ] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    assert!(
        press(&mut engine, "R", at).is_empty(),
        "the page is already on its way"
    );
    engine.update(
        Event::Reply(*following, Ok(Reply::Page(page(Log::Following)))),
        at + secs(1),
    );
    let rows = screen(&mut engine, 80, 20, at + secs(1));
    assert!(
        rows[19].starts_with(" following refreshed"),
        "{:?}",
        rows[19]
    );

    engine.update(
        Event::Reply(*latest, Ok(Reply::Page(page(Log::Latest)))),
        at + secs(2),
    );
    let rows = screen(&mut engine, 80, 20, at + secs(2));
    assert!(
        rows[19].starts_with(" following refreshed"),
        "a page nobody asked to refresh lands quietly: {:?}",
        rows[19]
    );

    let (mut engine, me) = fresh(at);
    engine.update(Event::Reply(me, Ok(Reply::Account(account(3)))), at);
    let effects = engine.pump(at);
    let [
        Effect::Run(following, Call::Page(Log::Following)),
        Effect::Run(_, _),
    ] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    press(&mut engine, "R", at);
    engine.update(Event::Reply(*following, Err(maintenance())), at + secs(1));
    let rows = screen(&mut engine, 80, 20, at + secs(1));
    assert!(contains(&rows, "the site is in maintenance"), "{rows:#?}");
    assert!(!contains(&rows, "refreshed"), "{rows:#?}");
}

#[test]
fn losing_the_keyboard_ends_the_app_with_a_reason() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = engine.update(Event::InputLost(io::Error::other("tty gone")), at);
    assert_eq!(
        effects,
        [Effect::Abort(
            "can't read the keyboard: tty gone".to_owned()
        )]
    );
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
