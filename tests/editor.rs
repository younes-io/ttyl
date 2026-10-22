mod common;

use std::time::Instant;

use common::{boot, contains, press, screen};
use devnull::draft::{Target, Write};
use devnull::engine::{Effect, Engine, Event};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HEIGHT: u16 = 30;
const FAILED: &str = "editor exited with an error. draft unchanged.";

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

fn composing(text: &str, at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    press(&mut engine, text, at);
    engine
}

#[test]
fn ctrl_e_hands_the_draft_to_the_editor() {
    let at = Instant::now();
    let mut engine = composing("first", at);
    assert_eq!(
        ctrl(&mut engine, 'e', at),
        [Effect::Edit("first".to_owned())]
    );
}

#[test]
fn the_edited_text_comes_back_and_is_saved_without_the_final_newline() {
    let at = Instant::now();
    let mut engine = composing("first", at);
    ctrl(&mut engine, 'e', at);
    let effects = engine.update(Event::Edited("first\nsecond\n".to_owned()), at);
    assert_eq!(
        effects,
        [Effect::Draft(Write::Save(
            Target::New,
            "first\nsecond".to_owned()
        ))]
    );
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, " second"), "{rows:#?}");
    press(&mut engine, "!", at);
    assert_eq!(
        engine.draft(&Target::New),
        Some("first\nsecond!"),
        "the cursor lands at the end"
    );
}

#[test]
fn an_editor_that_saves_nothing_new_writes_nothing_and_an_emptied_file_drops_the_draft() {
    let at = Instant::now();
    let mut engine = composing("same", at);
    assert!(
        engine
            .update(Event::Edited("same\n".to_owned()), at)
            .is_empty()
    );
    assert_eq!(
        engine.update(Event::Edited("\n".to_owned()), at),
        [Effect::Draft(Write::Drop(Target::New))]
    );
}

#[test]
fn a_failed_editor_keeps_the_old_text_and_says_so() {
    let at = Instant::now();
    let mut engine = composing("keep me", at);
    ctrl(&mut engine, 'e', at);
    assert!(engine.update(Event::EditorFailed, at).is_empty());
    assert_eq!(engine.draft(&Target::New), Some("keep me"));
    for width in [100, 44] {
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(contains(&rows, " keep me"), "{rows:#?}");
        let failed = if width == 100 {
            FAILED
        } else {
            "editor failed. draft unchanged."
        };
        assert!(contains(&rows, failed), "{width}: {rows:#?}");
    }
    press(&mut engine, "!", at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(
        !contains(&rows, "draft unchanged"),
        "the note goes on the next edit"
    );
}

#[test]
fn ctrl_e_does_nothing_while_a_commit_is_sending() {
    let at = Instant::now();
    let mut engine = composing("hello", at);
    assert!(!ctrl(&mut engine, 's', at).is_empty());
    assert!(ctrl(&mut engine, 'e', at).is_empty());
}
