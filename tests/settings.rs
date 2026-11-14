mod common;

use std::time::Instant;

use common::{contains, page, press, runs, screen, sign_in, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Account, Call, Error, Reply};
use ttyl::config::{Audience, Change};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Device, DeviceId, Lane, PeoplePage, Settings, Zone};

const THIS: &str = "SHA256:thisterminal";

fn me(pasteis: bool, remotes: bool) -> Account {
    let remotes = remotes.then(|| {
        serde_json::json!({
            "from": "following",
            "pull_receipts": true,
            "requests": 0,
            "everyone": false,
            "unread_count": 0,
        })
    });
    serde_json::from_value(serde_json::json!({
        "data": {
            "handle": "ada",
            "name": "Ada",
            "pasteis_mode": pasteis,
            "preferred_lane": null,
            "automatic_lane": 3,
            "remotes": remotes,
        },
        "unread_notifications_count": 0,
    }))
    .unwrap()
}

fn settings() -> Settings {
    let json = serde_json::json!({ "data": {
        "timezone": "Europe/Lisbon",
        "show_local_time": false,
        "translation_language": "fr",
        "automatic_translation": true,
        "translation_languages": [
            { "code": "fr", "name": "French", "native_name": "français" },
            { "code": "de", "name": "German", "native_name": "Deutsch" },
            { "code": "pt", "name": "Portuguese", "native_name": "português" },
        ],
        "is_admin": false,
        "push": {},
    }});
    Settings::parse(&json.to_string()).unwrap()
}

fn people(handle: &str) -> PeoplePage {
    let json = serde_json::json!({
        "data": [{
            "id": format!("u-{handle}"),
            "handle": handle,
            "name": format!("{handle} name"),
            "avatar_url": null,
            "html_url": format!("https://dev-null.sh/{handle}"),
            "lane": 0,
            "created_at": "2025-01-01T00:00:00Z",
        }],
        "meta": { "next_cursor": null },
    });
    PeoplePage::parse(&json.to_string()).unwrap()
}

fn devices() -> Vec<Device> {
    let json = serde_json::json!({ "data": [
        {
            "id": "d-phone",
            "name": "phone",
            "app_name": "dev-null",
            "fingerprint": "SHA256:phone",
            "approved_at": "2026-10-20T10:00:00Z",
            "last_used_at": "2026-10-21T08:00:00Z",
        },
        {
            "id": "d-laptop",
            "name": "laptop",
            "app_name": "ttyl",
            "fingerprint": THIS,
            "approved_at": "2026-10-01T10:00:00Z",
            "last_used_at": null,
        },
    ]});
    Device::parse_list(&json.to_string()).unwrap()
}

fn reply(
    engine: &mut Engine,
    effects: &[Effect],
    answer: impl Fn(&Call) -> Option<Result<Reply, Error>>,
    at: Instant,
) -> Vec<Effect> {
    let mut next = Vec::new();
    for effect in effects {
        if let Effect::Run(ticket, call) = effect
            && let Some(result) = answer(call)
        {
            next.extend(engine.update(Event::Reply(*ticket, result), at));
            next.extend(engine.pump(at));
        }
    }
    next
}

/// Signs in as Ada and opens settings with `g,`, answering everything it asks for.
fn open(remotes: bool, at: Instant) -> Engine {
    let engine = Engine::new("ttyl".to_owned(), at).with_fingerprint(THIS.to_owned());
    let mut engine = sign_in(engine, at, me(false, remotes), page);
    let mut effects = press(&mut engine, "g,", at);
    let mut asked = Vec::new();
    while !effects.is_empty() {
        asked.extend(runs(&effects).into_iter().cloned());
        effects = reply(
            &mut engine,
            &effects,
            |call| {
                Some(Ok(match call {
                    Call::Me => Reply::Account(me(false, remotes)),
                    Call::Settings => Reply::Settings(settings()),
                    Call::Muted { .. } => Reply::People(people("grace")),
                    Call::Blocked { .. } => Reply::People(people("bob")),
                    Call::Devices => Reply::Devices(devices()),
                    other => panic!("only what settings shows goes out: {other:?}"),
                }))
            },
            at,
        );
    }
    asked.sort_by_key(|call| format!("{call:?}"));
    assert_eq!(
        asked,
        [
            Call::Blocked { cursor: None },
            Call::Devices,
            Call::Me,
            Call::Muted { cursor: None },
            Call::Settings,
        ]
    );
    engine
}

fn ctrl(engine: &mut Engine, ch: char, at: Instant) {
    engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
}

fn row<'a>(rows: &'a [String], needle: &str) -> &'a str {
    rows.iter()
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle}: {rows:#?}"))
}

fn configure(change: Change) -> Call {
    Call::Configure(change)
}

#[test]
fn g_comma_lists_the_settings_the_people_and_the_devices_under_headings() {
    let at = Instant::now();
    let mut engine = open(true, at);
    let rows = screen(&mut engine, 120, 40, at);
    for heading in [
        "account",
        "region",
        "remotes",
        "muted · 1",
        "blocked · 1",
        "devices · 2",
    ] {
        assert!(
            rows.iter()
                .any(|row| row.trim_matches(|ch| ch == ' ' || ch == '│') == heading),
            "{heading}: {rows:#?}"
        );
    }
    assert!(row(&rows, "pasteis").contains("false"), "{rows:#?}");
    assert!(row(&rows, "color").contains("auto · teal"), "{rows:#?}");
    assert!(
        row(&rows, "timezone").contains("Europe/Lisbon"),
        "{rows:#?}"
    );
    assert!(row(&rows, "language").contains("fr · French"), "{rows:#?}");
    assert!(row(&rows, "from").contains("following"), "{rows:#?}");
    assert!(row(&rows, "@grace").contains("grace name"), "{rows:#?}");
    assert!(contains(&rows, "@bob"), "{rows:#?}");
    let laptop = row(&rows, "laptop");
    assert!(laptop.contains("this device"), "{laptop}");
    assert!(laptop.contains(THIS), "{laptop}");
    assert!(laptop.contains("never used"), "{laptop}");
    assert!(row(&rows, "phone").contains("used 2h"), "{rows:#?}");
    assert!(!row(&rows, "phone").contains("this device"), "{rows:#?}");
}

#[test]
fn enter_flips_a_switch_in_place_until_dev_null_sh_confirms_it() {
    let at = Instant::now();
    let mut engine = open(false, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&configure(Change::Pasteis(true))]);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "pasteis").contains("true · saving"), "{rows:#?}");

    let rest = reply(
        &mut engine,
        &effects,
        |_| Some(Ok(Reply::Account(me(true, false)))),
        at,
    );
    assert!(runs(&rest).is_empty(), "{rest:?}");
    let rows = screen(&mut engine, 120, 40, at);
    let pasteis = row(&rows, "pasteis");
    assert!(
        pasteis.contains("true") && !pasteis.contains("saving"),
        "{pasteis}"
    );
}

#[test]
fn a_refused_change_goes_back_and_says_why() {
    let at = Instant::now();
    let mut engine = open(false, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let refused = || Error::Api {
        status: 422,
        code: None,
        message: "pastéis are resting".to_owned(),
        retry_after: None,
    };
    reply(&mut engine, &effects, |_| Some(Err(refused())), at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "pasteis").contains("false"), "{rows:#?}");
    assert!(contains(&rows, "pastéis are resting"), "{rows:#?}");
}

#[test]
fn enter_moves_a_choice_to_its_next_value() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&configure(Change::Color(Some(Lane::Palette(0))))]
    );
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "color").contains("orange"), "{rows:#?}");
}

#[test]
fn enter_on_the_timezone_types_it_on_the_bottom_row_and_completes_iana_names() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jj", at);
    tap(&mut engine, KeyCode::Enter, at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        rows[39].starts_with(" region.timezone: Europe/Lisbon█"),
        "{}",
        rows[39]
    );

    ctrl(&mut engine, 'u', at);
    assert!(
        runs(&press(&mut engine, "tokyo", at)).is_empty(),
        "nothing goes while typing"
    );
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        rows[39].starts_with(" region.timezone: tokyo█"),
        "{}",
        rows[39]
    );
    assert!(rows[39].contains("Asia/Tokyo"), "{}", rows[39]);

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&configure(Change::Timezone(Some(Zone {
            name: "Asia/Tokyo".to_owned(),
            local_time: false,
        })))]
    );
}

#[test]
fn an_empty_timezone_unsets_it_and_esc_keeps_it() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jj", at);
    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        row(&rows, "timezone").contains("Europe/Lisbon"),
        "{rows:#?}"
    );

    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(rows[39].contains("enter unsets it"), "{}", rows[39]);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&configure(Change::Timezone(None))]);
}

#[test]
fn the_translation_language_list_filters_as_you_type_and_tab_moves_through_it() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jjjj", at);
    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(rows[39].contains("tab lists them"), "{}", rows[39]);

    press(&mut engine, "deu", at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(rows[39].contains("de  German · Deutsch"), "{}", rows[39]);
    assert!(!rows[39].contains("French"), "filtered: {}", rows[39]);

    ctrl(&mut engine, 'u', at);
    press(&mut engine, "p", at);
    tap(&mut engine, KeyCode::Tab, at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&configure(Change::Language(Some("pt".to_owned())))]
    );
}

#[test]
fn a_language_dev_null_sh_does_not_translate_into_is_refused_before_it_goes() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jjjj", at);
    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    press(&mut engine, "klingon", at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        rows[39].contains("doesn't translate into that"),
        "{}",
        rows[39]
    );
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(runs(&effects).is_empty(), "{effects:?}");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        contains(&rows, "fatal: dev-null.sh doesn't translate into 'klingon'"),
        "{rows:#?}"
    );
}

#[test]
fn turning_remotes_on_shows_who_can_push_once_dev_null_sh_says_so() {
    let at = Instant::now();
    let mut engine = open(false, at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(!contains(&rows, "pull-receipts"), "{rows:#?}");
    press(&mut engine, "jjjjjj", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&configure(Change::Remotes(true))]);
    reply(
        &mut engine,
        &effects,
        |_| Some(Ok(Reply::Account(me(false, true)))),
        at,
    );
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "pull-receipts").contains("true"), "{rows:#?}");

    // dev-null.sh only offers everyone where its config allows it.
    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(runs(&effects).is_empty(), "{effects:?}");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(
        contains(&rows, "only lets people you follow push"),
        "{rows:#?}"
    );

    press(&mut engine, "j", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&configure(Change::Prefs {
            from: Audience::Following,
            pull_receipts: false,
        })]
    );
}

#[test]
fn m_unmutes_from_the_list_and_m_again_mutes() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jjjjjjj", at);
    let effects = press(&mut engine, "m", at);
    assert_eq!(
        runs(&effects),
        [&Call::Mute {
            handle: "grace".to_owned(),
            on: false,
        }]
    );
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "@grace").contains("unmuted"), "{rows:#?}");
    assert!(contains(&rows, "unmuted @grace"), "{rows:#?}");
    reply(&mut engine, &effects, |_| Some(Ok(Reply::Done)), at);

    let effects = press(&mut engine, "m", at);
    assert_eq!(
        runs(&effects),
        [&Call::Mute {
            handle: "grace".to_owned(),
            on: true,
        }]
    );
}

#[test]
fn shift_b_unblocks_from_the_list_without_asking() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jjjjjjjj", at);
    let effects = press(&mut engine, "B", at);
    assert_eq!(
        runs(&effects),
        [&Call::Block {
            handle: "bob".to_owned(),
            on: false,
        }]
    );
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "@bob").contains("unblocked"), "{rows:#?}");
    assert!(
        contains(
            &rows,
            "unblocked @bob · the follows it ended don't come back"
        ),
        "{rows:#?}"
    );
}

#[test]
fn shift_d_asks_before_revoking_another_device() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "G", at);
    press(&mut engine, "k", at);
    assert!(runs(&press(&mut engine, "D", at)).is_empty(), "asks first");
    let rows = screen(&mut engine, 120, 40, at);
    assert_eq!(
        rows[39].trim_end(),
        " revoke phone? it can't sign in until approved again [y/N]"
    );
    assert!(runs(&press(&mut engine, "n", at)).is_empty());
    assert!(contains(&screen(&mut engine, 120, 40, at), "phone"));

    press(&mut engine, "D", at);
    let effects = press(&mut engine, "y", at);
    assert_eq!(
        runs(&effects),
        [&Call::Revoke(DeviceId("d-phone".to_owned()))]
    );
    reply(&mut engine, &effects, |_| Some(Ok(Reply::Done)), at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(contains(&rows, "devices · 1"), "{rows:#?}");
    assert!(contains(&rows, "revoked phone"), "{rows:#?}");
}

#[test]
fn revoking_this_device_warns_that_it_signs_ttyl_out_and_does() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "G", at);
    press(&mut engine, "D", at);
    let rows = screen(&mut engine, 120, 40, at);
    assert_eq!(
        rows[39].trim_end(),
        " revoke this device? ttyl signs out and asks to sign in again [y/N]"
    );
    let effects = press(&mut engine, "y", at);
    assert_eq!(
        runs(&effects),
        [&Call::Revoke(DeviceId("d-laptop".to_owned()))]
    );
    reply(&mut engine, &effects, |_| Some(Ok(Reply::Done)), at);
    assert!(engine.log().is_none(), "signed out");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(contains(&rows, "You revoked this device"), "{rows:#?}");
}
