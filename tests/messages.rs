mod common;

use std::fs;
use std::path::Path;
use std::time::Instant;

use common::{boot, contains, press, runs, screen, secs, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use ttyl::api::{Call, Error, Paging, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{
    Candidate, Chat, ConversationPage, Folder, Hush, Message, MessageId, MessagePage, OrgId, Refile,
};

const ADA: &str = "0a000000-0000-4000-8000-00000000000a";
const GRACE: &str = "0b000000-0000-4000-8000-00000000000b";

fn grace() -> Chat {
    Chat::Direct("grace".to_owned())
}

fn unix_room() -> Chat {
    Chat::Org(OrgId("c1000000-0000-4000-8000-0000000000f0".to_owned()))
}

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

/// The #unix-room org's thread, with `messages` in place of the fixture's.
fn org_thread(messages: Option<Vec<Value>>) -> Reply {
    let mut page = fixture("remote-org-thread");
    if let Some(messages) = messages {
        page["data"]["messages"] = Value::Array(messages);
    }
    Reply::Messages(Box::new(MessagePage::parse(&page.to_string()).unwrap()))
}

/// A message in #unix-room from the member with `handle`, who an org's thread names.
fn said(id: u64, handle: &str, body: &str, at: &str) -> Value {
    let members = fixture("remote-org-thread")["data"]["members"].clone();
    let user = (members.as_array().unwrap().iter())
        .map(|member| member["user"].clone())
        .find(|user| user["handle"] == handle)
        .unwrap();
    let mut said = message(id, user["id"].as_str().unwrap(), body, "", at);
    said["client_id"] = Value::Null;
    said["user"] = user;
    said
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
            chat: Chat::Org(_),
            paging: Paging::Latest,
        } => Some(org_thread(None)),
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

/// gm, then enter on #unix-room, the unread group under grace, and every answer on the way.
fn open_unix_room(engine: &mut Engine, at: Instant) {
    let effects = press(engine, "gm", at);
    answer(engine, &effects, at, dev_null);
    press(engine, "jj", at);
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
        runs(&effects).contains(&&Call::Conversations {
            folder: Folder::Open,
            query: String::new(),
            before: None
        }),
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
    let (room, group) = row(&rows, "#unix-room");
    assert!(
        grace < room && room < linus,
        "a group sits among the people, unread before read: {rows:#?}"
    );
    assert!(
        group.contains("● #unix-room 4 members (2)  grace: pushed the fix for the cursor race")
            && group.trim_end().ends_with("30m"),
        "{group:?}"
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
            chat: grace(),
            paging: Paging::Latest,
        }),
        "{effects:#?}"
    );
    let effects = answer(&mut engine, &effects, at, dev_null);
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            chat: grace(),
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
                chat,
                body,
                client_id,
            },
        ] if *chat == grace() && body == typed => client_id.clone(),
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
        chat: grace(),
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
            chat: grace(),
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
        chat: grace(),
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
            chat: grace(),
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
            chat: Chat::Direct("margaret".to_owned()),
            paging: Paging::Latest,
        }),
        "{effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("@margaret"), "{rows:#?}");
    assert!(rows[28].contains("› "), "{rows:#?}");
}

/// Types `text` into the open conversation and presses enter, returning the send and its id.
fn send(engine: &mut Engine, text: &str, at: Instant) -> (Vec<Effect>, String) {
    press(engine, text, at);
    let effects = tap(engine, KeyCode::Enter, at);
    let client_id = match runs(&effects).as_slice() {
        [Call::SendMessage { client_id, .. }] => client_id.0.clone(),
        other => panic!("enter sends the message: {other:#?}"),
    };
    (effects, client_id)
}

fn fail(engine: &mut Engine, effects: &[Effect], error: fn() -> Error, at: Instant) {
    for effect in effects {
        if let Effect::Run(ticket, Call::SendMessage { .. }) = effect {
            engine.update(Event::Reply(*ticket, Err(error())), at);
        }
    }
}

#[test]
fn a_message_grace_sent_just_before_yours_still_comes_in() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let (effects, client_id) = send(&mut engine, "on it", at);
    let mine = message(107, ADA, "on it", &client_id, "2026-10-21T09:59:00Z");
    answer(&mut engine, &effects, at, |call| {
        matches!(call, Call::SendMessage { .. })
            .then(|| Reply::Message(Box::new(serde_json::from_value(mine.clone()).unwrap())))
    });

    let effects = engine.pump(at + secs(3));
    let poll = Call::Messages {
        chat: grace(),
        paging: Paging::After(MessageId(105)),
    };
    assert!(
        runs(&effects).contains(&&poll),
        "a sent message doesn't move where polling picks up: {effects:#?}"
    );
    let theirs = message(
        106,
        GRACE,
        "wait, one more thing",
        "5a4b3c2d-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-21T09:58:30Z",
    );
    let effects = answer(&mut engine, &effects, at + secs(3), |call| {
        (*call == poll).then(|| thread(Some(vec![theirs.clone(), mine.clone()]), "open"))
    });
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            chat: grace(),
            through: MessageId(107),
        }),
        "{effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at + secs(3));
    assert!(
        row(&rows, "wait, one more thing").0 < row(&rows, "<ada> on it").0,
        "{rows:#?}"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.contains("<ada> on it"))
            .count(),
        1,
        "{rows:#?}"
    );
}

#[test]
fn a_poll_answered_after_the_conversation_reopened_leaves_it_loading() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let stale = engine.pump(at + secs(3));
    tap(&mut engine, KeyCode::Esc, at + secs(3));
    tap(&mut engine, KeyCode::Enter, at + secs(3));
    let arrived = message(
        107,
        GRACE,
        "pushed v2",
        "7c1e9a52-0b1d-4c55-8e41-1f9e2b3c4d5e",
        "2026-10-21T09:59:00Z",
    );
    answer(&mut engine, &stale, at + secs(3), |call| {
        matches!(call, Call::Messages { .. }).then(|| thread(Some(vec![arrived.clone()]), "open"))
    });
    let rows = screen(&mut engine, 100, 30, at + secs(3));
    assert!(
        !contains(&rows, "pushed v2") && !contains(&rows, "the start of your conversation"),
        "the old screen's poll doesn't stand in for the history: {rows:#?}"
    );

    let effects = engine.pump(at + secs(6));
    let latest = Call::Messages {
        chat: grace(),
        paging: Paging::Latest,
    };
    assert!(runs(&effects).contains(&&latest), "{effects:#?}");
    answer(&mut engine, &effects, at + secs(6), dev_null);
    let rows = screen(&mut engine, 100, 30, at + secs(6));
    assert!(contains(&rows, "are you around tomorrow?"), "{rows:#?}");
}

#[test]
fn a_send_that_may_have_landed_goes_again_with_the_same_id() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let (effects, first) = send(&mut engine, "on it", at);
    fail(&mut engine, &effects, || Error::Unexpected(502), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[28].contains("› on it"), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        matches!(runs(&effects).as_slice(), [Call::SendMessage { client_id, .. }] if client_id.0 == first),
        "dev-null.sh keeps one copy if the first did land: {effects:#?}"
    );
    fail(&mut engine, &effects, || Error::Unexpected(502), at);
    press(&mut engine, "!", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        matches!(runs(&effects).as_slice(), [Call::SendMessage { client_id, body, .. }] if client_id.0 != first && body == "on it!"),
        "changed text is a new message: {effects:#?}"
    );
}

#[test]
fn a_send_refused_after_leaving_comes_back_on_reopening() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let (effects, _) = send(&mut engine, "on it", at);
    tap(&mut engine, KeyCode::Esc, at);
    fail(
        &mut engine,
        &effects,
        || Error::Api {
            status: 403,
            code: None,
            message: "@grace hasn't merged your request yet.".to_owned(),
            retry_after: None,
        },
        at,
    );
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, &effects, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[28].contains("› on it"), "{rows:#?}");
}

#[test]
fn a_send_that_fails_after_reopening_comes_back_with_its_id() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_grace(&mut engine, at);
    let (effects, client_id) = send(&mut engine, "on it", at);
    tap(&mut engine, KeyCode::Esc, at);
    let mut reopened = tap(&mut engine, KeyCode::Enter, at);
    while !runs(&reopened).is_empty() {
        reopened = answer(&mut engine, &reopened, at, dev_null);
    }
    fail(&mut engine, &effects, || Error::Unexpected(502), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[28].contains("› on it"), "{rows:#?}");
    let effects = tap(&mut engine, KeyCode::Enter, at);
    match runs(&effects).as_slice() {
        [
            Call::SendMessage {
                client_id: again, ..
            },
        ] => assert_eq!(again.0, client_id),
        other => panic!("enter sends it again: {other:#?}"),
    }
}

#[test]
fn a_group_opens_like_a_channel_with_each_authors_nick_and_marks_it_read() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gm", at);
    answer(&mut engine, &effects, at, dev_null);
    press(&mut engine, "jj", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Messages {
            chat: unix_room(),
            paging: Paging::Latest,
        }),
        "{effects:#?}"
    );
    let effects = answer(&mut engine, &effects, at, dev_null);
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            chat: unix_room(),
            through: MessageId(210),
        }),
        "{effects:#?}"
    );

    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("#unix-room"), "{rows:#?}");
    let (start, _) = row(&rows, "── the start of #unix-room ──");
    let (created, _) = row(&rows, "08:00 -!- margaret created #unix-room");
    let (added, _) = row(&rows, "08:01 -!- margaret added linus");
    let (asked, _) = row(&rows, "15:00 <linus> who owns the cursor code now?");
    let (answered, _) = row(&rows, "15:05   <ada> grace does");
    let (pushed, _) = row(&rows, "09:30 <grace> pushed the fix");
    assert!(
        start < created
            && created < added
            && added < asked
            && asked < answered
            && answered < pushed,
        "{rows:#?}"
    );
    assert!(rows[28].contains("message #unix-room"), "{rows:#?}");

    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!row(&rows, "#unix-room").1.contains("(2)"), "{rows:#?}");
}

#[test]
fn a_group_sends_polls_and_marks_read_on_its_own_routes() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_unix_room(&mut engine, at);
    let (effects, client_id) = send(&mut engine, "reviewing now", at);
    assert!(
        matches!(runs(&effects).as_slice(), [Call::SendMessage { chat, body, .. }] if *chat == unix_room() && body == "reviewing now"),
        "{effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, "reviewing now").1.contains("--:--   <ada> "),
        "{rows:#?}"
    );

    let mine = message(
        211,
        ADA,
        "reviewing now",
        &client_id,
        "2026-10-21T09:58:00Z",
    );
    answer(&mut engine, &effects, at, |call| {
        matches!(call, Call::SendMessage { .. })
            .then(|| Reply::Message(Box::new(serde_json::from_value(mine.clone()).unwrap())))
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, "reviewing now").1.contains("09:58   <ada> "),
        "what dev-null.sh sends back is yours, though it doesn't say who wrote it: {rows:#?}"
    );

    let effects = engine.pump(at + secs(3));
    let poll = Call::Messages {
        chat: unix_room(),
        paging: Paging::After(MessageId(210)),
    };
    assert!(runs(&effects).contains(&&poll), "{effects:#?}");
    let arrived = said(212, "linus", "lgtm, merging", "2026-10-21T09:59:00Z");
    let effects = answer(&mut engine, &effects, at + secs(3), |call| {
        (*call == poll).then(|| org_thread(Some(vec![arrived.clone()])))
    });
    assert!(
        runs(&effects).contains(&&Call::MarkRead {
            chat: unix_room(),
            through: MessageId(212),
        }),
        "{effects:#?}"
    );
    let rows = screen(&mut engine, 100, 30, at + secs(3));
    assert_eq!(
        row(&rows, "<linus> lgtm, merging").0,
        27,
        "the view follows the newest message: {rows:#?}"
    );
}

#[test]
fn a_group_keeps_a_failed_send_and_resends_it_with_the_same_id() {
    let at = Instant::now();
    let mut engine = boot(at);
    open_unix_room(&mut engine, at);
    let (effects, first) = send(&mut engine, "on it", at);
    fail(&mut engine, &effects, || Error::Unexpected(502), at);
    tap(&mut engine, KeyCode::Esc, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, &effects, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("#unix-room"), "{rows:#?}");
    assert!(rows[28].contains("› on it"), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        matches!(runs(&effects).as_slice(), [Call::SendMessage { chat, client_id, .. }] if *chat == unix_room() && client_id.0 == first),
        "{effects:#?}"
    );
}

/// The open folder as the fixture lists it, changed by `edit` first.
fn remotes(edit: impl FnOnce(&mut Value)) -> ConversationPage {
    let mut page = fixture("remotes");
    edit(&mut page);
    ConversationPage::parse(&page.to_string()).unwrap()
}

/// Lists the fixture's conversations, changed by `edit`, then moves down to grace's.
fn on_grace(engine: &mut Engine, at: Instant, edit: impl FnOnce(&mut Value)) {
    let effects = press(engine, "gm", at);
    let page = remotes(edit);
    answer(engine, &effects, at, |call| {
        matches!(call, Call::Conversations { .. }).then(|| Reply::Conversations(page.clone()))
    });
    press(engine, "j", at);
}

fn conversations(folder: Folder, query: &str) -> Call {
    Call::Conversations {
        folder,
        query: query.to_owned(),
        before: None,
    }
}

fn file(change: Refile) -> Call {
    Call::File {
        chat: grace(),
        change,
    }
}

/// ctrl-r, which lists the folder again, and what goes out for it.
fn reload(engine: &mut Engine, at: Instant) -> Vec<Effect> {
    let ctrl_r = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
    let mut effects = engine.update(Event::Key(ctrl_r), at);
    effects.extend(engine.pump(at));
    effects
}

/// Answers what goes out with `serve` until nothing more does, and returns every call that went.
fn settle(
    engine: &mut Engine,
    mut effects: Vec<Effect>,
    at: Instant,
    serve: impl Fn(&Call) -> Option<Reply>,
) -> Vec<Call> {
    let mut ran = Vec::new();
    while !runs(&effects).is_empty() {
        ran.extend(runs(&effects).into_iter().cloned());
        effects = answer(engine, &effects, at, &serve);
    }
    ran
}

/// Someone dev-null.sh offers to start a conversation with, and why they can't be, if they can't.
fn candidate(handle: &str, reason: Option<&str>) -> Value {
    let mut user = fixture("remotes")["data"][0]["partner"].clone();
    user["id"] = json!(format!(
        "0f000000-0000-4000-8000-0000000000{}",
        handle.len()
    ));
    user["handle"] = json!(handle);
    user["name"] = json!(handle.to_uppercase());
    json!({ "user": user, "reason": reason })
}

#[test]
fn tab_turns_the_folders_and_requests_show_only_while_some_wait() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "open") && contains(&rows, "stashed"),
        "{rows:#?}"
    );
    assert!(!contains(&rows, "requests"), "{rows:#?}");
    let effects = tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(runs(&effects), [&conversations(Folder::Stashed, "")]);

    let mut engine = boot(at);
    on_grace(&mut engine, at, |page| page["requests_count"] = json!(2));
    assert!(contains(&screen(&mut engine, 100, 30, at), "requests 2"));
    let effects = tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(runs(&effects), [&conversations(Folder::Requests, "")]);
    let effects = tap(&mut engine, KeyCode::BackTab, at);
    assert_eq!(runs(&effects), [&conversations(Folder::Open, "")]);
}

#[test]
fn caret_pins_to_the_top_at_once_and_a_second_press_waits_for_the_first() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let first = press(&mut engine, "^", at);
    assert_eq!(runs(&first), [&file(Refile::Pin(true))]);
    let rows = screen(&mut engine, 100, 30, at);
    let (grace, pinned) = row(&rows, "@grace");
    assert!(grace < row(&rows, "@linus").0, "{rows:#?}");
    assert!(pinned.contains("pinned"), "{pinned}");
    assert!(
        contains(&rows, "pinned @grace"),
        "the toast says so: {rows:#?}"
    );

    let second = press(&mut engine, "^", at);
    assert!(runs(&second).is_empty(), "{second:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    let (_, unpinned) = row(&rows, "@grace");
    assert!(!unpinned.contains("pinned"), "{unpinned}");
    let next = answer(&mut engine, &first, at, |_| Some(Reply::Done));
    assert_eq!(runs(&next), [&file(Refile::Pin(false))]);
}

#[test]
fn s_stashes_in_place_and_a_second_s_takes_it_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let first = press(&mut engine, "s", at);
    assert_eq!(runs(&first), [&file(Refile::Stash(true))]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, "Grace @grace").1.contains("stashed"),
        "{rows:#?}"
    );
    assert!(contains(&rows, "stashed @grace"), "{rows:#?}");

    let second = press(&mut engine, "s", at);
    assert!(runs(&second).is_empty(), "{second:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    let (_, back) = row(&rows, "Grace @grace");
    assert!(!back.contains("stashed"), "{rows:#?}");
    assert!(contains(&rows, "unstashed @grace"), "{rows:#?}");
    let next = answer(&mut engine, &first, at, |_| Some(Reply::Done));
    assert_eq!(runs(&next), [&file(Refile::Stash(false))]);
}

#[test]
fn s_in_stashed_unstashes_in_place_and_a_second_s_stashes_it_again() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let effects = tap(&mut engine, KeyCode::Tab, at);
    let stashed = remotes(|page| {
        let rows = page["data"].as_array().unwrap();
        page["data"] = json!([rows[1], rows[2]]);
        page["data"][0]["is_archived"] = json!(true);
        page["data"][1]["is_archived"] = json!(true);
    });
    answer(&mut engine, &effects, at, |_| {
        Some(Reply::Conversations(stashed.clone()))
    });
    let first = press(&mut engine, "s", at);
    assert_eq!(runs(&first), [&file(Refile::Stash(false))]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        row(&rows, "Grace @grace").1.contains("unstashed"),
        "{rows:#?}"
    );

    let second = press(&mut engine, "s", at);
    assert!(runs(&second).is_empty(), "{second:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        !row(&rows, "Grace @grace").1.contains("stashed"),
        "{rows:#?}"
    );
    assert!(
        !row(&rows, "Linus @linus").1.contains("stashed"),
        "{rows:#?}"
    );
    let next = answer(&mut engine, &first, at, |_| Some(Reply::Done));
    assert_eq!(runs(&next), [&file(Refile::Stash(true))]);
}

#[test]
fn listing_open_again_drops_a_conversation_stashed_before_dev_null_had_it() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    press(&mut engine, "s", at);
    let effects = reload(&mut engine, at);
    assert!(
        runs(&effects).contains(&&conversations(Folder::Open, "")),
        "{effects:#?}"
    );
    answer(&mut engine, &effects, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "Grace @grace"), "{rows:#?}");
    assert!(contains(&rows, "@linus"), "{rows:#?}");
}

#[test]
fn m_asks_how_long_to_mute_and_unmutes_a_muted_one_at_once() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let effects = press(&mut engine, "m", at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    let (_, prompt) = row(&rows, "mute @grace");
    for choice in [
        "h for an hour",
        "t till tomorrow",
        "w for a week",
        "f forever",
    ] {
        assert!(prompt.contains(choice), "{prompt}");
    }
    let effects = press(&mut engine, "x", at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    let effects = press(&mut engine, "t", at);
    assert_eq!(runs(&effects), [&file(Refile::Mute(Some(Hush::Tomorrow)))]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(row(&rows, "@grace").1.contains("muted"), "{rows:#?}");
    assert!(contains(&rows, "muted @grace till tomorrow"), "{rows:#?}");

    let mut engine = boot(at);
    on_grace(&mut engine, at, |page| {
        page["data"][2]["muted_until_at"] = json!("2999-01-01T00:00:00Z");
    });
    let effects = press(&mut engine, "m", at);
    assert_eq!(runs(&effects), [&file(Refile::Mute(None))]);
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "unmuted @grace"
    ));
}

#[test]
fn m_then_a_mute_length_while_offline_sends_that_length_once_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |page| {
        page["data"][2]["muted_until_at"] = json!("2999-01-01T00:00:00Z");
    });
    let mut effects = reload(&mut engine, at);
    while !runs(&effects).is_empty() {
        let mut next = Vec::new();
        for effect in effects {
            if let Effect::Run(ticket, _) = effect {
                let lost = Error::Http(ureq::Error::ConnectionFailed);
                next.extend(engine.update(Event::Reply(ticket, Err(lost)), at));
            }
        }
        next.extend(engine.pump(at));
        effects = next;
    }
    let effects = press(&mut engine, "mmw", at);
    assert!(
        !(runs(&effects).iter()).any(|call| matches!(call, Call::File { .. })),
        "nothing is written while offline: {effects:#?}"
    );
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "muted @grace for a week"
    ));

    let online = |muted: bool| {
        let page = remotes(|page| {
            if muted {
                page["data"][2]["muted_until_at"] = json!("2999-01-01T00:00:00Z");
            }
        });
        move |call: &Call| match call {
            Call::Conversations { .. } => Some(Reply::Conversations(page.clone())),
            Call::Page(log) => Some(Reply::Page(common::page(*log))),
            Call::File { .. } => Some(Reply::Done),
            _ => None,
        }
    };
    let effects = reload(&mut engine, at);
    let ran = settle(&mut engine, effects, at, online(true));
    let week = file(Refile::Mute(Some(Hush::Week)));
    assert!(ran.contains(&week), "back online: {ran:#?}");

    let effects = reload(&mut engine, at);
    settle(&mut engine, effects, at, online(false));
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        !row(&rows, "@grace").1.contains("muted"),
        "once sent, dev-null.sh's word stands: {rows:#?}"
    );
}

#[test]
fn a_merges_a_request_and_shift_d_drops_one_after_asking() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |page| page["requests_count"] = json!(2));
    let effects = tap(&mut engine, KeyCode::Tab, at);
    let requests = remotes(|page| {
        let rows = page["data"].as_array().unwrap();
        page["data"] = json!([rows[1], rows[2]]);
        page["requests_count"] = json!(2);
    });
    answer(&mut engine, &effects, at, |_| {
        Some(Reply::Conversations(requests.clone()))
    });
    let merge = press(&mut engine, "a", at);
    let merged = Call::Answer {
        handle: "grace".to_owned(),
        merge: true,
    };
    assert_eq!(runs(&merge), [&merged]);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "Grace @grace"), "{rows:#?}");
    assert!(contains(&rows, "requests 1"), "{rows:#?}");

    let effects = press(&mut engine, "Dn", at);
    assert!(runs(&effects).is_empty(), "n keeps it: {effects:#?}");
    let effects = press(&mut engine, "D", at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "drop @linus's request? they aren't told [y/N]"),
        "{rows:#?}"
    );
    let effects = press(&mut engine, "y", at);
    let dropped = Call::Answer {
        handle: "linus".to_owned(),
        merge: false,
    };
    assert_eq!(runs(&effects), [&dropped]);

    let effects = answer(&mut engine, &merge, at, |_| Some(Reply::Done));
    assert!(
        runs(&effects).contains(&&conversations(Folder::Requests, "")),
        "{effects:#?}"
    );
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "merged @grace's request"
    ));
}

#[test]
fn slash_finds_conversations_and_people_and_typing_it_fires_no_key() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    press(&mut engine, "/", at);
    let effects = press(&mut engine, "gras^m", at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    for _ in 0..3 {
        tap(&mut engine, KeyCode::Backspace, at);
    }
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "/gra█"), "{rows:#?}");
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let people = Call::Candidates("gra".to_owned());
    assert_eq!(
        runs(&effects),
        [&conversations(Folder::Open, "gra"), &people]
    );
    let found = remotes(|page| {
        let rows = page["data"].as_array().unwrap();
        page["data"] = json!([rows[2]]);
    });
    let candidates = json!({ "candidates": [
        candidate("grace", None),
        candidate("graham", None),
        candidate("gravity", Some("has remotes off")),
    ] });
    let candidates = Candidate::parse_list(&candidates.to_string()).unwrap();
    answer(&mut engine, &effects, at, |call| match call {
        Call::Conversations { .. } => Some(Reply::Conversations(found.clone())),
        Call::Candidates(_) => Some(Reply::Candidates(candidates.clone())),
        _ => None,
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "/gra"), "{rows:#?}");
    assert!(!contains(&rows, "@linus"), "{rows:#?}");
    let (start, _) = row(&rows, "write to");
    let (graham, _) = row(&rows, "@graham");
    let (gravity, off) = row(&rows, "@gravity");
    assert!(start < graham && graham < gravity, "{rows:#?}");
    assert!(off.contains("has remotes off"), "{off}");
    assert_eq!(
        rows.iter().filter(|row| row.contains("@grace")).count(),
        1,
        "{rows:#?}"
    );

    press(&mut engine, "jj", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(runs(&effects).is_empty(), "{effects:#?}");
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "can't message @gravity: has remotes off"
    ));
    press(&mut engine, "k", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).iter().any(|call| matches!(call, Call::Messages { chat: Chat::Direct(handle), .. } if handle == "graham")),
        "{effects:#?}"
    );

    tap(&mut engine, KeyCode::Esc, at);
    let effects = tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(
        runs(&effects),
        [&conversations(Folder::Open, "")],
        "esc drops the find before leaving"
    );
}

#[test]
fn a_found_person_stays_selected_when_older_conversations_land() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    press(&mut engine, "/", at);
    press(&mut engine, "gra", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let found = remotes(|page| {
        let rows = page["data"].as_array().unwrap();
        page["data"] = json!([rows[2]]);
        page["has_more"] = json!(true);
    });
    let candidates = json!({ "candidates": [candidate("graham", None), candidate("grant", None)] });
    let candidates = Candidate::parse_list(&candidates.to_string()).unwrap();
    let older = answer(&mut engine, &effects, at, |call| match call {
        Call::Conversations { before: None, .. } => Some(Reply::Conversations(found.clone())),
        Call::Candidates(_) => Some(Reply::Candidates(candidates.clone())),
        _ => None,
    });

    press(&mut engine, "G", at);
    let page = remotes(|page| {
        let rows = page["data"].as_array().unwrap();
        page["data"] = json!([rows[1], rows[4]]);
    });
    answer(&mut engine, &older, at, |call| {
        matches!(
            call,
            Call::Conversations {
                before: Some(_),
                ..
            }
        )
        .then(|| Reply::Conversations(page.clone()))
    });
    assert!(contains(&screen(&mut engine, 100, 30, at), "@ken"));
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).iter().any(|call| matches!(call, Call::Messages { chat: Chat::Direct(handle), .. } if handle == "grant")),
        "{effects:#?}"
    );
}

#[test]
fn a_pin_dev_null_refuses_says_why_and_lists_the_folder_again() {
    let at = Instant::now();
    let mut engine = boot(at);
    on_grace(&mut engine, at, |_| {});
    let effects = press(&mut engine, "^", at);
    let mut next = Vec::new();
    for effect in &effects {
        if let Effect::Run(ticket, Call::File { .. }) = effect {
            let refused = Error::Api {
                status: 403,
                code: None,
                message: "You can't pin this conversation.".to_owned(),
                retry_after: None,
            };
            next.extend(engine.update(Event::Reply(*ticket, Err(refused)), at));
        }
    }
    next.extend(engine.pump(at));
    assert_eq!(runs(&next), [&conversations(Folder::Open, "")]);
    answer(&mut engine, &next, at, dev_null);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "You can't pin this conversation."),
        "{rows:#?}"
    );
    assert!(!row(&rows, "@grace").1.contains("pinned"), "{rows:#?}");
}
