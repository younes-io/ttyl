mod common;

use std::fs;
use std::path::Path;
use std::time::Instant;

use common::{boot, contains, press, runs, screen, secs, tap};
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};
use ttyl::api::{Call, Paging, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{ConversationPage, Message, MessageId, MessagePage};

const ADA: &str = "0a000000-0000-4000-8000-00000000000a";
const GRACE: &str = "0b000000-0000-4000-8000-00000000000b";

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn message(id: u64, from: &str, body: &str, client_id: &str, at: &str) -> Value {
    json!({
        "id": id, "user_id": from, "client_id": client_id, "body": body,
        "images": [], "voice": null, "event": null, "created_at": at,
    })
}

/// Grace's conversation, with `messages` in place of the fixture's and dev-null.sh letting ada
/// write as `access` says.
fn thread(messages: Option<Vec<Value>>, access: &str) -> Reply {
    let mut page = fixture("remote-thread");
    if let Some(messages) = messages {
        page["data"]["messages"] = Value::Array(messages);
    }
    page["data"]["access"] = json!(access);
    Reply::Messages(Box::new(MessagePage::parse(&page.to_string()).unwrap()))
}

/// Answers each call among `effects` that `serve` knows, and returns everything the engine did next.
fn answer(
    engine: &mut Engine,
    effects: &[Effect],
    at: Instant,
    serve: impl Fn(&Call) -> Option<Reply>,
) -> Vec<Effect> {
    let mut next = Vec::new();
    for effect in effects {
        if let Effect::Run(ticket, call) = effect
            && let Some(reply) = serve(call)
        {
            next.extend(engine.update(Event::Reply(*ticket, Ok(reply)), at));
        }
    }
    next.extend(engine.pump(at));
    next
}

fn dev_null(call: &Call) -> Option<Reply> {
    match call {
        Call::Conversations { .. } => Some(Reply::Conversations(
            ConversationPage::parse(&fixture("remotes").to_string()).unwrap(),
        )),
        Call::Messages {
            paging: Paging::Latest,
            ..
        } => Some(thread(None, "open")),
        Call::MarkRead { .. } => Some(Reply::Done),
        _ => None,
    }
}

/// gm, then enter on grace's conversation, the second after the pinned one, and every answer
/// dev-null.sh gives on the way.
fn open_grace(engine: &mut Engine, at: Instant) {
    let effects = press(engine, "gm", at);
    answer(engine, &effects, at, dev_null);
    press(engine, "j", at);
    let mut effects = tap(engine, KeyCode::Enter, at);
    while !runs(&effects).is_empty() {
        effects = answer(engine, &effects, at, dev_null);
    }
}

fn row<'a>(rows: &'a [String], needle: &str) -> (usize, &'a str) {
    rows.iter()
        .enumerate()
        .find(|(_, row)| row.contains(needle))
        .map(|(at, row)| (at, row.as_str()))
        .unwrap_or_else(|| panic!("no row shows {needle:?}: {rows:#?}"))
}

#[test]
fn gm_lists_conversations_pinned_then_unread_one_line_each() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    assert!(
        runs(&effects).contains(&&Call::Conversations { before: None }),
        "{effects:#?}"
    );
    answer(&mut engine, &effects, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("messages"), "{rows:#?}");

    let (margaret, pinned) = row(&rows, "@margaret");
    let (grace, unread) = row(&rows, "@grace");
    let (linus, image) = row(&rows, "@linus");
    let (ken, voice) = row(&rows, "@ken");
    assert!(
        margaret < grace && grace < linus && linus < ken,
        "pinned first, then unread, then dev-null.sh's newest first: {rows:#?}"
    );
    assert!(pinned.contains("you: the lock order is in the commit message now"));
    assert!(pinned.trim_end().ends_with("pinned 1d"), "{pinned:?}");
    assert!(
        unread.contains("Grace @grace (2)  did you see the cursor patch?")
            && unread.trim_end().ends_with("20m"),
        "{unread:?}"
    );
    assert!(!contains(&rows, "off-by-one"), "only the first line shows");
    assert!(image.contains("[image]"), "{image:?}");
    assert!(voice.contains("[voice note]"), "{voice:?}");
    assert!(
        !contains(&rows, "unix-room"),
        "org conversations stay on the web"
    );
}

#[test]
fn enter_opens_the_conversation_oldest_first_and_marks_it_read() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    answer(&mut engine, &effects, at, dev_null);
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Messages {
            handle: "grace".to_owned(),
            paging: Paging::Latest,
        }),
        "{effects:#?}"
    );
    let effects = answer(&mut engine, &effects, at, dev_null);
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            handle: "grace".to_owned(),
            through: MessageId(105),
        }),
        "opening marks it read through the newest message: {effects:#?}"
    );

    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("@grace"), "{rows:#?}");
    let (start, _) = row(&rows, "the start of your conversation with @grace");
    let (asked, asking) = row(&rows, "are you around tomorrow?");
    let (answered, answer) = row(&rows, "sure, after the standup");
    let (sent, image) = row(&rows, "[image]");
    let (latest, patch) = row(&rows, "did you see the cursor patch?");
    let (wrapped, rest) = row(&rows, "it fixes the off-by-one in wrap");
    assert!(start < asked && asked < answered && answered < sent && sent < latest);
    assert!(asking.contains("<grace> are you around"), "{asking:?}");
    assert!(answer.contains("  <ada> sure"), "nicks line up: {answer:?}");
    assert!(patch.contains("09:40 <grace> did you see"), "{patch:?}");
    assert_eq!(wrapped, latest + 1);
    assert!(
        rest.starts_with(&" ".repeat("09:40 <grace> ".len())),
        "a message's lines indent under its text: {rest:?}"
    );
    assert!(
        image.contains("<grace> [image]"),
        "an image shows as a mark: {image:?}"
    );
    assert!(
        row(&rows, "── Yesterday ──").0 < asked && row(&rows, "── Today ──").0 > answered,
        "{rows:#?}"
    );
    assert!(
        rows[28].contains("› ") && rows[28].contains("message @grace"),
        "{rows:#?}"
    );
    assert_eq!(
        wrapped, 27,
        "the newest message sits just above the input line: {rows:#?}"
    );

    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!row(&rows, "@grace").1.contains("(2)"), "{rows:#?}");
}

#[test]
fn enter_sends_what_was_typed_and_it_shows_once_dev_null_has_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);

    let typed = "quick q: yes, merging it now";
    let effects = press(&mut engine, typed, at);
    assert!(
        runs(&effects).is_empty(),
        "keys go to the input: {effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[28].contains(typed), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    let client_id = match runs(&effects).as_slice() {
        [
            Call::SendMessage {
                handle,
                body,
                client_id,
            },
        ] if handle == "grace" && body == typed => client_id.clone(),
        other => panic!("enter sends the message: {other:#?}"),
    };
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, typed).1.contains("--:--   <ada> "),
        "it waits dimmed: {rows:#?}"
    );
    assert!(rows[28].contains("message @grace"), "the input empties");

    let landed: Message = serde_json::from_value(message(
        106,
        ADA,
        typed,
        &client_id.0,
        "2026-10-21T09:58:00Z",
    ))
    .unwrap();
    for effect in effects {
        if let Effect::Run(ticket, Call::SendMessage { .. }) = effect {
            engine.update(
                Event::Reply(ticket, Ok(Reply::Message(Box::new(landed.clone())))),
                at,
            );
        }
    }
    let rows = screen(&mut engine, 100, 30, at);
    assert!(row(&rows, typed).1.contains("09:58   <ada> "), "{rows:#?}");
    assert!(!contains(&rows, "--:--"), "{rows:#?}");
}

#[test]
fn what_was_typed_stays_when_the_conversation_is_left_and_opened_again() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    press(&mut engine, "half a thought", at);
    tap(&mut engine, KeyCode::Esc, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, &effects, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[28].contains("› half a thought"), "{rows:#?}");
}

#[test]
fn an_open_conversation_brings_in_new_messages_as_they_arrive() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);

    let effects = engine.pump(at + secs(3));
    let poll = Call::Messages {
        handle: "grace".to_owned(),
        paging: Paging::After(MessageId(105)),
    };
    assert!(runs(&effects).contains(&&poll), "{effects:#?}");
    let arrived = message(
        107,
        GRACE,
        "pushed v2 with the test",
        "7c1e9a52-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-21T09:59:00Z",
    );
    let effects = answer(&mut engine, &effects, at + secs(3), |call| {
        (*call == poll).then(|| thread(Some(vec![arrived.clone()]), "open"))
    });
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            handle: "grace".to_owned(),
            through: MessageId(107),
        }),
        "what arrives while it is open is read: {effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at + secs(3));
    assert_eq!(
        row(&rows, "pushed v2").0,
        27,
        "the view follows the newest message: {rows:#?}"
    );
    assert!(row(&rows, "did you see the cursor patch?").0 < 27);

    tap(&mut engine, KeyCode::Esc, at + secs(3));
    let effects = engine.pump(at + secs(30));
    assert!(
        !runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Messages { .. })),
        "a closed conversation is not polled: {effects:#?}"
    );
}

#[test]
fn scrolled_back_the_view_stays_put_and_says_what_came_in_below() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let before = screen(&mut engine, 100, 8, at);
    assert!(contains(&before, "it fixes the off-by-one"), "{before:#?}");

    tap(&mut engine, KeyCode::PageUp, at);
    let back = screen(&mut engine, 100, 8, at);
    assert!(
        contains(&back, "are you around tomorrow?") && !contains(&back, "off-by-one"),
        "pgup shows older messages: {back:#?}"
    );

    let effects = engine.pump(at + secs(3));
    let arrived = message(
        107,
        GRACE,
        "pushed v2",
        "7c1e9a52-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-21T09:59:00Z",
    );
    answer(&mut engine, &effects, at + secs(3), |call| {
        matches!(call, Call::Messages { .. }).then(|| thread(Some(vec![arrived.clone()]), "open"))
    });
    let rows = screen(&mut engine, 100, 8, at + secs(3));
    assert_eq!(rows[2..5], back[2..5], "the view stays where it was read");
    assert!(rows[5].contains("1 new message below · pgdn"), "{rows:#?}");

    tap(&mut engine, KeyCode::PageDown, at + secs(3));
    tap(&mut engine, KeyCode::PageDown, at + secs(3));
    let rows = screen(&mut engine, 100, 8, at + secs(3));
    assert!(rows[5].contains("<grace> pushed v2"), "{rows:#?}");
}

#[test]
fn reaching_the_oldest_loaded_message_loads_the_page_before_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    answer(&mut engine, &effects, at, dev_null);
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, &effects, at, |call| match call {
        Call::Messages { .. } => {
            let Reply::Messages(mut page) = thread(None, "open") else {
                unreachable!()
            };
            page.has_older = true;
            Some(Reply::Messages(page))
        }
        other => dev_null(other),
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "── older messages above ──"), "{rows:#?}");

    let effects = engine.pump(at);
    let older = Call::Messages {
        handle: "grace".to_owned(),
        paging: Paging::Before(MessageId(101)),
    };
    assert!(runs(&effects).contains(&&older), "{effects:#?}");
    let first = message(
        99,
        GRACE,
        "hi! loved the talk on wrap",
        "1d2c3b4a-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-20T15:00:00Z",
    );
    answer(&mut engine, &effects, at, |call| {
        (*call == older).then(|| thread(Some(vec![first.clone()]), "open"))
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, "the start of your conversation with @grace").0
            < row(&rows, "hi! loved the talk on wrap").0
            && row(&rows, "hi! loved the talk on wrap").0 < row(&rows, "are you around").0,
        "{rows:#?}"
    );
}

#[test]
fn a_message_that_comes_while_the_last_mark_flies_is_marked_read_after_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    answer(&mut engine, &effects, at, dev_null);
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let flying = answer(&mut engine, &effects, at, |call| {
        matches!(call, Call::Messages { .. }).then(|| thread(None, "open"))
    });
    assert!(
        runs(&flying)
            .iter()
            .any(|call| matches!(call, Call::MarkRead { .. })),
        "{flying:#?}"
    );

    let effects = engine.pump(at + secs(3));
    let arrived = message(
        107,
        GRACE,
        "pushed v2",
        "7c1e9a52-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-21T09:59:00Z",
    );
    answer(&mut engine, &effects, at + secs(3), |call| {
        matches!(call, Call::Messages { .. }).then(|| thread(Some(vec![arrived.clone()]), "open"))
    });
    let effects = answer(&mut engine, &flying, at + secs(3), dev_null);
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            handle: "grace".to_owned(),
            through: MessageId(107),
        }),
        "{effects:#?}"
    );
}

#[test]
fn a_conversation_dev_null_closes_says_why_and_sends_nothing() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    answer(&mut engine, &effects, at, dev_null);
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, &effects, at, |call| match call {
        Call::Messages { .. } => Some(thread(None, "waiting")),
        other => dev_null(other),
    });
    press(&mut engine, "hello", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[28].contains("@grace hasn't merged your request yet."),
        "{rows:#?}"
    );
}

#[test]
fn capital_m_on_a_commit_opens_the_conversation_with_its_author() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "2jjjjM", at);
    assert!(
        runs(&effects).contains(&&Call::Messages {
            handle: "margaret".to_owned(),
            paging: Paging::Latest,
        }),
        "{effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("@margaret"), "{rows:#?}");
    assert!(rows[28].contains("› "), "{rows:#?}");
}
