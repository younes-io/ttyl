mod common;

use std::io;
use std::time::Instant;

use common::{
    account, boot, boot_serving, contains, following_nobody, key, open_browser, page, press, runs,
    screen, secs, serve,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::Log;

fn fresh(at: Instant) -> (Engine, ttyl::sched::Ticket) {
    let mut engine = Engine::new("ttyl".to_owned(), at);
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
    assert!(rows[1].contains("/trending"), "{:?}", rows[1]);
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
    for hidden in ["move down", "expand the selected commit", "open the thread"] {
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

    let rows = screen(&mut engine, 100, 40, at + secs(30));
    assert!(
        rows[39].starts_with(
            " fatal: the site is in maintenance · your log is still here · retrying in 30s  ? help"
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
        !contains(&rows, "maintenance"),
        "another log's failure stays off this log's screen: {rows:#?}"
    );
    engine.update(
        Event::Reply(*trending, Ok(Reply::Page(page(Log::Trending)))),
        at,
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

fn revoked_mid_flight(at: Instant) -> (Engine, ttyl::sched::Ticket) {
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
    assert!(rows[1].contains("/following"), "{:?}", rows[1]);
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
    assert_eq!(
        rows[19], " g-  g top · n notifications",
        "a pending chord outranks the toast"
    );
    press(&mut engine, "g", at + secs(2));

    engine.pump(at + secs(5));
    let rows = screen(&mut engine, 80, 20, at + secs(5));
    assert_eq!(rows[19], " j/k move  enter open  c commit  ? help  Q quit");
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

#[test]
fn another_logs_failure_stays_off_the_active_logs_footer() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "R", at);
    let [Effect::Run(ticket, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    press(&mut engine, "2", at);
    engine.update(Event::Reply(*ticket, Err(refused(500, "server_error"))), at);
    let rows = screen(&mut engine, 120, 30, at + secs(1));
    assert!(
        !rows[29].contains("fatal") && rows[29].contains("? help"),
        "latest keeps its hints: {:?}",
        rows[29]
    );
    press(&mut engine, "1", at + secs(1));
    let rows = screen(&mut engine, 120, 30, at + secs(1));
    assert!(
        rows[29].starts_with(
            " fatal: server_error from the server · your log is still here · retrying in 29s"
        ),
        "following owns the failure and its countdown: {:?}",
        rows[29]
    );
}

#[test]
fn a_failed_refresh_keeps_its_error_until_its_own_reload_answers() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let effects = press(&mut engine, "R", at + secs(10));
    let [Effect::Run(ticket, Call::Page(Log::Latest))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(maintenance())), at + secs(10));
    let rows = screen(&mut engine, 100, 30, at + secs(11));
    assert!(
        rows[29].starts_with(
            " fatal: the site is in maintenance · your log is still here · retrying in 29s"
        ),
        "{:?}",
        rows[29]
    );
    for effect in engine.pump(at + secs(30)) {
        match effect {
            Effect::Run(ticket, Call::Me) => {
                engine.update(
                    Event::Reply(ticket, Ok(Reply::Account(account(0)))),
                    at + secs(30),
                );
            }
            Effect::Run(ticket, Call::Since { log, .. }) => {
                engine.update(
                    Event::Reply(ticket, Ok(Reply::Page(page(log)))),
                    at + secs(30),
                );
            }
            other => panic!("{other:?}"),
        }
    }
    let rows = screen(&mut engine, 100, 30, at + secs(31));
    assert!(
        rows[29].starts_with(
            " fatal: the site is in maintenance · your log is still here · retrying in 9s"
        ),
        "an unrelated success leaves the refresh error and its countdown: {:?}",
        rows[29]
    );
    let effects = engine.pump(at + secs(40));
    let [Effect::Run(reload, Call::Page(Log::Latest))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(
        Event::Reply(*reload, Ok(Reply::Page(page(Log::Latest)))),
        at + secs(40),
    );
    let rows = screen(&mut engine, 100, 30, at + secs(40));
    assert!(
        !rows[29].contains("fatal"),
        "the reload that answers clears it: {:?}",
        rows[29]
    );
}

#[test]
fn an_older_page_failure_speaks_only_at_the_end_of_the_feed() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "2G", at + secs(5));
    let Some(Effect::Run(ticket, Call::Older { .. })) = effects.first() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(maintenance())), at + secs(5));
    let rows = screen(&mut engine, 120, 30, at + secs(12));
    assert!(
        rows[..29]
            .iter()
            .any(|row| row.contains("fatal: could not load older commits · retrying in 23s")),
        "{rows:#?}"
    );
    assert!(
        !rows[29].contains("fatal") && rows[29].contains("? help"),
        "{:?}",
        rows[29]
    );
}

fn offline() -> Error {
    Error::Http(ureq::Error::ConnectionFailed)
}

fn older_fails_with(error: Error, at: Instant) -> Engine {
    let mut engine = boot(at);
    let effects = press(&mut engine, "2G", at + secs(5));
    let Some(Effect::Run(ticket, Call::Older { .. })) = effects.first() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(error)), at + secs(5));
    engine
}

#[test]
fn the_footer_countdown_ends_in_a_request() {
    let at = Instant::now();
    let mut engine = older_fails_with(maintenance(), at);
    press(&mut engine, "gg", at + secs(6));
    for effect in engine.pump(at + secs(30)) {
        if let Effect::Run(ticket, Call::Me | Call::Since { .. }) = effect {
            engine.update(Event::Reply(ticket, Err(offline())), at + secs(30));
        }
    }
    let foot = screen(&mut engine, 120, 30, at + secs(31))[29].clone();
    let wait: u64 = foot
        .split("retrying in ")
        .nth(1)
        .and_then(|rest| rest.split('s').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{foot:?}"));
    let effects = engine.pump(at + secs(31 + wait));
    assert!(!runs(&effects).is_empty(), "{foot:?} then {effects:?}");
}

#[test]
fn an_offline_older_page_speaks_only_at_the_end_of_the_feed() {
    let at = Instant::now();
    let mut engine = older_fails_with(offline(), at);
    let rows = screen(&mut engine, 120, 30, at + secs(12));
    assert!(
        rows[..29].iter().any(|row| row.contains("retrying in 23s")),
        "{rows:#?}"
    );
    assert!(
        !rows[29].contains("fatal") && rows[29].contains("? help"),
        "{:?}",
        rows[29]
    );
}

#[test]
fn the_help_overlay_footer_names_the_key_that_closes_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "R", at);
    let [Effect::Run(ticket, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(maintenance())), at);
    press(&mut engine, "?", at);
    let foot = screen(&mut engine, 120, 30, at + secs(1))[29].clone();
    assert!(
        foot.contains("fatal") && foot.contains("? close") && !foot.contains("? help"),
        "{foot:?}"
    );
}

#[test]
fn a_failing_footer_keeps_help_wherever_a_working_one_does() {
    let at = Instant::now();
    let mut working = boot(at);
    let mut failing = boot(at);
    let effects = press(&mut failing, "R", at);
    let [Effect::Run(ticket, Call::Page(Log::Following))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    failing.update(Event::Reply(*ticket, Err(offline())), at);
    for width in 1..=120 {
        let foot = |engine: &mut Engine| screen(engine, width, 20, at + secs(1))[19].clone();
        let (working, failing) = (foot(&mut working), foot(&mut failing));
        assert!(
            !working.contains("? help") || failing.contains("? help"),
            "{width}: {failing:?}"
        );
    }
}

#[test]
fn a_cached_feed_matches_a_fresh_one_at_the_same_instant() {
    fn seconds_page(log: Log) -> ttyl::model::LogPage {
        let mut page = page(log);
        for row in &mut page.rows {
            row.commit.created_at = "2026-10-21T09:58:30Z".parse().unwrap();
        }
        page
    }
    let at = Instant::now();
    let draw = |engine: &mut Engine, wall: &str| {
        let wall = chrono::DateTime::parse_from_rfc3339(wall).unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| engine.draw(frame, at, &wall))
            .unwrap();
        terminal.backend().buffer().clone()
    };
    let mut cached = boot_serving(at, seconds_page);
    draw(&mut cached, "2026-10-21T10:00:05+00:00");
    let later = draw(&mut cached, "2026-10-21T10:00:55+00:00");
    let fresh = draw(
        &mut boot_serving(at, seconds_page),
        "2026-10-21T10:00:55+00:00",
    );
    assert_eq!(later, fresh);
}
