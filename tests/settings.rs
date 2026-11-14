mod common;

use std::time::Instant;

use common::{contains, marked, page, press, reload, runs, screen, sign_in, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Account, Call, Error, Reply};
use ttyl::config::{Audience, Change};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Device, DeviceId, Lane, PeoplePage, Settings, Zone};
use ttyl::sched::Ticket;

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
            "id": "0a000000-0000-4000-8000-00000000000a",
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
    listing(&[handle], None)
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
        [&Call::Revoke {
            id: DeviceId("d-phone".to_owned()),
            this: false,
        }]
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
        [&Call::Revoke {
            id: DeviceId("d-laptop".to_owned()),
            this: true,
        }]
    );
    reply(&mut engine, &effects, |_| Some(Ok(Reply::Done)), at);
    assert!(engine.log().is_none(), "signed out");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(contains(&rows, "You revoked this device"), "{rows:#?}");
}

#[test]
fn revoking_this_device_signs_out_even_once_settings_closed() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "G", at);
    press(&mut engine, "D", at);
    let effects = press(&mut engine, "y", at);
    tap(&mut engine, KeyCode::Esc, at);
    assert!(
        !contains(&screen(&mut engine, 120, 40, at), "devices · 2"),
        "settings closed"
    );

    reply(&mut engine, &effects, |_| Some(Ok(Reply::Done)), at);
    assert!(engine.log().is_none(), "signed out");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(contains(&rows, "You revoked this device"), "{rows:#?}");
}

fn listing(handles: &[&str], next: Option<&str>) -> PeoplePage {
    let people: Vec<serde_json::Value> = (handles.iter())
        .map(|handle| {
            serde_json::json!({
                "id": format!("u-{handle}"),
                "handle": handle,
                "name": format!("{handle} name"),
                "avatar_url": null,
                "html_url": format!("https://dev-null.sh/{handle}"),
                "lane": 0,
                "created_at": "2025-01-01T00:00:00Z",
            })
        })
        .collect();
    let json = serde_json::json!({ "data": people, "meta": { "next_cursor": next } });
    PeoplePage::parse(&json.to_string()).unwrap()
}

/// What dev-null.sh answers for the settings screen and the timeline behind it.
fn shown(call: &Call) -> Option<Reply> {
    Some(match call {
        Call::Me => Reply::Account(me(false, false)),
        Call::Settings => Reply::Settings(settings()),
        Call::Muted { cursor: None } => Reply::People(people("grace")),
        Call::Blocked { cursor: None } => Reply::People(people("bob")),
        Call::Devices => Reply::Devices(devices()),
        Call::Page(log) => Reply::Page(page(*log)),
        other => panic!("only what settings and the timeline show goes out: {other:?}"),
    })
}

/// Answers each call with `answer` until nothing more goes out, and hands back the calls it held
/// back by answering `None`.
fn drive(
    engine: &mut Engine,
    mut effects: Vec<Effect>,
    answer: impl Fn(&Call) -> Option<Reply>,
    at: Instant,
) -> Vec<(Ticket, Call)> {
    let mut held = Vec::new();
    while !effects.is_empty() {
        for effect in std::mem::take(&mut effects) {
            if let Effect::Run(ticket, call) = effect {
                match answer(&call) {
                    Some(reply) => {
                        effects.extend(engine.update(Event::Reply(ticket, Ok(reply)), at));
                        effects.extend(engine.pump(at));
                    }
                    None => held.push((ticket, call)),
                }
            }
        }
    }
    held
}

fn land(engine: &mut Engine, ticket: Ticket, reply: Reply, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, Ok(reply)), at);
    effects.extend(engine.pump(at));
    effects
}

/// Signs in as Ada and opens settings, answering everything but the second page of the people she
/// mutes, which comes back with its ticket.
fn open_paging(blocked: &[&str], at: Instant) -> (Engine, Ticket) {
    let engine = Engine::new("ttyl".to_owned(), at).with_fingerprint(THIS.to_owned());
    let mut engine = sign_in(engine, at, me(false, false), page);
    let effects = press(&mut engine, "g,", at);
    let held = drive(
        &mut engine,
        effects,
        |call| match call {
            Call::Muted { cursor: None } => Some(Reply::People(listing(&["grace"], Some("old-2")))),
            Call::Muted { .. } => None,
            Call::Blocked { cursor: None } => Some(Reply::People(listing(blocked, None))),
            other => shown(other),
        },
        at,
    );
    let [
        (
            ticket,
            Call::Muted {
                cursor: Some(cursor),
            },
        ),
    ] = held.as_slice()
    else {
        panic!("the second page waits: {held:?}");
    };
    assert_eq!(cursor, "old-2");
    (engine, *ticket)
}

#[test]
fn the_cursor_stays_on_its_person_as_a_page_lands_above() {
    let at = Instant::now();
    let (mut engine, second) = open_paging(&["bob", "carol"], at);
    press(&mut engine, "Gkk", at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "@carol").contains("│ @carol"), "{rows:#?}");

    land(
        &mut engine,
        second,
        Reply::People(listing(&["linus"], None)),
        at,
    );
    let effects = press(&mut engine, "B", at);
    assert_eq!(
        runs(&effects),
        [&Call::Block {
            handle: "carol".to_owned(),
            on: false,
        }]
    );
}

#[test]
fn a_page_from_before_a_reload_does_not_land_on_the_new_list() {
    let at = Instant::now();
    let (mut engine, old_second) = open_paging(&["bob"], at);
    let effects = reload(&mut engine, at);
    let held = drive(
        &mut engine,
        effects,
        |call| match call {
            Call::Muted { cursor: None } => Some(Reply::People(listing(&["grace"], Some("new-2")))),
            Call::Muted { .. } => None,
            other => shown(other),
        },
        at,
    );
    assert!(held.is_empty(), "{held:?}");

    let effects = land(
        &mut engine,
        old_second,
        Reply::People(listing(&["linus"], Some("old-3"))),
        at,
    );
    assert_eq!(
        runs(&effects),
        [&Call::Muted {
            cursor: Some("new-2".to_owned()),
        }],
        "the new list's second page goes once the old one is out of the way"
    );
    let held = drive(
        &mut engine,
        effects,
        |call| match call {
            Call::Muted { .. } => Some(Reply::People(listing(&["ken"], None))),
            other => shown(other),
        },
        at,
    );
    assert!(held.is_empty(), "{held:?}");
    let rows = screen(&mut engine, 120, 40, at);
    assert!(contains(&rows, "muted · 2"), "{rows:#?}");
    assert!(contains(&rows, "@ken"), "{rows:#?}");
    assert!(!contains(&rows, "@linus"), "{rows:#?}");
}

#[test]
fn an_older_me_does_not_undo_a_switch_just_saved() {
    let at = Instant::now();
    let mut engine = open(false, at);
    let mut effects = reload(&mut engine, at);
    effects.extend(tap(&mut engine, KeyCode::Enter, at));
    let held = drive(
        &mut engine,
        effects,
        |call| match call {
            Call::Me => None,
            Call::Configure(Change::Pasteis(true)) => Some(Reply::Account(me(true, false))),
            other => shown(other),
        },
        at,
    );
    let [(older, Call::Me)] = held.as_slice() else {
        panic!("me is still out: {held:?}");
    };

    land(&mut engine, *older, Reply::Account(me(false, false)), at);
    let rows = screen(&mut engine, 120, 40, at);
    let pasteis = row(&rows, "pasteis");
    assert!(
        pasteis.contains("true") && !pasteis.contains("saving"),
        "{pasteis}"
    );

    // A `me` asked for after the change says what dev-null.sh holds now.
    let effects = reload(&mut engine, at);
    drive(&mut engine, effects, shown, at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "pasteis").contains("false"), "{rows:#?}");
}

#[test]
fn older_settings_do_not_undo_a_timezone_just_saved() {
    let at = Instant::now();
    let mut engine = open(false, at);
    let mut effects = reload(&mut engine, at);
    press(&mut engine, "jj", at);
    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    press(&mut engine, "tokyo", at);
    effects.extend(tap(&mut engine, KeyCode::Enter, at));
    let held = drive(
        &mut engine,
        effects,
        |call| match call {
            Call::Settings => None,
            Call::Configure(Change::Timezone(_)) => Some(Reply::Account(me(false, false))),
            other => shown(other),
        },
        at,
    );
    let [(older, Call::Settings)] = held.as_slice() else {
        panic!("settings are still out: {held:?}");
    };

    land(&mut engine, *older, Reply::Settings(settings()), at);
    let rows = screen(&mut engine, 120, 40, at);
    assert!(row(&rows, "timezone").contains("Asia/Tokyo"), "{rows:#?}");
}

#[test]
fn a_paste_after_tab_saves_the_match_the_menu_shows() {
    let at = Instant::now();
    let mut engine = open(false, at);
    press(&mut engine, "jj", at);
    tap(&mut engine, KeyCode::Enter, at);
    ctrl(&mut engine, 'u', at);
    tap(&mut engine, KeyCode::Tab, at);
    tap(&mut engine, KeyCode::Tab, at);
    engine.update(Event::Paste("tokyo".to_owned()), at);
    assert_eq!(
        marked(&mut engine, 120, 40, at).as_deref(),
        Some("Asia/Tokyo")
    );

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        runs(&effects),
        [&configure(Change::Timezone(Some(Zone {
            name: "Asia/Tokyo".to_owned(),
            local_time: false,
        })))]
    );
}
