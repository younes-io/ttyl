mod common;

use std::time::Instant;

use common::{account, contains, page, press, screen, sign_in, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::code;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::locale;
use ttyl::model::{self, CommitId, Segment, Settings, Translation};
use ttyl::sched::Ticket;

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";
const ORIGINAL: &str = "today's footgun";
const TRANSLATED: &str = "le piège du jour";

fn on_grace(at: Instant, device: Option<&str>) -> Engine {
    let engine = Engine::new("ttyl".to_owned(), at).with_language(device.map(str::to_owned));
    let mut engine = sign_in(engine, at, account(3), page);
    press(&mut engine, "2jj", at);
    engine
}

fn translate(language: &str) -> Call {
    Call::Translate {
        id: CommitId(GRACE.to_owned()),
        language: language.to_owned(),
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

fn settings(language: Option<&str>) -> Reply {
    let languages: Vec<_> = ["de", "en", "fr", "pt"]
        .map(|code| serde_json::json!({ "code": code, "name": code, "native_name": code }))
        .into();
    let json = serde_json::json!({ "data": {
        "translation_language": language,
        "translation_languages": languages,
    }});
    Reply::Settings(Settings::parse(&json.to_string()).unwrap())
}

fn translation(json: serde_json::Value) -> Reply {
    Reply::Translation(Box::new(Translation::parse(&json.to_string()).unwrap()))
}

fn french() -> Reply {
    translation(serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "fr",
        "source_name": "French",
        "available": true,
        "translated": {
            "body_segments": [{ "type": "text", "text": TRANSLATED }],
            "preview_segments": null,
        },
    }}))
}

fn english() -> Reply {
    translation(serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "en",
        "source_name": "English",
        "available": false,
        "translated": null,
    }}))
}

fn unavailable() -> Error {
    Error::Api {
        status: 503,
        code: None,
        message: "Could not translate this commit. Try again.".to_owned(),
        retry_after: None,
    }
}

/// Presses `t` on grace's commit and answers the settings, which say nothing, so the call that
/// goes out next is the translation into the device's language.
fn ask(engine: &mut Engine, at: Instant) -> (Ticket, Call) {
    let (ticket, call) = one_run(&press(engine, "t", at));
    assert_eq!(call, Call::Settings);
    let effects = reply(engine, ticket, Ok(settings(None)), at);
    one_run(&effects)
}

#[test]
fn t_swaps_the_body_for_its_translation_in_place() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, call) = ask(&mut engine, at);
    assert_eq!(call, translate("en"));
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "translating…"), "{rows:#?}");
    assert!(
        contains(&rows, ORIGINAL),
        "the original stays while it runs"
    );

    reply(&mut engine, ticket, Ok(french()), at);
    let rows = screen(&mut engine, 100, 40, at);
    let at_header = rows
        .iter()
        .position(|row| row.contains("translated from French · t original"))
        .unwrap_or_else(|| panic!("the header names the language: {rows:#?}"));
    assert!(rows[at_header].contains("@grace"), "{rows:#?}");
    assert!(
        rows[at_header + 1].contains(TRANSLATED),
        "the body sits where the original was: {rows:#?}"
    );
    assert!(contains(&rows, TRANSLATED), "{rows:#?}");
    assert!(!contains(&rows, ORIGINAL), "{rows:#?}");
    assert!(!contains(&rows, "translating…"), "{rows:#?}");
}

#[test]
fn t_again_shows_the_original_and_back_without_asking_twice() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, _) = ask(&mut engine, at);
    reply(&mut engine, ticket, Ok(french()), at);

    assert!(
        press(&mut engine, "t", at).is_empty(),
        "the original is kept"
    );
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, ORIGINAL), "{rows:#?}");
    assert!(!contains(&rows, "translated from"), "{rows:#?}");

    assert!(
        press(&mut engine, "t", at).is_empty(),
        "the translation is kept"
    );
    assert!(contains(&screen(&mut engine, 100, 40, at), TRANSLATED));
}

#[test]
fn the_translation_stays_on_through_a_thread_and_back() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, _) = ask(&mut engine, at);
    reply(&mut engine, ticket, Ok(french()), at);

    let effects = tap(&mut engine, KeyCode::Enter, at);
    common::answer(&mut engine, effects, at);
    let rows = screen(&mut engine, 80, 40, at);
    assert!(contains(&rows, TRANSLATED), "{rows:#?}");
    assert!(contains(&rows, "translated from French"), "{rows:#?}");

    tap(&mut engine, KeyCode::Esc, at);
    assert!(contains(&screen(&mut engine, 80, 40, at), TRANSLATED));
}

#[test]
fn a_pipe_reads_the_translation_while_it_shows() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, _) = ask(&mut engine, at);
    reply(&mut engine, ticket, Ok(french()), at);
    press(&mut engine, "|cat", at);
    assert_eq!(
        tap(&mut engine, KeyCode::Enter, at),
        vec![Effect::Pipe {
            command: "cat".to_owned(),
            input: TRANSLATED.to_owned(),
        }]
    );
}

#[test]
fn a_commit_already_in_the_language_says_so_and_is_not_asked_again() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, _) = ask(&mut engine, at);
    reply(&mut engine, ticket, Ok(english()), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(rows[39], " already in English");
    assert!(contains(&rows, ORIGINAL), "{rows:#?}");
    assert!(!contains(&rows, "translat"), "{rows:#?}");

    assert!(press(&mut engine, "t", at).is_empty(), "known already");
    assert_eq!(screen(&mut engine, 100, 40, at)[39], " already in English");
}

#[test]
fn a_failed_translation_says_why_and_t_tries_again() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    let (ticket, _) = ask(&mut engine, at);
    reply(&mut engine, ticket, Err(unavailable()), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert_eq!(rows[39], " Could not translate this commit. Try again.");
    assert!(!contains(&rows, "translating…"), "{rows:#?}");

    let effects = press(&mut engine, "t", at);
    let (ticket, call) = one_run(&effects);
    assert_eq!(
        call,
        translate("en"),
        "the settings are not asked for again"
    );
    reply(&mut engine, ticket, Ok(french()), at);
    assert!(contains(&screen(&mut engine, 100, 40, at), TRANSLATED));
}

/// The language the first translation goes out in, given the device's and the settings' answer.
fn language_asked(device: Option<&str>, answer: Result<Reply, Error>) -> Call {
    let at = Instant::now();
    let mut engine = on_grace(at, device);
    let (ticket, call) = one_run(&press(&mut engine, "t", at));
    assert_eq!(call, Call::Settings);
    let effects = reply(&mut engine, ticket, answer, at);
    one_run(&effects).1
}

#[test]
fn the_language_picked_on_dev_null_wins_over_the_device() {
    assert_eq!(
        language_asked(Some("de"), Ok(settings(Some("pt-BR")))),
        translate("pt")
    );
}

#[test]
fn the_device_language_follows_when_none_is_picked_or_settings_are_refused() {
    assert_eq!(
        language_asked(Some("de"), Ok(settings(None))),
        translate("de")
    );
    assert_eq!(
        language_asked(Some("de"), Err(common::not_found())),
        translate("de")
    );
}

#[test]
fn a_device_language_dev_null_cannot_translate_into_falls_back_to_english() {
    assert_eq!(
        language_asked(Some("tl"), Ok(settings(None))),
        translate("en")
    );
}

#[test]
fn settings_lost_on_the_network_are_asked_for_again_on_the_next_t() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("de"));
    let (ticket, call) = one_run(&press(&mut engine, "t", at));
    assert_eq!(call, Call::Settings);
    let lost = Error::Http(ureq::Error::ConnectionFailed);
    assert!(
        reply(&mut engine, ticket, Err(lost), at).is_empty(),
        "nothing loops on the failure"
    );
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "translating…"), "{rows:#?}");

    assert!(
        press(&mut engine, "t", at).is_empty(),
        "nothing goes out while offline"
    );
    let later = at + common::secs(30);
    let poll = engine.pump(later);
    let me = poll
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(ticket, Call::Me) => Some(*ticket),
            _ => None,
        })
        .expect("the poll checks the connection");
    let back = reply(&mut engine, me, Ok(Reply::Account(account(3))), later);
    let (ticket, call) = one_run(&back);
    assert_eq!(call, Call::Settings, "asked again once back online");
    let effects = reply(&mut engine, ticket, Ok(settings(Some("fr"))), later);
    assert_eq!(
        one_run(&effects).1,
        translate("fr"),
        "the picked language wins"
    );

    let at = Instant::now();
    let mut engine = on_grace(at, Some("de"));
    let (ticket, _) = one_run(&press(&mut engine, "t", at));
    reply(&mut engine, ticket, Err(Error::Unexpected(502)), at);
    let (_, call) = one_run(&press(&mut engine, "t", at));
    assert_eq!(call, Call::Settings, "a 502 is asked again right away");
}

#[test]
fn space_expands_a_translated_reply_that_shows_more() {
    let at = Instant::now();
    let mut engine = on_grace(at, Some("en"));
    press(&mut engine, "kk", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    common::answer(&mut engine, effects, at);
    press(&mut engine, "j", at);
    let (ticket, _) = ask(&mut engine, at);
    let long = translation(serde_json::json!({ "translation": {
        "language": "en",
        "source_language": "fr",
        "source_name": "French",
        "available": true,
        "translated": {
            "body_segments": [{ "type": "text", "text": "the whole reply, every last word" }],
            "preview_segments": [{ "type": "text", "text": "the start of the reply" }],
        },
    }}));
    reply(&mut engine, ticket, Ok(long), at);
    let rows = screen(&mut engine, 80, 40, at);
    assert!(contains(&rows, "show more"), "{rows:#?}");

    tap(&mut engine, KeyCode::Char(' '), at);
    let rows = screen(&mut engine, 80, 40, at);
    assert!(contains(&rows, "every last word"), "{rows:#?}");
    assert!(!contains(&rows, "show more"), "{rows:#?}");
}

#[test]
fn a_code_block_fence_outruns_the_backticks_inside_it() {
    let line = |text: &str| {
        vec![code::Span {
            text: text.to_owned(),
            hl: None,
        }]
    };
    let block = Segment::Block {
        language: Some("md".to_owned()),
        lines: vec![line("```rust"), line("  ````"), line("x")],
    };
    assert_eq!(
        model::markdown(&[
            Segment::Text {
                text: "see:\n".to_owned()
            },
            block
        ]),
        "see:\n\n`````md\n```rust\n  ````\nx\n`````"
    );
}

#[test]
fn english_is_the_last_resort() {
    assert_eq!(language_asked(None, Ok(settings(None))), translate("en"));
}

#[test]
fn the_device_language_is_lc_all_then_lc_messages_then_lang() {
    let fr = Some("fr_FR.UTF-8");
    let de = Some("de_DE.UTF-8");
    let es = Some("es_ES.UTF-8");
    assert_eq!(locale::device([fr, de, es]).as_deref(), Some("fr"));
    assert_eq!(locale::device([None, de, es]).as_deref(), Some("de"));
    assert_eq!(locale::device([None, None, es]).as_deref(), Some("es"));
    assert_eq!(locale::device([None, None, None]), None);
}

#[test]
fn a_locale_that_names_no_language_is_passed_over() {
    assert_eq!(
        locale::device([Some("C"), Some(""), Some("pt_BR.UTF-8")]).as_deref(),
        Some("pt")
    );
    assert_eq!(locale::device([Some("POSIX"), None, Some("C.UTF-8")]), None);
}

#[test]
fn segments_write_back_as_the_markdown_they_came_from() {
    for log in [
        ttyl::model::Log::Following,
        ttyl::model::Log::Latest,
        ttyl::model::Log::Trending,
    ] {
        for row in page(log).rows {
            assert_eq!(
                ttyl::model::markdown(&row.commit.body_segments),
                row.commit.body,
                "{}",
                row.commit.short_hash
            );
        }
    }
}
