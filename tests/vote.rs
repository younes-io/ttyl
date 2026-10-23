mod common;

use std::time::{Duration, Instant};

use common::{boot_as, boot_serving, buffer, contains, page, press, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Account, Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Card, CommitId, Focus, Log, LogPage, Row};

const POLL: usize = 7;
const TABS: &str = "8b000000-0000-4000-8000-000000000801";
const SPACES: &str = "8b000000-0000-4000-8000-000000000802";

fn poll_row() -> Row {
    page(Log::Latest).rows[POLL].clone()
}

fn id() -> CommitId {
    poll_row().commit.id
}

fn on_poll(at: Instant) -> Engine {
    on_poll_serving(at, page)
}

fn on_poll_serving(at: Instant, pages: fn(Log) -> LogPage) -> Engine {
    let mut engine = boot_serving(at, pages);
    press(&mut engine, "2jjjjjjj", at);
    engine
}

fn with_poll(log: Log, change: impl Fn(&mut ttyl::model::Poll)) -> LogPage {
    let mut page = page(log);
    for row in &mut page.rows {
        if let Some(poll) = row.media.poll.as_mut() {
            change(poll);
        }
    }
    page
}

fn closed(log: Log) -> LogPage {
    with_poll(log, |poll| poll.is_closed = true)
}

fn voted(log: Log) -> LogPage {
    with_poll(log, |poll| poll.has_voted = true)
}

fn voted_for(option: &str) -> Reply {
    let row = poll_row();
    let mut media = row.media;
    let poll = media.poll.as_mut().unwrap();
    poll.has_voted = true;
    poll.total_votes = Some(4);
    for each in &mut poll.options {
        let mine = each.id == option;
        each.voted = mine;
        each.votes_count = Some(if mine { 3 } else { 1 });
        each.percentage = Some(if mine { 75 } else { 25 });
    }
    Reply::Commit(Box::new(Focus {
        card: Card {
            commit: row.commit,
            media,
            stars_count: row.stars_count,
            starred: row.starred,
            quoted_commit: row.quoted_commit,
        },
        replies_count: row.replies_count,
    }))
}

fn vote(option: &str) -> Call {
    Call::Vote {
        id: id(),
        option: option.to_owned(),
    }
}

fn one_run(effects: &[Effect]) -> (ttyl::sched::Ticket, Call) {
    let [Effect::Run(ticket, call)] = effects else {
        panic!("one call goes out: {effects:?}");
    };
    (*ticket, call.clone())
}

/// The option the filled bar is on, or `None` while no poll is being picked in.
fn marked(engine: &mut Engine, at: Instant) -> Option<String> {
    let (width, height) = (100, 40);
    let buffer = buffer(engine, width, height, at);
    (0..height).find_map(|y| {
        let bar: String = (0..width)
            .filter(|x| buffer[(*x, y)].bg == ttyl::theme::PRIMARY)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        (!bar.trim().is_empty()).then(|| bar.trim().to_owned())
    })
}

#[test]
fn v_turns_the_poll_into_a_picker_in_place_with_the_first_option_on() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    assert!(runs(&press(&mut engine, "v", at)).is_empty());
    assert_eq!(marked(&mut engine, at).as_deref(), Some("1  tabs"));
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "2  spaces"), "{rows:#?}");
    assert!(contains(&rows, "votes are final"), "{rows:#?}");
    assert!(!contains(&rows, "v to vote"), "{rows:#?}");
    assert!(!contains(&rows, "┌"), "no box opens: {rows:#?}");
}

#[test]
fn the_hint_line_offers_v_on_an_open_poll_only() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    let rows = screen(&mut engine, 160, 40, at);
    assert!(contains(&rows, "v vote"), "{rows:#?}");
    let mut closed = on_poll_serving(at, closed);
    let rows = screen(&mut closed, 160, 40, at);
    assert!(!contains(&rows, "v vote"), "{rows:#?}");
}

#[test]
fn j_k_and_the_digits_move_the_mark_within_the_options() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    press(&mut engine, "v", at);
    for (keys, option) in [
        ("j", "2  spaces"),
        ("j", "2  spaces"),
        ("k", "1  tabs"),
        ("k", "1  tabs"),
        ("2", "2  spaces"),
        ("4", "2  spaces"),
        ("1", "1  tabs"),
    ] {
        press(&mut engine, keys, at);
        assert_eq!(
            marked(&mut engine, at).as_deref(),
            Some(option),
            "after {keys}"
        );
    }
}

#[test]
fn enter_votes_for_the_marked_option_and_ends_the_pick() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    press(&mut engine, "vj", at);
    let (_, call) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    assert_eq!(call, vote(SPACES));
    assert_eq!(marked(&mut engine, at), None);
}

#[test]
fn esc_backs_out_without_voting() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    press(&mut engine, "v2", at);
    assert!(runs(&tap(&mut engine, KeyCode::Esc, at)).is_empty());
    assert_eq!(marked(&mut engine, at), None);
    assert!(contains(&screen(&mut engine, 100, 40, at), "v to vote"));
}

#[test]
fn the_vote_fills_the_bars_checks_the_choice_and_says_so() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    press(&mut engine, "v", at);
    let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    engine.update(Event::Reply(ticket, Ok(voted_for(TABS))), at);
    engine.pump(at);

    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "voted for tabs"), "{rows:#?}");
    let filled = screen(&mut engine, 100, 40, at + Duration::from_secs(3));
    let tabs = filled
        .iter()
        .find(|row| row.contains("tabs") && row.contains('%'))
        .unwrap_or_else(|| panic!("tabs has a bar: {filled:#?}"));
    assert!(tabs.contains("75%") && tabs.contains('✓'), "{tabs}");
    assert!(!contains(&filled, "v to vote"), "{filled:#?}");
}

#[test]
fn a_refused_vote_shows_why() {
    let at = Instant::now();
    let mut engine = on_poll(at);
    press(&mut engine, "v", at);
    let (ticket, _) = one_run(&tap(&mut engine, KeyCode::Enter, at));
    let error = Error::Api {
        status: 422,
        code: None,
        message: "This poll is closed.".to_owned(),
        retry_after: None,
    };
    engine.update(Event::Reply(ticket, Err(error)), at);
    engine.pump(at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "This poll is closed."), "{rows:#?}");
}

#[test]
fn v_does_nothing_on_a_closed_poll_a_voted_poll_or_your_own() {
    let at = Instant::now();
    let margaret: Account = serde_json::from_value(serde_json::json!({
        "data": { "username": "margaret", "name": "Margaret" },
        "unread_notifications_count": 0,
    }))
    .unwrap();
    let mut own = boot_as(at, margaret, page);
    press(&mut own, "2jjjjjjj", at);
    for (why, mut engine) in [
        ("closed", on_poll_serving(at, closed)),
        ("voted", on_poll_serving(at, voted)),
        ("own", own),
    ] {
        assert!(runs(&press(&mut engine, "v", at)).is_empty(), "{why}");
        assert_eq!(marked(&mut engine, at), None, "{why}");
    }
}
