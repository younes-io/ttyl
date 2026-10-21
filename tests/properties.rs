mod common;

use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, FixedOffset};
use devnull::code;
use devnull::engine::Engine;
use devnull::feed;
use devnull::model::{Log, LogPage, Row, Segment, Status, Suggestion};
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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

fn fixture_row() -> Row {
    page("log-latest").rows[0].clone()
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

fn row() -> impl Strategy<Value = Row> {
    (
        prop::collection::vec(segment(), 1..6),
        prop::option::of(prop::collection::vec(segment(), 1..3)),
        "\\PC{0,60}",
        "[a-z0-9_]{1,40}",
        body(),
        0i64..20_000,
        prop::sample::select(vec![Status::Approved, Status::Pending, Status::Rejected]),
        0u32..2_000_000,
        0u32..2_000_000,
    )
        .prop_map(
            |(segments, preview, name, username, body, hours, status, replies, stars)| {
                let mut row = fixture_row();
                row.replies_count = replies;
                row.stars_count = stars;
                let commit = &mut row.commit;
                commit.body_segments = segments;
                commit.preview_segments = preview;
                commit.author.name = name;
                commit.author.username = username;
                commit.body = body;
                commit.created_at = now().to_utc() - chrono::Duration::hours(hours);
                commit.status = status;
                row
            },
        )
}

fn attachments(row: Row) -> impl Strategy<Value = Row> {
    let rows = page("log-latest").rows;
    let donor = move |hash: &str| {
        rows.iter()
            .find(|row| row.commit.short_hash == hash)
            .unwrap()
            .clone()
    };
    let quotes = vec![None, Some(donor("e4f5a62")), Some(donor("0000a01"))];
    let replies = vec![donor("0000301"), donor("e4f5a61"), donor("0000201")];
    let media = vec![
        donor("0000301"),
        donor("0000601"),
        donor("0000701"),
        donor("0000801"),
    ];
    (
        prop::sample::select(quotes),
        prop::sample::select(replies),
        prop::sample::select(media),
        prop::option::of(body()),
        "\\PC{0,80}",
    )
        .prop_map(move |(quote, reply, media, title, url)| {
            let mut row = row.clone();
            if let Some(quote) = quote {
                row.commit.quoted_commit_id = quote.commit.quoted_commit_id;
                row.quoted_commit = quote.quoted_commit;
            }
            row.reply_preview = reply.reply_preview;
            row.media = media.media;
            if let Some(preview) = &mut row.media.link_preview {
                preview.title = title;
                preview.url = url;
            }
            row
        })
}

fn suggestion(row: Row) -> Suggestion {
    Suggestion {
        commit: row.commit,
        stars_count: row.stars_count,
        starred: row.starred,
    }
}

fn any_page() -> impl Strategy<Value = LogPage> {
    (
        prop::sample::select(vec![Log::Following, Log::Latest, Log::Trending]),
        prop::collection::vec(row().prop_flat_map(attachments), 0..5),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(row().prop_map(suggestion), 0..4),
    )
        .prop_map(|(log, rows, more, is_ranked, suggestions)| LogPage {
            log,
            rows,
            next_cursor: more.then(|| "cursor".to_owned()),
            is_ranked,
            new_commits_count: None,
            suggestions,
        })
}

fn row_page(row: Row) -> LogPage {
    LogPage {
        rows: vec![row],
        ..page("log-latest")
    }
}

proptest! {
    #[test]
    fn every_line_fits_the_width(
        page in any_page(),
        width in 10u16..160,
        expand in any::<bool>(),
    ) {
        let expanded = page
            .rows
            .iter()
            .filter(|_| expand)
            .map(|row| row.commit.id.clone())
            .collect();
        let feed = feed::layout(&page, width, &expanded, &now());
        for line in &feed.lines {
            prop_assert!(line.width() <= usize::from(width), "{width}: {:?}", text(line));
        }
    }

    #[test]
    fn layout_keeps_every_visible_character_of_a_text_body_in_order(
        body in body(),
        width in 10u16..160,
    ) {
        let mut row = fixture_row();
        row.commit.body_segments = vec![Segment::Text { text: body.clone() }];
        row.commit.preview_segments = None;
        let page = row_page(row);
        let feed = feed::layout(&page, width, &HashSet::new(), &now());
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
        let mut row = fixture_row();
        row.commit.body_segments = vec![Segment::Text { text: body.clone() }];
        let author = row.commit.author.clone();
        following.rows.clear();
        following.suggestions = vec![suggestion(row)];
        let feed = feed::layout(&following, width, &HashSet::new(), &now());
        let line = feed
            .lines
            .iter()
            .find(|line| text(line).starts_with("● "))
            .expect("the suggestion row");
        prop_assert!(line.width() <= usize::from(width), "{width}: {:?}", text(line));
        let shown = text(line);
        let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let full = format!("● {} @{}  {collapsed}", author.name, author.username);
        let fits = shown == full;
        let cut = shown
            .strip_suffix('…')
            .is_some_and(|prefix| full.starts_with(prefix) && prefix != full);
        prop_assert!(fits || cut, "{shown:?} from {full:?}");
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
        keys in prop::collection::vec(prop::sample::select(vec!["j", "k", "gg", "G", " "]), 0..30),
        width in 70u16..120,
        height in 4u16..30,
    ) {
        let hashes: Vec<_> = page("log-latest")
            .rows
            .iter()
            .map(|row| row.commit.short_hash.clone())
            .collect();
        let at = Instant::now();
        let mut engine: Engine = common::boot(at);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut selected = 0usize;
        let last = hashes.len() - 1;
        for key in std::iter::once("2").chain(keys) {
            selected = match key {
                "j" => (selected + 1).min(last),
                "k" => selected.saturating_sub(1),
                "gg" => 0,
                "G" => last,
                _ => selected,
            };
            common::press(&mut engine, key, at);
            terminal.draw(|frame| engine.draw(frame, at, &now())).unwrap();
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

#[test]
fn a_suggestion_cut_right_after_its_author_still_ends_in_an_ellipsis() {
    let mut following = page("log-following");
    let mut row = fixture_row();
    row.commit.body_segments = vec![Segment::Text {
        text: "hello".to_owned(),
    }];
    following.rows.clear();
    following.suggestions = vec![suggestion(row)];
    let feed = feed::layout(&following, 12, &HashSet::new(), &now());
    let line = feed
        .lines
        .iter()
        .find(|line| text(line).starts_with("● "))
        .expect("the suggestion row");
    assert_eq!(text(line), "● Ada @ada …");
}
