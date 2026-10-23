mod common;

use std::time::Instant;

use common::{answer, boot, buffer, commit, contains, press, runs, screen, secs, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Call, Error, Reply};
use ttyl::draft::{Target, Write};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitId, Focus, Segment, Status};
use ttyl::sched::Ticket;
use ttyl::theme;

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";
const POSTED: &str = "2c3d4e5f-0000-4000-8000-000000000999";
const WIDTHS: [u16; 2] = [100, 44];
const HEIGHT: u16 = 30;
const PENDING: &str = "Your commit is pending review. Everyone will see it once it is approved.";

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

fn on_grace(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    engine
}

fn send(engine: &mut Engine, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

fn one_post(effects: &[Effect]) -> (Ticket, Call) {
    let [Effect::Run(ticket, call @ Call::Post { .. })] = effects else {
        panic!("one commit goes out: {effects:?}");
    };
    (*ticket, call.clone())
}

fn posted(body: &str, status: Status, parent: Option<CommitId>) -> Reply {
    let mut card = commit(&grace()).unwrap().commit;
    card.commit.id = CommitId(POSTED.to_owned());
    card.commit.short_hash = "a1b2c3d".to_owned();
    card.commit.body = body.to_owned();
    card.commit.body_segments = vec![Segment::Text {
        text: body.to_owned(),
    }];
    card.commit.preview_segments = None;
    card.commit.status = status;
    card.commit.parent_id = parent;
    card.commit.quoted_commit_id = None;
    card.commit.author.username = "ada".to_owned();
    card.commit.author.name = "Ada".to_owned();
    card.quoted_commit = None;
    card.stars_count = 0;
    card.starred = false;
    Reply::Commit(Box::new(Focus {
        card,
        replies_count: 0,
    }))
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

fn refused(status: u16, message: &str, retry_after: Option<u64>) -> Error {
    Error::Api {
        status,
        code: Some("rate_limited".to_owned()),
        message: message.to_owned(),
        retry_after: retry_after.map(secs),
    }
}

fn on_top(rows: &[String]) -> bool {
    let posted = rows.iter().position(|row| row.ends_with("a1b2c3d"));
    let grace = rows.iter().position(|row| row.ends_with("0000201"));
    posted.is_some() && posted < grace
}

fn panel_title(rows: &[String]) -> Option<String> {
    rows.iter()
        .find(|row| row.starts_with(" commit ─") || row.starts_with(" committing… ─"))
        .cloned()
}

#[test]
fn ctrl_s_sends_the_draft_once_and_locks_the_panel_until_it_lands() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "chello", at);
    let (_, call) = one_post(&send(&mut engine, at));
    assert_eq!(
        call,
        Call::Post {
            target: Target::New,
            body: "hello".to_owned()
        }
    );
    assert!(
        send(&mut engine, at).is_empty(),
        "a second ctrl-s sends nothing"
    );
    assert!(press(&mut engine, "x", at).is_empty(), "editing is locked");
    assert!(tap(&mut engine, KeyCode::Backspace, at).is_empty());
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        let title = panel_title(&rows).unwrap();
        assert!(title.starts_with(" committing… ─"), "{rows:#?}");
        assert!(contains(&rows, " hello"), "{rows:#?}");
        assert!(
            rows[usize::from(HEIGHT) - 1].starts_with(" esc close"),
            "{rows:#?}"
        );
    }
    assert_eq!(engine.draft(&Target::New), Some("hello"));
}

#[test]
fn ctrl_enter_sends_where_the_terminal_reports_it() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "chi", at);
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    one_post(&effects);
}

#[test]
fn a_new_commit_closes_the_panel_drops_the_draft_and_goes_on_top_of_latest_and_following() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "cfirst line\nsecond", at);
    let (ticket, call) = one_post(&send(&mut engine, at));
    let effects = reply(
        &mut engine,
        ticket,
        Ok(posted("first line\nsecond", Status::Approved, None)),
        at,
    );
    assert_eq!(effects, [Effect::Draft(Write::Drop(Target::New))]);
    assert_eq!(engine.draft(&Target::New), None);
    assert_eq!(
        engine.log().unwrap().selected(),
        3,
        "grace stays selected below the new commit"
    );
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert_eq!(panel_title(&rows), None, "{rows:#?}");
        assert!(contains(&rows, "[log a1b2c3d] first line"), "{rows:#?}");
        assert!(
            rows.iter()
                .filter(|row| row.contains("[log a1b2c3d]"))
                .all(|row| !row.contains("second")),
            "the toast keeps the first line: {rows:#?}"
        );
    }
    assert!(matches!(call, Call::Post { .. }));
    for log in ["2", "1"] {
        press(&mut engine, log, at);
        press(&mut engine, "gg", at);
        let rows = screen(&mut engine, 100, 60, at);
        assert!(
            rows.iter().take(8).any(|row| row.ends_with("a1b2c3d")),
            "{log} shows it on top: {rows:#?}"
        );
    }
}

#[test]
fn a_pending_commit_says_it_waits_for_review() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "chello", at);
    let (ticket, _) = one_post(&send(&mut engine, at));
    reply(
        &mut engine,
        ticket,
        Ok(posted("hello", Status::Pending, None)),
        at,
    );
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, PENDING), "{rows:#?}");
    let rows = screen(&mut engine, 44, HEIGHT, at);
    assert!(contains(&rows, "Your commit is pending"), "{rows:#?}");
}

#[test]
fn a_reply_counts_on_its_parent_and_joins_the_end_of_its_loaded_replies() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    let before = screen(&mut engine, 100, HEIGHT, at);
    let count = |rows: &[String]| -> u32 {
        let row = rows
            .iter()
            .position(|row| row.ends_with("0000201"))
            .unwrap();
        let line = rows[row..]
            .iter()
            .find(|row| row.contains(theme::icons().reply))
            .unwrap()
            .clone();
        let after = line.split(theme::icons().reply).nth(1).unwrap().trim();
        after.split_whitespace().next().unwrap().parse().unwrap()
    };
    let replies = count(&before);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    press(&mut engine, "rmy reply", at);
    let (ticket, call) = one_post(&send(&mut engine, at));
    assert_eq!(
        call,
        Call::Post {
            target: Target::Reply(grace()),
            body: "my reply".to_owned()
        }
    );
    reply(
        &mut engine,
        ticket,
        Ok(posted("my reply", Status::Approved, Some(grace()))),
        at,
    );
    assert_eq!(engine.draft(&Target::Reply(grace())), None);
    let thread = engine.thread().unwrap();
    assert_eq!(thread.focus(), &grace());
    let rows = screen(&mut engine, 100, 60, at);
    let earlier = rows.iter().position(|row| row.ends_with("000004e"));
    let posted = rows.iter().position(|row| row.ends_with("a1b2c3d"));
    assert!(
        earlier.is_some() && posted > earlier,
        "the reply joins the end of the loaded replies: {rows:#?}"
    );
    assert!(
        contains(&rows, &format!("{} {}", theme::icons().reply, replies + 1)),
        "{rows:#?}"
    );
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert_eq!(count(&rows), replies + 1, "{rows:#?}");
}

#[test]
fn a_reader_at_the_head_of_latest_lands_on_the_new_commit() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2chello", at);
    let (ticket, _) = one_post(&send(&mut engine, at));
    reply(
        &mut engine,
        ticket,
        Ok(posted("hello", Status::Approved, None)),
        at,
    );
    assert_eq!(engine.log().unwrap().selected(), 0);
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(contains(&rows, " ┃ hello"), "{rows:#?}");
    }
}

#[test]
fn a_quote_goes_on_top_of_latest() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "qquoted you", at);
    let (ticket, call) = one_post(&send(&mut engine, at));
    assert_eq!(
        call,
        Call::Post {
            target: Target::Quote(grace()),
            body: "quoted you".to_owned()
        }
    );
    reply(
        &mut engine,
        ticket,
        Ok(posted("quoted you", Status::Approved, None)),
        at,
    );
    assert_eq!(engine.log().unwrap().selected(), 3);
    press(&mut engine, "gg", at);
    let rows = screen(&mut engine, 100, 60, at);
    assert!(on_top(&rows), "{rows:#?}");
}

#[test]
fn an_empty_or_too_long_body_sends_nothing_and_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    assert!(send(&mut engine, at).is_empty());
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(
        contains(&rows, "nothing to commit. the body is empty."),
        "{rows:#?}"
    );
    engine.update(Event::Paste("x".repeat(4001)), at);
    assert!(send(&mut engine, at).is_empty());
    for width in WIDTHS {
        let rows = screen(&mut engine, width, HEIGHT, at);
        let status = &rows[usize::from(HEIGHT) - 2];
        assert!(status.contains("1 over the limit"), "{width}: {rows:#?}");
        assert!(status.ends_with("-1 left"), "{width}: {rows:#?}");
    }
    assert_eq!(engine.draft(&Target::New).map(str::len), Some(4001));
}

#[test]
fn the_counter_shows_from_200_left_and_turns_red_past_the_limit() {
    let at = Instant::now();
    for (length, shown, red) in [
        (3799, None, false),
        (3800, Some("200 left"), false),
        (4000, Some("0 left"), false),
        (4003, Some("-3 left"), true),
    ] {
        let mut engine = on_grace(at);
        press(&mut engine, "c", at);
        engine.update(Event::Paste("y".repeat(length)), at);
        for width in WIDTHS {
            let rows = screen(&mut engine, width, HEIGHT, at);
            let row = usize::from(HEIGHT) - 2;
            match shown {
                Some(shown) => {
                    assert!(rows[row].ends_with(shown), "{length} at {width}: {rows:#?}")
                }
                None => assert!(
                    !rows[row].contains("left"),
                    "{length} at {width}: {rows:#?}"
                ),
            }
            if shown.is_some() {
                let buffer = buffer(&mut engine, width, HEIGHT, at);
                let x = u16::try_from(rows[row].chars().count() - 1).unwrap();
                let fg = buffer[(x, u16::try_from(row).unwrap())].fg;
                assert_eq!(fg == theme::DESTRUCTIVE, red, "{length} at {width}");
            }
        }
    }
}

#[test]
fn the_fold_hint_shows_once_past_280_characters_or_8_lines() {
    let at = Instant::now();
    let hint = "the feed folds your commit here";
    for (text, after) in [
        ("z".repeat(281), None),
        (
            (1..=9)
                .map(|n| format!("line {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
            Some(" line 8"),
        ),
    ] {
        let mut engine = on_grace(at);
        press(&mut engine, "c", at);
        engine.update(Event::Paste(text.clone()), at);
        for width in WIDTHS {
            let rows = screen(&mut engine, width, HEIGHT, at);
            let hints: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.contains(hint))
                .map(|(at, _)| at)
                .collect();
            assert_eq!(hints.len(), 1, "{width}: {rows:#?}");
            if let Some(after) = after {
                assert_eq!(rows[hints[0] - 1], after, "{width}: {rows:#?}");
                assert_eq!(rows[hints[0] + 1], " line 9", "{width}: {rows:#?}");
            }
        }
    }
    let mut engine = on_grace(at);
    press(&mut engine, "c", at);
    engine.update(Event::Paste("z".repeat(280)), at);
    assert!(!contains(&screen(&mut engine, 100, HEIGHT, at), hint));
}

#[test]
fn a_failed_commit_keeps_the_draft_and_is_never_sent_again() {
    let at = Instant::now();
    for error in [
        refused(429, "Too many commits. Try again in a minute.", Some(5)),
        refused(500, "Server error", None),
        refused(403, "You are timed out.", None),
    ] {
        let mut engine = on_grace(at);
        press(&mut engine, "chello", at);
        let (ticket, _) = one_post(&send(&mut engine, at));
        let message = error.to_string();
        let effects = reply(&mut engine, ticket, Err(error), at);
        assert!(runs(&effects).is_empty(), "{effects:?}");
        let rows = screen(&mut engine, 100, HEIGHT, at);
        assert!(panel_title(&rows).is_some_and(|title| title.starts_with(" commit ─")));
        assert!(contains(&rows, &message), "{rows:#?}");
        assert_eq!(engine.draft(&Target::New), Some("hello"));
        for later in [1, 6, 60, 600] {
            let effects = engine.update(Event::Tick, at + secs(later));
            let mut effects = effects;
            effects.extend(engine.pump(at + secs(later)));
            assert!(
                !runs(&effects)
                    .iter()
                    .any(|call| matches!(call, Call::Post { .. })),
                "{message} is not resent after {later}s: {effects:?}"
            );
        }
    }
}

#[test]
fn esc_while_sending_leaves_the_draft_and_the_answer_still_lands() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "chello", at);
    let (ticket, _) = one_post(&send(&mut engine, at));
    tap(&mut engine, KeyCode::Esc, at);
    let effects = reply(
        &mut engine,
        ticket,
        Ok(posted("hello", Status::Approved, None)),
        at,
    );
    assert_eq!(effects, [Effect::Draft(Write::Drop(Target::New))]);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "[log a1b2c3d] hello"), "{rows:#?}");
}
