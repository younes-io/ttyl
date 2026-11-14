mod common;

use std::time::Instant;

use common::{boot, commit, press, screen};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::CommitId;

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";

fn on_grace(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    engine
}

fn hit(engine: &mut Engine, code: KeyCode, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), at);
    effects.extend(engine.pump(at));
    effects
}

fn body() -> String {
    commit(&CommitId(GRACE.to_owned()))
        .unwrap()
        .commit
        .commit
        .body
}

#[test]
fn a_commit_pipes_its_text_to_the_typed_command() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    assert!(
        press(&mut engine, "|wc -l", at).is_empty(),
        "nothing runs while typing"
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert_eq!(rows[29].trim(), "| wc -l█", "{rows:#?}");
    assert_eq!(
        hit(&mut engine, KeyCode::Enter, at),
        vec![Effect::Pipe {
            command: "wc -l".to_owned(),
            input: body(),
        }]
    );
}

#[test]
fn the_next_pipe_starts_from_the_last_command() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "|llm", at);
    hit(&mut engine, KeyCode::Enter, at);
    engine.update(Event::Piped("llm".to_owned(), Ok(())), at);
    press(&mut engine, "|", at);
    let effects = hit(&mut engine, KeyCode::Enter, at);
    assert!(
        matches!(&effects[..], [Effect::Pipe { command, .. }] if command == "llm"),
        "{effects:?}"
    );
}

#[test]
fn esc_or_an_empty_command_runs_nothing() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    press(&mut engine, "|cat", at);
    assert!(hit(&mut engine, KeyCode::Esc, at).is_empty());
    press(&mut engine, "|  ", at);
    assert!(hit(&mut engine, KeyCode::Enter, at).is_empty());
    assert!(
        press(&mut engine, "j", at).is_empty(),
        "keys move the log again once the prompt is gone"
    );
}

#[test]
fn a_failed_pipe_says_why() {
    let at = Instant::now();
    let mut engine = on_grace(at);
    engine.update(
        Event::Piped("false".to_owned(), Err("exited with 1".to_owned())),
        at,
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[29].contains("fatal: false: exited with 1"),
        "{rows:#?}"
    );
}
