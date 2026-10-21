use std::collections::HashSet;

use chrono::{DateTime, FixedOffset};
use devnull::app::App;
use devnull::code;
use devnull::feed;
use devnull::model::{Commit, Entry, Log, LogPage, Segment, Status, Tombstone};
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;

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

fn fixture_commit() -> Commit {
    match &page("log-latest").entries[0] {
        Entry::Commit(commit) => (**commit).clone(),
        Entry::Tombstone(_) => panic!("the first latest entry is a commit"),
    }
}

fn text(line: &Line) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z]{1,12}",
        "[a-z]{20,90}",
        "\\PC{1,12}",
        prop::sample::select(vec![
            "👩‍💻",
            "🇫🇷",
            "❤️",
            "漢字かな",
            "e\u{301}",
            "ｗｉｄｅ",
            "`code`"
        ])
        .prop_map(str::to_owned),
    ]
}

fn body() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            4 => word(),
            2 => Just(" ".to_owned()),
            1 => prop::sample::select(vec!["\n", "\n\n", "\t", "   "]).prop_map(str::to_owned),
        ],
        0..40,
    )
    .prop_map(|parts| parts.concat())
}

fn segment() -> impl Strategy<Value = Segment> {
    let block_line = "[a-z(){}=;$' ]{0,70}";
    prop_oneof![
        body().prop_map(|text| Segment::Text { text }),
        "[a-z(){}=;$ ]{1,40}".prop_map(|code| Segment::Code { code }),
        prop::collection::vec(block_line, 1..6).prop_map(|lines| Segment::Block {
            language: None,
            html: lines
                .iter()
                .map(|line| format!("<span class=\"hl-keyword\">{line}</span>"))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
    ]
}

fn commit() -> impl Strategy<Value = Commit> {
    (
        prop::collection::vec(segment(), 1..6),
        prop::option::of(prop::collection::vec(segment(), 1..3)),
        prop::option::of("\\PC{0,60}"),
        "[a-z0-9_]{1,40}",
        body(),
        0i64..20_000,
        prop::sample::select(vec![Status::Approved, Status::Pending, Status::Rejected]),
        prop::option::of(0u32..2_000_000),
        prop::option::of(0u32..2_000_000),
    )
        .prop_map(
            |(segments, preview, name, username, body, hours, status, replies, stars)| {
                let mut commit = fixture_commit();
                commit.body_segments = segments;
                commit.preview_segments = preview;
                commit.author.name = name;
                commit.author.username = username;
                commit.body = body;
                commit.created_at = now().to_utc() - chrono::Duration::hours(hours);
                commit.status = status;
                commit.replies_count = replies;
                commit.stars_count = stars;
                commit
            },
        )
}

fn entry() -> impl Strategy<Value = Entry> {
    prop_oneof![
        4 => commit().prop_map(|commit| Entry::Commit(Box::new(commit))),
        1 => Just(Entry::Tombstone(Tombstone {
            id: devnull::model::CommitId("deleted".to_owned()),
            parent_id: None,
            deleted_at: now().to_utc(),
            replies_count: Some(2),
        })),
    ]
}

fn any_page() -> impl Strategy<Value = LogPage> {
    (
        prop::sample::select(vec![Log::Following, Log::Latest, Log::Trending]),
        prop::collection::vec(entry(), 0..5),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(entry(), 0..4),
    )
        .prop_map(|(log, entries, more, is_ranked, suggestions)| LogPage {
            log,
            entries,
            next_cursor: more.then(|| "cursor".to_owned()),
            is_ranked,
            new_commits_count: None,
            suggestions,
        })
}

fn commit_page(commit: Commit) -> LogPage {
    LogPage {
        entries: vec![Entry::Commit(Box::new(commit))],
        ..page("log-latest")
    }
}

proptest! {
    #[test]
    fn every_line_fits_the_width(
        page in any_page(),
        width in 10u16..160,
        selected in prop::option::of(0usize..5),
        expand in any::<bool>(),
    ) {
        let expanded = page
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Commit(commit) if expand => Some(commit.id.clone()),
                _ => None,
            })
            .collect();
        let feed = feed::layout(&page, width, selected, &expanded, &now());
        for line in &feed.lines {
            prop_assert!(line.width() <= usize::from(width), "{width}: {:?}", text(line));
        }
    }

    #[test]
    fn layout_keeps_every_visible_character_of_a_text_body_in_order(
        body in body(),
        width in 10u16..160,
    ) {
        let mut commit = fixture_commit();
        commit.body_segments = vec![Segment::Text { text: body.clone() }];
        commit.preview_segments = None;
        let page = commit_page(commit);
        let feed = feed::layout(&page, width, Some(0), &HashSet::new(), &now());
        let rendered: String = feed
            .lines
            .iter()
            .flat_map(|line| line.spans.iter().skip(1))
            .flat_map(|span| span.content.chars())
            .filter(|ch| !ch.is_whitespace())
            .collect();
        let expected: String = body.chars().filter(|ch| !ch.is_whitespace()).collect();
        prop_assert!(rendered.contains(&expected), "{expected:?} missing from {rendered:?}");
    }

    #[test]
    fn a_suggestion_shows_its_body_on_one_line_cut_with_an_ellipsis(
        body in body(),
        width in 10u16..160,
    ) {
        let mut following = page("log-following");
        let mut commit = fixture_commit();
        commit.body = body.clone();
        following.suggestions = vec![Entry::Commit(Box::new(commit))];
        let feed = feed::layout(&following, width, None, &HashSet::new(), &now());
        let at = feed
            .lines
            .iter()
            .position(|line| text(line).starts_with("● "))
            .expect("the suggestion header");
        let line = &feed.lines[at + 1];
        prop_assert!(line.width() <= usize::from(width), "{width}: {:?}", text(line));
        let shown = text(line);
        let shown = shown.strip_prefix("  ").expect("indented under the node");
        let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let fits = shown == collapsed;
        let cut = shown
            .strip_suffix('…')
            .is_some_and(|prefix| collapsed.starts_with(prefix) && prefix != collapsed);
        prop_assert!(fits || cut, "{shown:?} from {collapsed:?}");
    }

    #[test]
    fn block_lines_never_panics(html in "[<>&;#xX0-9a-f/\"= spanclsdiy:\n-]{0,80}|\\PC{0,80}") {
        code::block_lines(&html);
    }

    #[test]
    fn block_text_without_markup_comes_back_unchanged(source in "[^<&]{0,120}") {
        let lines = code::block_lines(&source);
        let joined = lines
            .iter()
            .map(|line| line.iter().map(|span| span.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        prop_assert_eq!(joined, source);
    }

    #[test]
    fn the_selected_header_stays_on_screen(
        keys in prop::collection::vec(prop::sample::select(vec!['j', 'k', 'g', 'G', ' ']), 0..30),
        width in 70u16..120,
        height in 4u16..30,
    ) {
        let latest = page("log-latest");
        let hashes: Vec<_> = latest
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Commit(commit) => commit.short_hash.clone(),
                Entry::Tombstone(_) => "[deleted]".to_owned(),
            })
            .collect();
        let mut app = App::new([page("log-following"), latest, page("log-trending")]);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut selected = 0usize;
        let last = hashes.len() - 1;
        for key in std::iter::once('2').chain(keys) {
            selected = match key {
                'j' => (selected + 1).min(last),
                'k' => selected.saturating_sub(1),
                'g' => 0,
                'G' => last,
                _ => selected,
            };
            app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
            terminal.draw(|frame| app.draw(frame, &now())).unwrap();
        }
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        prop_assert!(
            rows.iter().any(|row| row.contains(&hashes[selected])),
            "{} not on screen: {rows:#?}",
            hashes[selected]
        );
    }
}
