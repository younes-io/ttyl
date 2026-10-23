mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{account, boot, contains, page, press, scratch, screen, sign_in, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::attach::{self, Attachment, Image, Look};
use ttyl::draft::{Draft, Drafts, Target, Write};
use ttyl::engine::{Effect, Engine, Event};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x07\x80\0\0\x04\x38\x08\x06";

fn png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, PNG).unwrap();
    path
}

/// Pastes `text` the way a terminal pastes a dropped file, reading what it names as the app does.
fn drop_in(engine: &mut Engine, text: &str, at: Instant) -> Vec<Effect> {
    let effects = engine.update(Event::Paste(text.to_owned()), at);
    read(engine, effects, at)
}

fn read(engine: &mut Engine, effects: Vec<Effect>, at: Instant) -> Vec<Effect> {
    let mut out = Vec::new();
    for effect in effects {
        match effect {
            Effect::Read(paths, pasted) => {
                let looks = (paths.into_iter())
                    .map(|path| {
                        let look = attach::look(&path);
                        (path, look)
                    })
                    .collect();
                out.extend(engine.update(Event::Read(looks, pasted), at));
            }
            other => out.push(other),
        }
    }
    out.extend(engine.pump(at));
    out
}

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    read(engine, effects, at)
}

fn panel(engine: &mut Engine, at: Instant) -> Vec<String> {
    let rows = screen(engine, 100, 30, at);
    let title = rows
        .iter()
        .position(|row| row.starts_with(" commit ─"))
        .unwrap_or_else(|| panic!("the composer is open: {rows:#?}"));
    rows[title..].to_vec()
}

fn composing(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    engine
}

#[test]
fn a_dropped_file_is_attached_with_its_size() {
    let dir = scratch("attach-dropped");
    let shot = png(&dir, "shot.png");
    let at = Instant::now();
    let mut engine = composing(at);
    press(&mut engine, "look", at);
    drop_in(&mut engine, &format!("'{}' ", shot.display()), at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "look"), "{rows:#?}");
    assert!(contains(&rows, "shot.png · 1920×1080 · 26 B"), "{rows:#?}");
    assert!(
        contains(&rows, "attached shot.png · tab to describe it"),
        "{rows:#?}"
    );
    assert!(!contains(&rows, &dir.display().to_string()), "{rows:#?}");
}

#[test]
fn a_pasted_path_to_nothing_is_typed_as_text() {
    let at = Instant::now();
    let mut engine = composing(at);
    drop_in(&mut engine, "/no/such/file.png", at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "/no/such/file.png"), "{rows:#?}");
    assert!(!contains(&rows, "attached"), "{rows:#?}");
}

#[test]
fn a_file_the_server_would_refuse_says_why_and_holds_the_commit() {
    let dir = scratch("attach-refused");
    let notes = dir.join("notes.txt");
    fs::write(&notes, "not an image").unwrap();
    let at = Instant::now();
    let mut engine = composing(at);
    press(&mut engine, "see", at);
    drop_in(&mut engine, &notes.display().to_string(), at);
    let rows = panel(&mut engine, at);
    assert!(
        contains(&rows, "notes.txt not a jpeg, png or webp."),
        "{rows:#?}"
    );
    let effects = ctrl(&mut engine, 's', at);
    assert!(common::runs(&effects).is_empty(), "{effects:?}");
}

#[test]
fn a_commit_carries_three_images_at_most() {
    let dir = scratch("attach-three");
    let at = Instant::now();
    let mut engine = composing(at);
    let paths: Vec<String> = (1..=4)
        .map(|n| png(&dir, &format!("{n}.png")).display().to_string())
        .collect();
    drop_in(&mut engine, &paths[..3].join(" "), at);
    assert!(contains(
        &panel(&mut engine, at),
        "attached 3 images · tab to describe them"
    ));
    drop_in(&mut engine, &paths[3], at);
    let rows = panel(&mut engine, at);
    assert!(
        contains(&rows, "a commit carries up to 3 images."),
        "{rows:#?}"
    );
    assert!(!contains(&rows, "4.png"), "{rows:#?}");
}

#[test]
fn a_commit_carries_a_poll_or_images_not_both() {
    let dir = scratch("attach-either");
    let shot = png(&dir, "shot.png");
    let at = Instant::now();
    let mut engine = composing(at);
    drop_in(&mut engine, &shot.display().to_string(), at);
    ctrl(&mut engine, 'p', at);
    let rows = panel(&mut engine, at);
    assert!(
        contains(&rows, "a commit carries a poll or images, not both."),
        "{rows:#?}"
    );
    assert!(!contains(&rows, "option 1"), "{rows:#?}");
}

#[test]
fn tab_describes_an_image_and_backspace_on_an_empty_one_removes_it() {
    let dir = scratch("attach-describe");
    let shot = png(&dir, "shot.png");
    let at = Instant::now();
    let mut engine = composing(at);
    drop_in(&mut engine, &shot.display().to_string(), at);
    assert!(contains(
        &panel(&mut engine, at),
        "describe it for people who can't see it"
    ));
    tap(&mut engine, KeyCode::Tab, at);
    let effects = press(&mut engine, "a cat", at);
    let draft = Draft {
        text: String::new(),
        attachment: Attachment::Images(vec![Image {
            path: shot.clone(),
            description: "a cat".to_owned(),
        }]),
    };
    assert!(
        effects.contains(&Effect::Draft(Write::Save(Target::New, draft))),
        "{effects:?}"
    );
    for _ in 0.."a cat".len() {
        tap(&mut engine, KeyCode::Backspace, at);
    }
    tap(&mut engine, KeyCode::Backspace, at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "removed shot.png"), "{rows:#?}");
    assert!(!contains(&rows, "1920×1080"), "{rows:#?}");
}

#[test]
fn a_saved_draft_reads_its_images_again_when_opened() {
    let dir = scratch("attach-restored");
    let shot = png(&dir, "shot.png");
    let gone = dir.join("gone.png");
    let draft = Draft {
        text: "two".to_owned(),
        attachment: Attachment::Images(vec![
            Image {
                path: shot.clone(),
                description: "a cat".to_owned(),
            },
            Image {
                path: gone.clone(),
                description: String::new(),
            },
        ]),
    };
    let at = Instant::now();
    let engine = Engine::new("ttyl".to_owned(), at)
        .with_drafts(Drafts::new(HashMap::from([(Target::New, draft)])));
    let mut engine = sign_in(engine, at, account(3), page);
    let effects = engine.update(Event::Key(common::key('c')), at);
    assert_eq!(effects, [Effect::Read(vec![shot, gone], None)]);
    read(&mut engine, effects, at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "a cat"), "{rows:#?}");
    assert!(contains(&rows, "gone.png is gone from disk."), "{rows:#?}");
}

#[test]
fn a_look_tells_a_sendable_file_from_one_that_is_not() {
    let dir = scratch("attach-look");
    let Look::Fine { probe, bytes } = attach::look(&png(&dir, "shot.png")) else {
        panic!("a png is fine");
    };
    assert_eq!((probe.width, probe.height, bytes), (1920, 1080, 26));
    assert_eq!(attach::look(&dir), Look::Gone);
    assert_eq!(attach::look(&dir.join("missing.png")), Look::Gone);
}

#[test]
fn dropped_text_is_read_the_ways_terminals_write_paths() {
    let paths = |text: &str| attach::dropped(text);
    let one = |path: &str| Some(vec![PathBuf::from(path)]);
    assert_eq!(paths("/home/a/shot.png\n"), one("/home/a/shot.png"));
    assert_eq!(paths("'/home/a/my shot.png'"), one("/home/a/my shot.png"));
    assert_eq!(paths("\"/home/a/my shot.png\""), one("/home/a/my shot.png"));
    assert_eq!(paths("/home/a/my\\ shot.png"), one("/home/a/my shot.png"));
    assert_eq!(
        paths("file:///home/a/my%20shot.png"),
        one("/home/a/my shot.png")
    );
    assert_eq!(paths("file://localhost/home/a/b.png"), one("/home/a/b.png"));
    assert_eq!(
        paths("/a.png /b.png\n/c.png"),
        Some(vec!["/a.png".into(), "/b.png".into(), "/c.png".into()])
    );
    assert_eq!(paths("shot.png"), None);
    assert_eq!(paths("look at /a.png"), None);
    assert_eq!(paths("'/a.png"), None);
    assert_eq!(paths("file://host/a.png"), None);
    assert_eq!(paths("   "), None);
}

#[test]
fn a_long_description_shows_how_it_starts_once_left() {
    let dir = scratch("attach-long");
    let shot = png(&dir, "shot.png");
    let at = Instant::now();
    let mut engine = composing(at);
    drop_in(&mut engine, &shot.display().to_string(), at);
    tap(&mut engine, KeyCode::Tab, at);
    let long = format!(
        "a gradient from blue {}to pink",
        "and then some more ".repeat(5)
    );
    press(&mut engine, &long, at);
    let rows = panel(&mut engine, at);
    assert!(
        contains(&rows, "to pink"),
        "the cursor's end is shown: {rows:#?}"
    );
    tap(&mut engine, KeyCode::Tab, at);
    let rows = panel(&mut engine, at);
    assert!(contains(&rows, "a gradient from blue"), "{rows:#?}");
    assert!(contains(&rows, "…"), "{rows:#?}");
}
