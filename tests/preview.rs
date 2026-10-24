mod common;

use std::time::Instant;

use common::{boot, buffer, contains, press, runs, screen, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::{Call, Error, Reply};
use ttyl::code::Hl;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::Segment;
use ttyl::sched::Ticket;
use ttyl::theme;

const WIDTHS: [u16; 2] = [100, 44];
const HEIGHT: u16 = 30;

fn writing(at: Instant, text: &str) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    press(&mut engine, &format!("c{text}"), at);
    engine
}

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

fn previews(effects: &[Effect]) -> Vec<(Ticket, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Run(ticket, Call::Preview(text)) => Some((*ticket, text.clone())),
            _ => None,
        })
        .collect()
}

fn one_preview(effects: &[Effect]) -> Ticket {
    let [(ticket, _)] = previews(effects)[..] else {
        panic!("one preview goes out: {:#?}", runs(effects));
    };
    ticket
}

fn rust_block() -> Vec<Segment> {
    let json = serde_json::json!([
        { "type": "text", "text": "look at this\n" },
        { "type": "block", "language": "rust", "html": "fn main() {}" },
    ]);
    serde_json::from_value(json).expect("segments")
}

fn land(engine: &mut Engine, ticket: Ticket, result: Result<Reply, Error>, at: Instant) {
    engine.update(Event::Reply(ticket, result), at);
    engine.pump(at);
}

#[test]
fn ctrl_r_asks_the_server_to_render_the_draft() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let effects = ctrl(&mut engine, 'r', at);
    assert_eq!(
        previews(&effects)
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>(),
        vec!["hello".to_owned()]
    );
}

#[test]
fn the_preview_says_it_is_rendering_until_the_reply() {
    let at = Instant::now();
    for width in WIDTHS {
        let mut engine = writing(at, "hello");
        ctrl(&mut engine, 'r', at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(contains(&rows, "rendering…"), "{rows:#?}");
        assert!(contains(&rows, "hello"), "{rows:#?}");
    }
}

#[test]
fn the_rendered_draft_shows_with_highlighted_code() {
    let at = Instant::now();
    for width in WIDTHS {
        let mut engine = writing(at, "look at this");
        let ticket = one_preview(&ctrl(&mut engine, 'r', at));
        land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(!contains(&rows, "rendering…"), "{rows:#?}");
        let y = rows
            .iter()
            .position(|row| row.contains("fn main() {}"))
            .unwrap_or_else(|| panic!("the block is drawn: {rows:#?}"));
        let x = rows[y].find("fn main").unwrap() as u16;
        let cells = buffer(&mut engine, width, HEIGHT, at);
        assert_eq!(
            cells[(x, y as u16)].fg,
            theme::hl(Some(Hl::Keyword)).fg.unwrap(),
            "fn is colored as a keyword"
        );
    }
}

#[test]
fn going_back_and_forth_without_an_edit_asks_once() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let ticket = one_preview(&ctrl(&mut engine, 'r', at));
    land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
    assert!(previews(&ctrl(&mut engine, 'r', at)).is_empty());
    assert!(previews(&ctrl(&mut engine, 'r', at)).is_empty());
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "fn main() {}"), "{rows:#?}");
}

#[test]
fn an_edit_asks_again() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let ticket = one_preview(&ctrl(&mut engine, 'r', at));
    land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
    ctrl(&mut engine, 'r', at);
    press(&mut engine, "!", at);
    let effects = ctrl(&mut engine, 'r', at);
    assert_eq!(previews(&effects)[0].1, "hello!");
}

#[test]
fn esc_goes_back_to_writing() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let ticket = one_preview(&ctrl(&mut engine, 'r', at));
    land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
    tap(&mut engine, KeyCode::Esc, at);
    press(&mut engine, "!", at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "hello!"), "{rows:#?}");
    assert!(!contains(&rows, "fn main() {}"), "{rows:#?}");
}

#[test]
fn typing_in_the_preview_changes_nothing() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let ticket = one_preview(&ctrl(&mut engine, 'r', at));
    press(&mut engine, "xyz", at);
    land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "hello"), "{rows:#?}");
    assert!(!contains(&rows, "xyz"), "{rows:#?}");
}

#[test]
fn a_failed_render_shows_the_draft_as_typed() {
    let at = Instant::now();
    for width in WIDTHS {
        let mut engine = writing(at, "hello");
        let ticket = one_preview(&ctrl(&mut engine, 'r', at));
        land(&mut engine, ticket, Err(Error::Unexpected(500)), at);
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(contains(&rows, "preview unavailable"), "{rows:#?}");
        assert!(contains(&rows, "hello"), "{rows:#?}");
    }
}

#[test]
fn a_reply_for_an_older_draft_is_ignored() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let stale = one_preview(&ctrl(&mut engine, 'r', at));
    tap(&mut engine, KeyCode::Esc, at);
    press(&mut engine, "!", at);
    let fresh = one_preview(&ctrl(&mut engine, 'r', at));
    land(&mut engine, stale, Ok(Reply::Preview(rust_block())), at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "rendering…"), "{rows:#?}");
    assert!(!contains(&rows, "fn main() {}"), "{rows:#?}");
    let text = vec![Segment::Text {
        text: "hello!".to_owned(),
    }];
    land(&mut engine, fresh, Ok(Reply::Preview(text)), at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "hello!"), "{rows:#?}");
    assert!(!contains(&rows, "rendering…"), "{rows:#?}");
}

#[test]
fn ctrl_s_commits_from_the_preview() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    ctrl(&mut engine, 'r', at);
    let effects = ctrl(&mut engine, 's', at);
    assert!(
        runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Post { .. })),
        "{:#?}",
        runs(&effects)
    );
}

#[test]
fn the_hints_name_the_preview_keys() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "ctrl-r preview"), "{rows:#?}");
    ctrl(&mut engine, 'r', at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "esc write"), "{rows:#?}");
    assert!(contains(&rows, "ctrl-s commit"), "{rows:#?}");
}

#[test]
fn a_refused_commit_from_the_preview_says_why() {
    let at = Instant::now();
    let mut engine = writing(at, "hello");
    let ticket = one_preview(&ctrl(&mut engine, 'r', at));
    land(&mut engine, ticket, Ok(Reply::Preview(rust_block())), at);
    let effects = ctrl(&mut engine, 's', at);
    let post = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(ticket, Call::Post { .. }) => Some(*ticket),
            _ => None,
        })
        .expect("ctrl-s sends the draft");
    let refusal = Error::Api {
        status: 422,
        code: None,
        message: "The body field is required.".to_owned(),
        retry_after: None,
    };
    land(&mut engine, post, Err(refusal), at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "The body field is required."), "{rows:#?}");
}
