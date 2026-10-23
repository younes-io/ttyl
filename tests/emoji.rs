mod common;

use std::time::Instant;

use common::{boot, contains, press, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::draft::Target;
use ttyl::emoji::{search, trigger};

fn names(query: &str) -> Vec<&'static str> {
    search(query).into_iter().map(|emoji| emoji.name).collect()
}

#[test]
fn names_starting_with_the_query_come_first_then_later_words_then_the_rest() {
    assert_eq!(names("laughi"), ["laughing"]);
    assert_eq!(&names("joy")[..3], ["joy", "joy_cat", "joystick"]);
    let cat = names("cat");
    let first_word = cat
        .iter()
        .position(|name| !name.starts_with("cat"))
        .unwrap();
    assert!(
        cat[first_word..]
            .iter()
            .take_while(|name| name.split('_').any(|word| word.starts_with("cat")))
            .count()
            > 0,
        "{cat:?}"
    );
    assert_eq!(cat.last().map(|name| name.contains("cat")), Some(true));
}

#[test]
fn every_match_is_listed_and_each_emoji_once() {
    let found = search("an");
    assert!(found.len() > 100, "no cap: {}", found.len());
    let mut glyphs: Vec<_> = found.iter().map(|emoji| emoji.glyph).collect();
    glyphs.sort_unstable();
    glyphs.dedup();
    assert_eq!(glyphs.len(), found.len());
    assert!(
        !names("bee").contains(&"honeybee"),
        "honeybee is bee under a longer name"
    );
}

#[test]
fn a_shortcode_opens_after_a_space_or_the_start_with_two_characters() {
    assert_eq!(trigger(":la"), Some(0));
    assert_eq!(trigger("hi :la"), Some(3));
    assert_eq!(trigger("(:+1"), Some(1));
    for quiet in ["10:30", "hi :l", "a:la", "::la", "hi :la ", "hi"] {
        assert_eq!(trigger(quiet), None, "{quiet:?}");
    }
}

fn composing(text: &str, at: Instant) -> ttyl::engine::Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    press(&mut engine, text, at);
    engine
}

#[test]
fn typing_a_shortcode_lists_its_matches_above_the_panel() {
    let at = Instant::now();
    let mut engine = composing("hi :laughi", at);
    let rows = screen(&mut engine, 100, 30, at);
    let title = rows
        .iter()
        .position(|row| row.contains(" :laughi ") && row.contains(" 1/1 "))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(rows[title + 1].contains("laughing"), "{rows:#?}");
    assert!(rows[title + 2].contains("tab insert"), "{rows:#?}");
    assert!(rows[title + 3].starts_with(" commit ─"), "{rows:#?}");

    press(&mut engine, " ", at);
    assert!(!contains(&screen(&mut engine, 100, 30, at), " :laughi "));
}

#[test]
fn tab_or_enter_swaps_the_shortcode_for_the_selected_emoji() {
    let at = Instant::now();
    let mut engine = composing("hi :laughi", at);
    tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(engine.draft(&Target::New), Some("hi 😆"));

    let mut engine = composing(":joy", at);
    tap(&mut engine, KeyCode::Down, at);
    tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(engine.draft(&Target::New), Some("😹"));
    tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(
        engine.draft(&Target::New),
        Some("😹\n"),
        "enter types again"
    );

    let mut engine = composing(":joy", at);
    tap(&mut engine, KeyCode::Up, at);
    tap(&mut engine, KeyCode::Tab, at);
    let last = search("joy").last().unwrap().glyph;
    assert_eq!(
        engine.draft(&Target::New),
        Some(last),
        "up wraps to the end"
    );
}

#[test]
fn esc_hides_the_list_first_and_closes_the_panel_second() {
    let at = Instant::now();
    let mut engine = composing("ok :joy", at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, " :joy "), "{rows:#?}");
    assert!(contains(&rows, " commit ─"), "the panel stays: {rows:#?}");

    press(&mut engine, "s", at);
    assert!(
        !contains(&screen(&mut engine, 100, 30, at), " :joys "),
        "the same shortcode stays hidden"
    );
    press(&mut engine, " :ca", at);
    assert!(contains(&screen(&mut engine, 100, 30, at), " :ca "));

    tap(&mut engine, KeyCode::Esc, at);
    tap(&mut engine, KeyCode::Esc, at);
    assert!(!contains(&screen(&mut engine, 100, 30, at), " commit ─"));
    assert_eq!(engine.draft(&Target::New), Some("ok :joys :ca"));
}
