mod common;

use std::time::Instant;

use chrono::{DateTime, Utc};
use common::{boot, contains, press, runs, screen, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Call, Error, Reply};
use ttyl::attach::Attachment;
use ttyl::draft::{Target, Write};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Scheduled, ScheduledId};
use ttyl::sched::Ticket;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;
const PENDING: &str = "Your commit is pending review. Everyone will see it once it is approved.";

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
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

fn one_run(effects: &[Effect]) -> (Ticket, Call) {
    let [Effect::Run(ticket, call)] = effects else {
        panic!("one call goes out: {effects:?}");
    };
    (*ticket, call.clone())
}

fn bottom(rows: &[String]) -> &str {
    &rows[usize::from(HEIGHT) - 1]
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().to_utc()
}

fn waiting(id: &str, at: &str, body: &str) -> Scheduled {
    Scheduled {
        id: ScheduledId(id.to_owned()),
        at: utc(at),
        body: body.to_owned(),
        author: None,
        poll: false,
        images: 0,
    }
}

fn refused(status: u16, message: &str) -> Error {
    Error::Api {
        status,
        code: None,
        message: message.to_owned(),
        retry_after: None,
    }
}

/// A draft with `send at:` open over it, drawn once so the prompt knows the wall clock.
fn writing_later(text: &str, at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    press(&mut engine, text, at);
    assert!(ctrl(&mut engine, 't', at).is_empty());
    screen(&mut engine, WIDTH, HEIGHT, at);
    engine
}

/// The queue screen, answered with `items`.
fn queue(items: Vec<Scheduled>, at: Instant) -> Engine {
    let mut engine = boot(at);
    let (ticket, call) = one_run(&press(&mut engine, "gl", at));
    assert_eq!(call, Call::Queue);
    reply(&mut engine, ticket, Ok(Reply::Queue(items)), at);
    engine
}

fn two_waiting() -> Vec<Scheduled> {
    vec![
        waiting("one", "2026-10-21T12:30:00+00:00", "standup notes"),
        waiting(
            "two",
            "2026-10-23T14:00:00+00:00",
            "shipped the scheduler\nit runs on cron",
        ),
    ]
}

#[test]
fn send_at_shows_where_the_time_lands_while_it_is_typed() {
    let at = Instant::now();
    let mut engine = writing_later("shipped it", at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert_eq!(
        bottom(&rows),
        " send at: █  14:30 · tomorrow 9:00 · fri 14:00 · in 2h"
    );
    for (typed, shown) in [
        (
            "fri 14:00",
            " send at: fri 14:00█  → Fri 23 Oct 14:00 · in 2d 4h",
        ),
        ("in 2h", " send at: in 2h█  → Wed 21 Oct 12:00 · in 2h"),
        ("9:00", " send at: 9:00█  → Thu 22 Oct 09:00 · in 23h"),
        (
            "soon",
            " send at: soon█  14:30 · tomorrow 9:00 · fri 14:00 · in 2h",
        ),
        (
            "2026-10-20T09:00:00Z",
            " send at: 2026-10-20T09:00:00Z█  that time has passed",
        ),
        (
            "in 40d",
            " send at: in 40d█  dev-null.sh schedules up to 30 days ahead",
        ),
    ] {
        let mut engine = writing_later("shipped it", at);
        press(&mut engine, typed, at);
        let rows = screen(&mut engine, WIDTH, HEIGHT, at);
        assert_eq!(bottom(&rows), shown, "{typed}");
    }
    assert!(
        contains(&rows, " shipped it"),
        "the draft stays on screen: {rows:#?}"
    );
}

#[test]
fn enter_schedules_the_draft_then_closes_it_with_where_to_find_it() {
    let at = Instant::now();
    let mut engine = writing_later("shipped it", at);
    press(&mut engine, "fri 14:00", at);
    let (ticket, call) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    assert_eq!(
        call,
        Call::Schedule {
            body: "shipped it".to_owned(),
            attachment: Attachment::None,
            at: DateTime::parse_from_rfc3339("2026-10-23T14:00:00+00:00").unwrap(),
        }
    );
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(contains(&rows, " scheduling… "), "{rows:#?}");

    let scheduled = waiting("one", "2026-10-23T14:00:00+00:00", "shipped it");
    let effects = reply(
        &mut engine,
        ticket,
        Ok(Reply::Scheduled(Box::new(scheduled))),
        at,
    );
    assert_eq!(effects, [Effect::Draft(Write::Drop(Target::New))]);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert_eq!(
        bottom(&rows),
        " scheduled for Fri 23 Oct 14:00 · gl lists it"
    );
    assert!(!contains(&rows, " shipped it"), "{rows:#?}");
}

#[test]
fn enter_waits_for_a_time_that_will_do() {
    let at = Instant::now();
    for typed in ["", "soon", "08:00 yesterday", "in 40d"] {
        let mut engine = writing_later("shipped it", at);
        press(&mut engine, typed, at);
        assert!(tap(&mut engine, KeyCode::Enter, at).is_empty(), "{typed}");
        let rows = screen(&mut engine, WIDTH, HEIGHT, at);
        assert!(
            bottom(&rows).starts_with(&format!(" send at: {typed}█")),
            "{typed}: {rows:#?}"
        );
    }
}

#[test]
fn esc_goes_back_to_the_draft_and_ctrl_s_then_commits_now() {
    let at = Instant::now();
    let mut engine = writing_later("shipped it", at);
    press(&mut engine, "fri 14:00", at);
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(!contains(&rows, "send at:"), "{rows:#?}");
    let (_, call) = one_run(&ctrl(&mut engine, 's', at));
    assert!(
        matches!(&call, Call::Post { target: Target::New, body, .. } if body == "shipped it"),
        "{call:?}"
    );
}

#[test]
fn a_paste_lands_in_send_at_and_leaves_the_draft_alone() {
    let at = Instant::now();
    let mut engine = writing_later("shipped it", at);
    engine.update(Event::Paste("in 2h".to_owned()), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert_eq!(
        bottom(&rows),
        " send at: in 2h█  → Wed 21 Oct 12:00 · in 2h"
    );
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    let (_, call) = one_run(&ctrl(&mut engine, 's', at));
    assert!(
        matches!(&call, Call::Post { body, .. } if body == "shipped it"),
        "{call:?}"
    );
}

#[test]
fn a_refused_schedule_keeps_the_draft_and_says_why() {
    let at = Instant::now();
    let mut engine = writing_later("shipped it", at);
    press(&mut engine, "in 2h", at);
    let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    let limit = "You can have up to 10 scheduled commits. Cancel one before scheduling another.";
    reply(&mut engine, ticket, Err(refused(422, limit)), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(contains(&rows, " shipped it"), "{rows:#?}");
    assert!(contains(&rows, "up to 10 scheduled commits"), "{rows:#?}");
    assert_eq!(engine.draft(&Target::New), Some("shipped it"));
}

#[test]
fn only_a_new_commit_goes_out_later() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jjr", at);
    press(&mut engine, "ack", at);
    ctrl(&mut engine, 't', at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(!contains(&rows, "send at:"), "{rows:#?}");
    assert!(!contains(&rows, "ctrl-t later"), "{rows:#?}");
}

#[test]
fn gl_lists_what_waits_soonest_first() {
    let at = Instant::now();
    let mut engine = queue(two_waiting(), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    let row = |text: &str| {
        rows.iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text}: {rows:#?}"))
    };
    assert!(contains(&rows, " later "), "titled later: {rows:#?}");
    let first = row("Wed 21 Oct 12:30 · in 2h 30m");
    assert!(rows[first + 1].contains("│ standup notes"), "{rows:#?}");
    let second = row("Fri 23 Oct 14:00 · in 2d 4h");
    assert!(
        rows[second + 1].contains("│ shipped the scheduler"),
        "{rows:#?}"
    );
    assert!(rows[second + 2].contains("│ it runs on cron"), "{rows:#?}");
    assert!(first < second, "{rows:#?}");
    assert!(
        contains(&rows, "S commit now") && contains(&rows, "D cancel"),
        "{rows:#?}"
    );
}

#[test]
fn an_empty_queue_says_how_to_fill_it() {
    let at = Instant::now();
    let mut engine = queue(Vec::new(), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(
        contains(
            &rows,
            "Nothing scheduled. In the composer, ctrl-t sends a commit later."
        ),
        "{rows:#?}"
    );
}

#[test]
fn d_asks_then_y_cancels_the_selected_one() {
    let at = Instant::now();
    let mut engine = queue(two_waiting(), at);
    press(&mut engine, "j", at);
    assert!(press(&mut engine, "D", at).is_empty());
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert_eq!(bottom(&rows), " cancel \"shipped the scheduler\"? [y/N]");
    let (ticket, call) = one_run(&press(&mut engine, "y", at));
    assert_eq!(call, Call::Unschedule(ScheduledId("two".to_owned())));
    reply(&mut engine, ticket, Ok(Reply::Done), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(!contains(&rows, "shipped the scheduler"), "{rows:#?}");
    assert!(contains(&rows, "standup notes"), "{rows:#?}");
    assert_eq!(bottom(&rows), " canceled. it won't go out.");
}

#[test]
fn d_then_any_other_key_keeps_it() {
    let at = Instant::now();
    let mut engine = queue(two_waiting(), at);
    press(&mut engine, "D", at);
    assert!(press(&mut engine, "n", at).is_empty());
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(!contains(&rows, "[y/N]"), "{rows:#?}");
    assert!(contains(&rows, "standup notes"), "{rows:#?}");
}

#[test]
fn s_commits_the_selected_one_now() {
    let at = Instant::now();
    for (pending, toast) in [(false, "committed it now"), (true, PENDING)] {
        let mut engine = queue(two_waiting(), at);
        let (ticket, call) = one_run(&press(&mut engine, "S", at));
        assert_eq!(call, Call::SendNow(ScheduledId("one".to_owned())));
        reply(&mut engine, ticket, Ok(Reply::Submitted { pending }), at);
        let rows = screen(&mut engine, WIDTH, HEIGHT, at);
        assert!(!contains(&rows, "standup notes"), "{rows:#?}");
        assert!(contains(&rows, "shipped the scheduler"), "{rows:#?}");
        assert_eq!(bottom(&rows), format!(" {toast}"));
    }
}

#[test]
fn a_refused_cancel_says_why_and_reloads_the_queue() {
    let at = Instant::now();
    let mut engine = queue(two_waiting(), at);
    press(&mut engine, "D", at);
    let (ticket, _) = one_run(&press(&mut engine, "y", at));
    let gone = "This commit was already submitted or canceled. Refresh to see its status.";
    let (ticket, call) = one_run(&reply(&mut engine, ticket, Err(refused(409, gone)), at));
    assert_eq!(call, Call::Queue);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(
        contains(&rows, "already submitted or canceled"),
        "{rows:#?}"
    );
    let rest = vec![two_waiting().remove(1)];
    reply(&mut engine, ticket, Ok(Reply::Queue(rest)), at);
    let rows = screen(&mut engine, WIDTH, HEIGHT, at);
    assert!(!contains(&rows, "standup notes"), "{rows:#?}");
}

#[test]
fn scheduling_with_the_queue_open_reloads_it() {
    let at = Instant::now();
    let mut engine = queue(Vec::new(), at);
    press(&mut engine, "c", at);
    press(&mut engine, "standup notes", at);
    ctrl(&mut engine, 't', at);
    screen(&mut engine, WIDTH, HEIGHT, at);
    press(&mut engine, "12:30", at);
    let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    let scheduled = two_waiting().remove(0);
    let effects = reply(
        &mut engine,
        ticket,
        Ok(Reply::Scheduled(Box::new(scheduled))),
        at,
    );
    assert_eq!(runs(&effects), [&Call::Queue]);
}
