mod common;

use std::time::Instant;

use common::{boot, contains, press, screen, tap};
use ratatui::crossterm::event::KeyCode;

#[test]
fn gn_opens_the_inbox_and_esc_returns_to_the_same_place() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("notifications"), "{rows:#?}");
    assert!(!rows[0].contains("latest"), "{rows:#?}");
    assert_eq!(rows[1], " all  replies  quotes  stars  mentions  follows");
    assert_eq!(rows[29], " tab filter  esc back  ? help  Q quit");

    press(&mut engine, "gn", at);
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(
        screen(&mut engine, 100, 30, at),
        before,
        "one esc undoes gn"
    );
}

#[test]
fn esc_from_the_inbox_returns_to_the_thread_it_was_opened_from() {
    let at = Instant::now();
    let mut engine = boot(at);
    common::serve(&mut engine, at);
    tap(&mut engine, KeyCode::Enter, at);
    common::serve(&mut engine, at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    assert!(engine.thread().is_none());
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(screen(&mut engine, 100, 30, at), before);
}

#[test]
fn tab_and_shift_tab_cycle_the_filters_and_wrap() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let bold = |engine: &mut devnull::engine::Engine| {
        let buffer = common::buffer(engine, 100, 30, at);
        (0..100u16)
            .filter(|x| {
                buffer[(*x, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
            .map(|x| buffer[(x, 1)].symbol().to_owned())
            .collect::<String>()
    };
    assert_eq!(bold(&mut engine), "all");
    tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(bold(&mut engine), "replies");
    tap(&mut engine, KeyCode::BackTab, at);
    tap(&mut engine, KeyCode::BackTab, at);
    assert_eq!(bold(&mut engine), "follows");
}

#[test]
fn the_narrow_filter_bar_and_hints_fit_44_columns() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 44, 20, at);
    assert_eq!(rows[1], " all replies quotes stars mentions follows");
    assert!(contains(&rows[19..], "? help"), "{rows:#?}");
}

fn notifications() -> devnull::model::NotificationPage {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications.json"
    ))
    .unwrap();
    devnull::model::NotificationPage::parse(&json, devnull::model::Filter::All).unwrap()
}

fn laid_out(page: &devnull::model::NotificationPage, more: bool, width: u16) -> Vec<String> {
    let feed = devnull::inbox::layout(
        page.filter,
        &page.groups,
        page.read_at,
        more,
        width,
        &common::now(),
    );
    feed.lines
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

#[test]
fn each_type_reads_as_the_web_words_it_with_the_time_at_the_right_edge() {
    let rows = laid_out(&notifications(), false, 100);
    let sentences: Vec<&str> = rows
        .iter()
        .filter(|row| row.starts_with('●') || row.starts_with('○'))
        .map(|row| row[4..].trim_end_matches(char::is_alphanumeric).trim())
        .collect();
    assert_eq!(
        sentences,
        [
            "Grace @grace replied to your commit",
            "Grace and 4 others starred your commit",
            "Margaret and 1 other followed you",
            "Ken @ken quoted your commit",
            "Linus @linus mentioned you",
            "Linus and 1 other starred your commit",
            "Your commit was approved.",
            "Your commit was rejected.",
            "You leveled up.",
            "That's the oldest notification. There's nothing before it.",
        ]
    );
    assert!(rows[0].ends_with(" 1h"), "{rows:#?}");
    assert_eq!(rows[0].chars().count(), 100);
}

#[test]
fn a_row_unread_before_this_load_gets_a_filled_node() {
    let rows = laid_out(&notifications(), true, 100);
    let nodes: String = rows
        .iter()
        .filter_map(|row| row.chars().next())
        .filter(|node| *node != '│')
        .collect();
    assert_eq!(nodes, "●●○○○○○○○", "null and equal read_at were unread");
}

#[test]
fn the_line_under_a_row_is_its_commit_or_the_rejection_reason() {
    let rows = laid_out(&notifications(), true, 100);
    let under = |sentence: &str| {
        let at = rows.iter().position(|row| row.contains(sentence)).unwrap();
        rows[at + 1].clone()
    };
    assert_eq!(
        under("replied"),
        "│ today's footgun: $name = 'devnull'; escape your output."
    );
    assert_eq!(under("rejected"), "│ reason: Links to a paid course.");
    assert_eq!(under("followed you"), "│", "a follow has no commit line");
    assert!(rows.last().unwrap().contains("leveled up"), "{rows:#?}");
}

#[test]
fn a_group_without_names_for_its_count_says_how_many_updates_it_holds() {
    let mut page = notifications();
    page.groups[0].event_count = 3;
    let rows = laid_out(&page, true, 100);
    assert!(
        rows[0].starts_with("● Grace replied to your commit · 3 updates"),
        "{rows:#?}"
    );
}

#[test]
fn rows_fit_44_columns_and_keep_the_time() {
    for row in laid_out(&notifications(), true, 44) {
        assert!(row.chars().count() <= 44, "{row}");
    }
    let rows = laid_out(&notifications(), true, 44);
    assert_eq!(rows[0], "● Grace @grace replied to your commit     1h");
    assert!(
        rows[3].ends_with("… 1h") || rows[3].ends_with(" 1h"),
        "{rows:#?}"
    );
}

#[test]
fn an_empty_list_reads_as_the_web_empty_copy_for_its_filter() {
    let mut page = notifications();
    page.groups.clear();
    page.filter = devnull::model::Filter::Stars;
    assert_eq!(
        laid_out(&page, false, 100),
        [
            "$ cat /dev/notifications",
            "No stars yet. When someone stars one of your commits, it shows up here."
        ]
    );
}
