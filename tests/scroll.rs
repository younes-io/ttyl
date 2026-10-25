mod common;

use std::collections::HashSet;
use std::time::Instant;

use common::{boot_serving, log_of};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitPage, Log, LogPage, RepliesPage};

const ID: &str = "7a11c0de-0000-4000-8000-000000000040";

fn card() -> serde_json::Value {
    let body = (1..=40)
        .map(|n| format!("line {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::json!({
        "id": ID,
        "body": body,
        "body_segments": [{ "type": "text", "text": body }],
        "replies_count": 0,
    })
}

fn tall(log: Log) -> LogPage {
    LogPage {
        log,
        ..log_of(&[card()])
    }
}

fn serve(engine: &mut Engine, mut effects: Vec<Effect>, at: Instant) {
    loop {
        effects.extend(engine.pump(at));
        if effects.is_empty() {
            return;
        }
        for effect in std::mem::take(&mut effects) {
            let Effect::Run(ticket, call) = effect else {
                continue;
            };
            let reply = match call {
                Call::Page(log) => Reply::Page(tall(log)),
                Call::Commit(_) => {
                    let json = serde_json::json!({ "data": card_row(), "ancestors": [] });
                    Reply::Thread(Box::new(CommitPage::parse(&json.to_string()).unwrap()))
                }
                Call::Replies { .. } => Reply::Replies(
                    RepliesPage::parse(r#"{"data":[],"meta":{"next_cursor":null}}"#).unwrap(),
                ),
                other => panic!("unexpected call: {other:?}"),
            };
            effects.extend(engine.update(Event::Reply(ticket, Ok(reply)), at));
        }
    }
}

fn card_row() -> serde_json::Value {
    let latest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("fixtures/log-latest.json").unwrap())
            .unwrap();
    let mut row = latest["data"][0].clone();
    for (field, value) in card().as_object().unwrap() {
        row[field] = value.clone();
    }
    row
}

fn missing(seen: &HashSet<String>) -> Vec<String> {
    (1..=40)
        .map(|n| format!("line {n:02}"))
        .filter(|line| !seen.iter().any(|row| row.contains(line.as_str())))
        .collect()
}

/// Expands the selected commit, then reads down it one line at a time, collecting every row shown.
fn read_down(engine: &mut Engine, at: Instant) -> HashSet<String> {
    let mut seen: HashSet<String> = common::screen(engine, 80, 15, at).into_iter().collect();
    for keys in std::iter::once(" ").chain(std::iter::repeat_n("j", 40)) {
        let effects = common::press(engine, keys, at);
        serve(engine, effects, at);
        seen.extend(common::screen(engine, 80, 15, at));
    }
    seen
}

#[test]
fn every_line_of_a_tall_commit_in_the_feed_can_be_reached() {
    let at = Instant::now();
    let mut engine = boot_serving(at, tall);
    let seen = read_down(&mut engine, at);
    assert_eq!(
        missing(&seen),
        Vec::<String>::new(),
        "never on screen in the feed"
    );
    assert!(
        seen.iter().any(|row| row.contains("more lines")),
        "hint says more is below"
    );
}

#[test]
fn every_line_of_a_tall_commit_in_its_thread_can_be_reached() {
    let at = Instant::now();
    let mut engine = boot_serving(at, tall);
    common::screen(&mut engine, 80, 15, at);
    let effects = common::tap(&mut engine, KeyCode::Enter, at);
    serve(&mut engine, effects, at);
    let seen = read_down(&mut engine, at);
    assert_eq!(
        missing(&seen),
        Vec::<String>::new(),
        "never on screen in the thread"
    );
    assert!(
        seen.iter().any(|row| row.contains("more lines")),
        "hint says more is below"
    );
}
