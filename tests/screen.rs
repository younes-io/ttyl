use std::collections::HashSet;

use chrono::{DateTime, FixedOffset};
use devnull::app::App;
use devnull::feed;
use devnull::model::LogPage;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn page(name: &str) -> LogPage {
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    LogPage::parse(&json).unwrap()
}

fn now() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-10-21T10:00:00+00:00").unwrap()
}

fn app() -> App {
    App::new(
        "ada".to_owned(),
        [
            page("log-following"),
            page("log-latest"),
            page("log-trending"),
        ],
    )
}

fn press(app: &mut App, keys: &str) {
    for ch in keys.chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
}

fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame, &now())).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn contains(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|row| row.contains(needle))
}

#[test]
fn opens_on_following_with_the_empty_state_and_suggestions() {
    let rows = screen(&mut app(), 120, 30);
    assert!(contains(
        &rows,
        "fatal: your log does not have any commits yet"
    ));
    assert!(contains(
        &rows,
        "Here is what a few developers committed lately:"
    ));
    let authors: Vec<_> = rows
        .iter()
        .filter(|row| row.contains(" ● "))
        .map(|row| row.rsplit('@').next().unwrap())
        .collect();
    assert_eq!(
        authors,
        ["ada", "grace", "linus"],
        "one suggestion per author, newest first"
    );
    assert!(
        contains(&rows, "today's footgun: ```php $name = 'devnull';"),
        "a suggestion body reads as one collapsed line: {rows:#?}"
    );
}

#[test]
fn latest_draws_headers_day_markers_and_badges() {
    let mut app = app();
    press(&mut app, "2");
    let rows = screen(&mut app, 70, 60);
    assert!(rows[0].ends_with("@ada"), "signed-in user: {:?}", rows[0]);
    let first = rows
        .iter()
        .skip(1)
        .find(|row| row.contains("@ada"))
        .expect("first commit header");
    assert!(first.starts_with(" ● Ada @ada · 18m"), "{first:?}");
    assert!(
        first.ends_with(char::is_alphanumeric),
        "short hash sits at the right edge: {first:?}"
    );
    assert!(contains(&rows, "┆ Today"));
    assert!(contains(&rows, "@linus · 2h  in review"));
    assert!(contains(&rows, "↩ 3   ★ 12"));
    assert!(contains(&rows, "today's footgun:"));
    assert!(
        contains(&rows, "echo \"<b>$name</b>\";"),
        "code block decodes entities: {rows:#?}"
    );
}

#[test]
fn space_unfolds_the_selected_long_commit() {
    let mut app = app();
    press(&mut app, "2jjjj");
    let folded = screen(&mut app, 70, 40);
    assert!(contains(&folded, "… show more"));
    assert!(contains(&folded, "┆ Yesterday"));

    press(&mut app, " ");
    let unfolded = screen(&mut app, 70, 40);
    assert!(!contains(&unfolded, "… show more"));
    assert!(
        contains(&unfolded, "10."),
        "the tenth list item shows once unfolded: {unfolded:#?}"
    );
}

#[test]
fn selection_stays_on_screen_while_moving_down_a_short_terminal() {
    let mut app = app();
    press(&mut app, "2");
    for author in ["@ada", "@ada", "@grace", "@linus", "@margaret", "@ken"] {
        let rows = screen(&mut app, 50, 12);
        let header = rows
            .iter()
            .find(|row| row.contains(" ● ") && row.contains(author));
        assert!(
            header.is_some(),
            "{author} header is visible when selected: {rows:#?}"
        );
        press(&mut app, "j");
    }
}

#[test]
fn trending_is_ranked_so_it_has_no_day_markers_and_ends_at_the_root() {
    let mut app = app();
    press(&mut app, "3");
    let rows = screen(&mut app, 70, 80);
    assert!(!contains(&rows, "┆"));
    assert!(contains(
        &rows,
        "○ That's the root commit. There's nothing before it."
    ));
    let order: Vec<_> = rows
        .iter()
        .skip(1)
        .filter_map(|row| row.find('@').map(|_| row.clone()))
        .collect();
    assert!(
        order[0].contains("@margaret"),
        "most starred first: {order:#?}"
    );
}

#[test]
fn every_line_fits_the_width_it_was_laid_out_for() {
    let page = page("log-latest");
    let expanded = page
        .entries
        .iter()
        .filter_map(|entry| match entry {
            devnull::model::Entry::Commit(commit) => Some(commit.id.clone()),
            devnull::model::Entry::Tombstone(_) => None,
        })
        .collect::<HashSet<_>>();
    for width in [24, 40, 57, 80, 120] {
        let feed = feed::layout(&page, width, Some(2), &expanded, &now());
        for line in &feed.lines {
            assert!(line.width() <= usize::from(width), "{width}: {line:?}");
        }
        assert_eq!(feed.entries.len(), page.entries.len());
    }
}
