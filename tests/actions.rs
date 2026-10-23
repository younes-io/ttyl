mod common;

use std::time::{Duration, Instant};

use common::{
    account, answer, boot, boot_as, buffer, commit, contains, page, press, runs, screen, secs, tap,
};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::clock::Spark;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitId, Focus, Reason};
use ttyl::sched::Ticket;
use ttyl::theme;

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

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

fn footer(engine: &mut Engine, at: Instant) -> String {
    let rows = screen(engine, 100, 30, at);
    rows[29]
        .trim_end_matches(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
        .trim_end()
        .to_owned()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Grace's star as drawn once any pop has finished.
fn drawn_star(engine: &mut Engine, width: u16, height: u16, at: Instant) -> (String, bool) {
    let at = at + Spark::Star.lasts();
    let buffer = buffer(engine, width, height, at);
    let star = star_at(engine, width, height, at);
    let count: String = (star.0 + 2..width)
        .map(|x| buffer[(x, star.1)].symbol().to_owned())
        .take_while(|symbol| symbol.chars().all(|c| c.is_ascii_digit()) && !symbol.is_empty())
        .collect();
    (count, buffer[star].fg == theme::AMBER)
}

fn star_at(engine: &mut Engine, width: u16, height: u16, at: Instant) -> (u16, u16) {
    let buffer = buffer(engine, width, height, at);
    let rows = screen(engine, width, height, at);
    let y = rows
        .iter()
        .position(|row| row.ends_with("0000201"))
        .unwrap_or_else(|| panic!("grace's row is on screen: {rows:#?}"));
    (y..rows.len())
        .flat_map(|y| (0..width).map(move |x| (x, y as u16)))
        .find(|at| buffer[*at].symbol() == "★")
        .unwrap_or_else(|| panic!("grace's row has a star: {rows:#?}"))
}

#[test]
fn a_new_star_pops_in_place_and_settles_amber() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let (ticket, _) = one_run(&press(&mut engine, "s", at));
    let star = star_at(&mut engine, 100, 30, at + Spark::Star.lasts());
    let frames: Vec<(String, bool)> = [0, 300, 500, 800, 1200]
        .map(|n| {
            let buffer = buffer(&mut engine, 100, 30, at + ms(n));
            (
                buffer[star].symbol().to_owned(),
                buffer[star].fg == theme::AMBER,
            )
        })
        .into_iter()
        .collect();
    assert_eq!(
        frames,
        [
            ("·", false),
            ("✧", false),
            ("✦", false),
            ("★", false),
            ("★", true)
        ]
        .map(|(glyph, amber)| (glyph.to_owned(), amber))
    );
    assert!(engine.next_wake(at + ms(1200)).unwrap() <= at + ms(1233));

    assert!(reply(&mut engine, ticket, Ok(starred(true, 28)), at).is_empty());
    engine.pump(at + ms(1500));
    let wake = engine.next_wake(at + ms(1500));
    assert!(
        wake.is_none_or(|wake| wake > at + ms(1600)),
        "frames slow down once the pop is over"
    );
}

#[test]
fn taking_a_star_back_does_not_pop() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "ss", at);
    let star = star_at(&mut engine, 100, 30, at);
    let buffer = buffer(&mut engine, 100, 30, at);
    assert_eq!(buffer[star].symbol(), "★");
    assert_ne!(buffer[star].fg, theme::AMBER);
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
    let star = star_at(&mut engine, 100, 30, at);
    assert_ne!(
        buffer(&mut engine, 100, 30, at)[star].fg,
        theme::AMBER,
        "the pop stops with the star it showed"
    );
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
    assert_eq!(rows[29], " j/k move  enter open  c commit  ? help  Q quit");
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

const GRACE_URL: &str = "https://dev-null.sh/@grace/commits/2c3d4e5f-0000-4000-8000-000000000201";

#[test]
fn y_copies_the_link_and_o_opens_it_on_any_commit() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = press(&mut engine, "y", at);
    assert_eq!(effects, [Effect::Copy(GRACE_URL.to_owned())]);
    assert_eq!(footer(&mut engine, at), format!(" copied {GRACE_URL}"));
    let effects = press(&mut engine, "O", at);
    assert_eq!(effects, [Effect::Open(GRACE_URL.to_owned())]);
    assert_eq!(footer(&mut engine, at), " opened in the browser");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    assert_eq!(
        press(&mut engine, "y", at),
        [Effect::Copy(GRACE_URL.to_owned())]
    );

    press(&mut engine, "y", at);
    let rows = screen(&mut engine, 44, 30, at);
    assert_eq!(
        rows[29].trim_end(),
        " copied https://dev-null.sh/@grace/commit…"
    );

    let mut engine = boot(at);
    let effects = press(&mut engine, "y", at);
    let [Effect::Copy(own)] = effects.as_slice() else {
        panic!("your own commit's link can be copied too");
    };
    assert!(
        own.starts_with("https://dev-null.sh/@ada/commits/"),
        "{own}"
    );
}

fn giving_pasteis(at: Instant) -> Engine {
    let mut me = account(0);
    me.me.pasteis_mode = true;
    me.me.pasteis_remaining = Some(8);
    let mut engine = boot_as(at, me, page);
    press(&mut engine, "2jj", at);
    engine
}

fn gave(remaining: u32) -> Reply {
    let page = commit(&grace()).unwrap();
    Reply::Pastel {
        commit: Box::new(Focus {
            card: page.commit,
            replies_count: page.replies_count,
        }),
        remaining,
    }
}

fn helps(engine: &mut Engine, at: Instant) -> Vec<String> {
    press(engine, "?", at);
    let rows = screen(engine, 100, 40, at);
    press(engine, "?", at);
    rows
}

#[test]
fn a_pastel_is_offered_only_in_pasteis_mode_and_never_on_your_own_commit() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    assert!(press(&mut engine, "P", at).is_empty());
    assert!(!contains(&helps(&mut engine, at), "give a pastel"));

    let mut engine = giving_pasteis(at);
    assert!(contains(&helps(&mut engine, at), "P  give a pastel"));
    press(&mut engine, "gg", at);
    assert!(press(&mut engine, "P", at).is_empty(), "row 0 is ada's own");
    assert!(!contains(&helps(&mut engine, at), "give a pastel"));
}

#[test]
fn a_pastel_shows_a_plus_one_that_floats_up_and_goes() {
    let at = Instant::now();
    let mut engine = giving_pasteis(at);
    press(&mut engine, "P", at);
    let (x, y) = star_at(&mut engine, 100, 30, at);
    let plus = (x + "★ 27 ".len() as u16 - 2, y);
    let drawn = |engine: &mut Engine, n: u64, (x, y): (u16, u16)| {
        let buffer = buffer(engine, 100, 30, at + ms(n));
        format!("{}{}", buffer[(x, y)].symbol(), buffer[(x + 1, y)].symbol())
    };
    assert_eq!(
        drawn(&mut engine, 0, plus),
        "+1",
        "it starts beside the star"
    );
    assert_eq!(
        drawn(&mut engine, 800, (plus.0, y - 1)),
        "+1",
        "then floats up a row"
    );
    assert_ne!(drawn(&mut engine, 800, plus), "+1");
    engine.pump(at + Spark::Pastel.lasts());
    assert_ne!(
        drawn(&mut engine, 1600, (plus.0, y - 1)),
        "+1",
        "then it is gone"
    );
}

#[test]
fn every_pastel_pressed_is_sent_one_at_a_time_and_says_how_many_are_left() {
    let at = Instant::now();
    let mut engine = giving_pasteis(at);
    let effects = press(&mut engine, "PP", at);
    assert_eq!(runs(&effects), [&Call::Pastel(grace())]);
    let (ticket, _) = one_run(&effects);

    let effects = reply(&mut engine, ticket, Ok(gave(7)), at);
    assert_eq!(
        runs(&effects),
        [&Call::Pastel(grace())],
        "the second press is a second gift"
    );
    assert_eq!(
        footer(&mut engine, at),
        " gave a pastel to @grace · 7 left today"
    );
    let (ticket, _) = one_run(&effects);
    assert!(reply(&mut engine, ticket, Ok(gave(6)), at).is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " gave a pastel to @grace · 6 left today"
    );
}

#[test]
fn with_no_pasteis_left_the_server_says_so_and_the_rest_are_not_sent() {
    let at = Instant::now();
    let mut engine = giving_pasteis(at);
    let (ticket, _) = one_run(&press(&mut engine, "PP", at));
    let exhausted = Err(Error::Api {
        status: 422,
        code: Some("pasteis_exhausted".to_owned()),
        message: "you ran out of pastéis. wait until tomorrow.".to_owned(),
        retry_after: None,
    });
    assert!(reply(&mut engine, ticket, exhausted, at).is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " you ran out of pastéis. wait until tomorrow."
    );
}

const ADA: &str = "9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61";

fn ada() -> CommitId {
    CommitId(ADA.to_owned())
}

#[test]
fn delete_is_offered_only_on_your_own_commit() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    assert!(press(&mut engine, "D", at).is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " j/k move  enter open  s star  c commit  ? help  Q quit"
    );
    assert!(!contains(&helps(&mut engine, at), "delete"));

    let mut engine = boot(at);
    assert!(contains(&helps(&mut engine, at), "D  delete"));
    press(&mut engine, "D", at);
    assert_eq!(
        footer(&mut engine, at),
        " delete e4f5a61? this can't be undone [y/N]"
    );
}

const PROMPT: &str = " delete e4f5a61? this can't be undone [y/N]";

#[test]
fn any_key_but_y_on_the_delete_prompt_sends_nothing_and_closes_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    for no in ["n", "N", "j", "D", "s", "2"] {
        assert!(press(&mut engine, &format!("D{no}"), at).is_empty(), "{no}");
        assert_eq!(
            footer(&mut engine, at),
            " j/k move  enter open  c commit  ? help  Q quit",
            "{no}"
        );
        press(&mut engine, "D", at);
        assert_eq!(footer(&mut engine, at), PROMPT, "{no} moved nothing");
        press(&mut engine, "n", at);
    }
    for no in [KeyCode::Esc, KeyCode::Enter, KeyCode::Backspace] {
        press(&mut engine, "D", at);
        assert!(tap(&mut engine, no, at).is_empty(), "{no:?}");
        assert!(engine.thread().is_none(), "{no:?} opened nothing");
        assert_eq!(
            footer(&mut engine, at),
            " j/k move  enter open  c commit  ? help  Q quit",
            "{no:?}"
        );
    }
}

#[test]
fn y_deletes_shows_deleting_then_the_bad_object_row() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "Dy", at);
    assert_eq!(runs(&effects), [&Call::Delete(ada())]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "deleting…"), "{rows:#?}");
    assert!(
        press(&mut engine, "Dy", at).is_empty(),
        "a commit being deleted offers no second delete"
    );

    let (ticket, _) = one_run(&effects);
    assert!(reply(&mut engine, ticket, Ok(Reply::Done), at).is_empty());
    assert_eq!(footer(&mut engine, at), " deleted e4f5a61");
    for width in [100, 44] {
        let rows = screen(&mut engine, width, 30, at);
        assert!(contains(&rows, "fatal: bad object e4f5a61"), "{rows:#?}");
        assert!(!contains(&rows, "deleting…"), "{rows:#?}");
    }
    assert!(press(&mut engine, "Dsy", at).is_empty());
}

#[test]
fn delete_from_a_thread_marks_the_same_commit_gone_on_the_log() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    let effects = press(&mut engine, "Dy", at);
    assert_eq!(runs(&effects), [&Call::Delete(ada())]);
    let (ticket, _) = one_run(&effects);
    reply(&mut engine, ticket, Ok(Reply::Done), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "fatal: bad object e4f5a61"), "{rows:#?}");
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "fatal: bad object e4f5a61"), "{rows:#?}");
}

#[test]
fn a_refused_delete_puts_the_row_back_and_says_why() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = one_run(&press(&mut engine, "Dy", at));
    let effects = reply(
        &mut engine,
        ticket,
        Err(refused("you can only delete your own commits.")),
        at,
    );
    assert!(effects.is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " you can only delete your own commits."
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "deleting…"), "{rows:#?}");
    assert!(!contains(&rows, "bad object"), "{rows:#?}");
    press(&mut engine, "D", at);
    assert_eq!(footer(&mut engine, at), PROMPT, "it can be tried again");
}

#[test]
fn the_delete_prompt_leaves_the_spinner_room_on_a_narrow_terminal() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "D", at);
    let rows = screen(&mut engine, 44, 30, at);
    assert_eq!(rows[29].trim_end(), " delete e4f5a61? [y/N]", "{rows:#?}");
    let buffer = buffer(&mut engine, 44, 30, at);
    assert_eq!(buffer[(1, 29)].fg, theme::DESTRUCTIVE);
}

fn report(reason: Reason, details: Option<&str>) -> Call {
    Call::Report {
        id: grace(),
        reason,
        details: details.map(str::to_owned),
    }
}

const PICK_HINT: &str = " 1-4 pick  enter next  esc cancel  Q quit";
const DETAILS_HINT: &str = " enter send  esc cancel";

#[test]
fn report_is_offered_only_on_a_commit_that_is_not_yours() {
    let at = Instant::now();
    let mut engine = boot(at);
    assert!(press(&mut engine, "!", at).is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " j/k move  enter open  c commit  ? help  Q quit"
    );
    let rows = helps(&mut engine, at);
    assert!(!contains(&rows, "report"), "{rows:#?}");

    let mut engine = on_grace(at);
    assert!(contains(&helps(&mut engine, at), "!  report"));
    press(&mut engine, "!", at);
    assert_eq!(footer(&mut engine, at), PICK_HINT);
}

#[test]
fn a_reason_picked_by_number_and_typed_details_send_one_report_and_thank_you() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    assert!(press(&mut engine, "!2", at).is_empty());
    assert_eq!(footer(&mut engine, at), DETAILS_HINT);
    assert!(
        press(&mut engine, "Qjk links to a scam", at).is_empty(),
        "keys typed in the details are text"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "> Qjk links to a scam"), "{rows:#?}");
    assert!(contains(&rows, "19/500"), "{rows:#?}");
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&report(Reason::Harassment, Some("Qjk links to a scam"))]
    );
    let (ticket, _) = one_run(&effects);
    assert!(reply(&mut engine, ticket, Ok(Reply::Done), at).is_empty());
    assert_eq!(footer(&mut engine, at), " reported 0000201. thanks.");
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        !contains(&rows, "report 0000201"),
        "the dialog closed: {rows:#?}"
    );
}

#[test]
fn a_reason_picked_with_j_and_k_and_no_details_sends_none() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "!jjjjk", at);
    tap(&mut engine, KeyCode::Enter, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&report(Reason::Unsafe, None)]);

    let mut engine = on_grace(at);
    press(&mut engine, "!kk4   ", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&report(Reason::Other, None)],
        "blank details are no details"
    );
}

#[test]
fn esc_on_the_report_dialog_sends_nothing() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "!j", at);
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    assert_eq!(
        footer(&mut engine, at),
        " j/k move  enter open  s star  c commit  ? help  Q quit"
    );
    press(&mut engine, "!3spam", at);
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    assert!(
        engine.thread().is_none(),
        "esc closed the dialog, not the log"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "report 0000201"), "{rows:#?}");
    assert_eq!(
        footer(&mut engine, at),
        " j/k move  enter open  s star  c commit  ? help  Q quit"
    );
}

#[test]
fn details_stop_at_500_characters_and_backspace_takes_one_back() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "!1", at);
    press(&mut engine, &"x".repeat(503), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "500/500"), "{rows:#?}");
    tap(&mut engine, KeyCode::Backspace, at);
    press(&mut engine, "yz", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let text = format!("{}y", "x".repeat(499));
    assert_eq!(runs(&effects), [&report(Reason::Spam, Some(&text))]);
}

#[test]
fn a_refused_report_shows_what_the_server_said() {
    for (status, code, message) in [
        (403, "own_commit", "You cannot report your own commit."),
        (404, "commit_unavailable", "This commit isn't available."),
    ] {
        let at = Instant::now();
        let mut engine = on_grace(at);
        press(&mut engine, "!1", at);
        let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
        let error = Error::Api {
            status,
            code: Some(code.to_owned()),
            message: message.to_owned(),
            retry_after: None,
        };
        assert!(reply(&mut engine, ticket, Err(error), at).is_empty());
        assert_eq!(footer(&mut engine, at), format!(" {message}"));
    }
}

#[test]
fn a_report_from_a_thread_goes_out_the_same_way() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    assert!(engine.thread().is_some());
    press(&mut engine, "!4", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&report(Reason::Other, None)]);
    assert!(engine.thread().is_some(), "the thread stays open");
}

#[test]
fn the_report_dialog_fits_wide_and_narrow_terminals() {
    for width in [100, 44] {
        let at = Instant::now();
        let mut engine = on_grace(at);
        press(&mut engine, "!j", at);
        let rows = screen(&mut engine, width, 30, at);
        for line in [
            "report 0000201",
            "› 2  Harassment or hate",
            "3  Harmful or private information",
            "details (optional)",
            "0/500",
        ] {
            assert!(contains(&rows, line), "{width}: {line} {rows:#?}");
        }
        assert_eq!(rows[29], PICK_HINT, "{width}");
        press(&mut engine, "2", at);
        press(&mut engine, &"long words ".repeat(10), at);
        let rows = screen(&mut engine, width, 30, at);
        let field = rows
            .iter()
            .find_map(|row| row.split_once("│ > "))
            .and_then(|(_, rest)| rest.split_once('│'))
            .map(|(typed, _)| typed.trim_end())
            .unwrap_or_else(|| panic!("{rows:#?}"));
        assert!(
            field.ends_with("words long words"),
            "the field shows the end of what was typed: {field}"
        );
        assert!(contains(&rows, "110/500"), "{rows:#?}");
        assert_eq!(rows[29], DETAILS_HINT, "{width}");
    }
}

#[test]
fn on_a_short_terminal_the_report_dialog_keeps_its_field_and_the_hint_line() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "!3why", at);
    for height in [10, 11] {
        let rows = screen(&mut engine, 44, height, at);
        assert!(contains(&rows, "report 0000201"), "{rows:#?}");
        assert!(contains(&rows, "4  Something else"), "{rows:#?}");
        assert!(contains(&rows, "> why"), "{rows:#?}");
        assert_eq!(rows[usize::from(height) - 1], DETAILS_HINT, "{rows:#?}");
    }
}
