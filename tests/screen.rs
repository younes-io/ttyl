mod common;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use common::{boot, boot_serving, contains, following_nobody, now, page, secs};
use ratatui::style::Color;
use ttyl::api::{Call, Error};
use ttyl::engine::Effect;
use ttyl::engine::{Engine, Event};
use ttyl::feed;
use ttyl::model::{Log, LogPage};
use ttyl::theme;

fn app() -> (Engine, Instant) {
    let at = Instant::now();
    (boot(at), at)
}

fn press(app: &mut (Engine, Instant), keys: &str) {
    common::press(&mut app.0, keys, app.1);
}

fn screen(app: &mut (Engine, Instant), width: u16, height: u16) -> Vec<String> {
    common::screen(&mut app.0, width, height, app.1)
}

#[test]
fn opens_on_following_with_the_empty_state_and_suggestions() {
    let at = Instant::now();
    let mut app = (
        boot_serving(at, |log| match log {
            Log::Following => following_nobody(),
            other => page(other),
        }),
        at,
    );
    let rows = screen(&mut app, 120, 30);
    assert!(contains(
        &rows,
        "fatal: your log does not have any commits yet"
    ));
    assert!(contains(
        &rows,
        "Here is what a few developers committed lately:"
    ));
    let suggestions: Vec<_> = rows
        .iter()
        .skip(1)
        .filter(|row| row.starts_with(" ● "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        suggestions,
        [
            " ● Grace @grace  today's footgun: $name = 'devnull'; escape your output.",
            " ● Linus @linus  first commit here. hello /dev/null.",
            " ● Margaret @margaret  notes from rewriting our scheduler. 1. the queue was never the \
             bottleneck. 2. the lock around t…",
        ],
        "one line per suggestion, the code block cut to its first line: {rows:#?}"
    );
}

#[test]
fn suggestions_end_on_a_way_to_trending() {
    let at = Instant::now();
    let mut engine = boot_serving(at, |log| match log {
        Log::Following => following_nobody(),
        other => page(other),
    });
    let rows = common::screen(&mut engine, 120, 30, at);
    let last = rows
        .iter()
        .rposition(|row| row.starts_with(" ● "))
        .expect("suggestions");
    assert_eq!(rows[last + 2], " Or browse Trending for more.", "{rows:#?}");
    let buffer = common::buffer(&mut engine, 120, 30, at);
    let y = u16::try_from(last + 2).unwrap();
    assert_eq!(
        buffer[(11, y)].fg,
        theme::PRIMARY,
        "Trending reads as a link"
    );
    assert_ne!(buffer[(1, y)].fg, theme::PRIMARY);
}

#[test]
fn following_nobody_with_nothing_to_suggest_points_at_trending() {
    let at = Instant::now();
    let mut engine = boot_serving(at, |log| match log {
        Log::Following => LogPage {
            suggestions: Vec::new(),
            ..following_nobody()
        },
        other => page(other),
    });
    let rows = common::screen(&mut engine, 120, 30, at);
    assert_eq!(
        rows[2..5],
        [
            " fatal: your log does not have any commits yet",
            "",
            " Your log shows your commits and commits from people you follow. Find someone worth following on Trending.",
        ],
        "{rows:#?}"
    );
    assert!(!contains(&rows, "Or browse"), "{rows:#?}");
}

#[test]
fn an_empty_latest_asks_for_the_first_commit() {
    let at = Instant::now();
    let mut engine = boot_serving(at, |log| LogPage {
        rows: Vec::new(),
        suggestions: Vec::new(),
        ..page(log)
    });
    common::press(&mut engine, "2", at);
    let rows = common::screen(&mut engine, 120, 30, at);
    assert_eq!(
        rows[2..5],
        [
            " fatal: no commits yet",
            "",
            " Nobody has committed anything. Be the first. It's just plain text; you'll be fine.",
        ],
        "{rows:#?}"
    );
}

#[test]
fn moving_up_onto_the_first_commit_of_a_day_shows_its_day() {
    let mut app = app();
    press(&mut app, "2G");
    while app.0.log().unwrap().selected() > 4 {
        press(&mut app, "k");
    }
    let rows = screen(&mut app, 80, 12);
    assert!(contains(&rows, "0000401"), "{rows:#?}");
    assert!(contains(&rows, "┆ Yesterday"), "{rows:#?}");

    press(&mut app, "gg");
    let rows = screen(&mut app, 80, 12);
    assert!(contains(&rows, "┆ Today"), "{rows:#?}");
}

#[test]
fn a_suggestion_shows_its_code_in_code_style() {
    let at = Instant::now();
    let mut engine = boot_serving(at, |log| match log {
        Log::Following => following_nobody(),
        other => page(other),
    });
    let buffer = common::buffer(&mut engine, 120, 30, at);
    let styled = |text: &str| {
        let (y, row) = (0..30)
            .map(|y| {
                let row: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
                (y, row)
            })
            .find(|(_, row)| row.contains(text))
            .unwrap_or_else(|| panic!("{text} is on screen"));
        let x = row[..row.find(text).unwrap()].chars().count();
        buffer[(u16::try_from(x).unwrap(), y)].bg
    };
    assert_eq!(
        styled("$name"),
        theme::MUTED,
        "code sits on the code background"
    );
    assert_ne!(styled("escape"), theme::MUTED, "prose does not");
}

#[test]
fn latest_draws_headers_day_markers_and_badges() {
    let mut app = app();
    press(&mut app, "2");
    let rows = screen(&mut app, 70, 120);
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
    assert!(contains(&rows, "@ada · 29m  in review"), "{rows:#?}");
    assert!(contains(&rows, "@ada · 15h  rejected"), "{rows:#?}");
    assert!(
        first.starts_with(&format!(
            " ● Ada @ada · 18m · {} 3  {} 12",
            theme::icons().reply,
            theme::icons().star
        )),
        "counts ride the header: {first:?}"
    );
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
    assert!(
        contains(&folded, "+8 lines · space to expand"),
        "{folded:#?}"
    );
    assert!(contains(&folded, "┆ Yesterday"));

    press(&mut app, " ");
    let unfolded = screen(&mut app, 70, 40);
    assert!(!contains(&unfolded, "+8 lines"));
    assert!(
        contains(&unfolded, "10."),
        "the tenth list item shows once unfolded: {unfolded:#?}"
    );
}

fn commit_rows(rows: &[String], hash: &str) -> Vec<String> {
    let start = rows
        .iter()
        .position(|row| row.contains(" ● ") && row.trim_end().ends_with(hash))
        .unwrap_or_else(|| panic!("{hash} is on screen: {rows:#?}"));
    let end = rows[start + 1..]
        .iter()
        .position(|row| row.starts_with(" ● ") || row.starts_with(" ┆"))
        .map_or(rows.len(), |end| start + 1 + end);
    rows[start..end].to_vec()
}

#[test]
fn a_quoted_commit_takes_one_line_under_the_quoting_commit() {
    let mut app = app();
    press(&mut app, "2");
    let rows = commit_rows(&screen(&mut app, 70, 120), "e4f5a62");
    assert_eq!(
        rows[2].trim_end(),
        format!(
            " │ {} Margaret @margaret  notes from rewriting our scheduler. 1. the …",
            theme::icons().quote
        ),
        "{rows:#?}"
    );
}

#[test]
fn a_deleted_quote_and_a_deleted_reply_read_as_bad_objects() {
    let mut app = app();
    press(&mut app, "2");
    let rows = screen(&mut app, 70, 120);
    let quote = commit_rows(&rows, "0000a01");
    assert!(
        contains(
            &quote,
            &format!("│ {} fatal: bad object 0000b01", theme::icons().quote)
        ),
        "{quote:#?}"
    );
    let reply = commit_rows(&rows, "0000201");
    assert!(
        reply
            .iter()
            .any(|row| row.trim_end().ends_with("○ fatal: bad object 0000202")),
        "{reply:#?}"
    );
}

#[test]
fn the_newest_reply_previews_on_one_line_under_its_commit() {
    let mut app = app();
    press(&mut app, "2");
    let rows = commit_rows(&screen(&mut app, 70, 120), "e4f5a61");
    assert_eq!(
        rows[3].trim_end(),
        " ┃   ● Grace @grace · 10m  @ada did you keep an index on created_at?"
    );
}

#[test]
fn links_images_and_polls_each_lead_with_their_icon() {
    let mut app = app();
    press(&mut app, "2");
    let rows = screen(&mut app, 80, 120);
    assert!(
        contains(
            &commit_rows(&rows, "0000601"),
            &format!(
                "│ {} We need tool support for keyset pagination · use-the-index-luke.com",
                theme::icons().link
            )
        ),
        "{rows:#?}"
    );
    for width in [100, 44] {
        let rows = screen(&mut app, width, 120);
        let images = commit_rows(&rows, "0000701");
        for line in [
            format!(
                "│ {} a desk with two monitors · 1600×1200",
                theme::icons().image
            ),
            format!("│ {} no description · 1200×1600", theme::icons().image),
        ] {
            assert!(contains(&images, &line), "{line}: {images:#?}");
        }
        let poll = commit_rows(&rows, "0000801");
        for line in [
            format!("│ {} ○ tabs", theme::icons().poll),
            "│   ○ spaces".to_owned(),
            "│   closes in ".to_owned(),
        ] {
            assert!(contains(&poll, &line), "{line}: {poll:#?}");
        }
    }
    // A narrow screen shortens the description and keeps the size whole.
    let rows = screen(&mut app, 41, 120);
    let cut = format!(
        "│ {} a desk with two monito… · 1600×1200",
        theme::icons().image
    );
    assert!(contains(&rows, &cut), "{rows:#?}");
}

#[test]
fn selection_stays_on_screen_while_moving_down_a_short_terminal() {
    let mut app = app();
    press(&mut app, "2");
    for author in [
        "@ada",
        "@ada",
        "@grace",
        "@linus",
        "@margaret",
        "@grace",
        "@ken",
        "@margaret",
        "@ada",
        "@linus",
        "@ken",
    ] {
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
    let rows = screen(&mut app, 70, 120);
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
    let page = page(Log::Latest);
    let expanded = page
        .rows
        .iter()
        .map(|row| row.commit.id.clone())
        .collect::<HashSet<_>>();
    for width in [24, 40, 57, 80, 120] {
        let (feed, _) = feed::layout(&page, width, &expanded, true, &now());
        for line in &feed.lines {
            assert!(line.width() <= usize::from(width), "{width}: {line:?}");
        }
        assert_eq!(feed.entries.len(), page.rows.len());
    }
}

#[test]
fn the_selected_header_is_always_on_screen() {
    for log_key in ["1", "2", "3"] {
        for height in 4..=14u16 {
            let at = Instant::now();
            let mut engine = boot(at);
            common::press(&mut engine, log_key, at);
            let rows = page(engine.log().unwrap().active()).rows;
            let mut path: Vec<char> = Vec::new();
            path.extend(std::iter::repeat_n('j', rows.len() + 2));
            path.extend(std::iter::repeat_n('k', rows.len() + 2));
            path.extend(['G', ' ', 'k', ' ', 'j', 'g', 'g', ' ', 'j', ' ']);
            for key in path {
                engine.update(Event::Key(common::key(key)), at);
                let screen = common::screen(&mut engine, 120, height, at);
                let selected = engine.log().unwrap().selected();
                let hash = &rows[selected.min(rows.len() - 1)].commit.short_hash;
                assert!(
                    screen
                        .iter()
                        .any(|row| row.contains(hash.as_str()) && row.contains('●')),
                    "log {log_key} height {height} key {key} selected {selected}: {screen:#?}"
                );
            }
        }
    }
}

#[test]
fn the_top_bar_never_overlaps_and_shrinks_the_logo_to_its_mark_then_drops_the_handle_then_the_count()
 {
    let mut app = app();
    let rows = [
        (100, " // /dev/null"),
        (26, " // /dev/null   ● 3  @ada"),
        (25, " //            ● 3  @ada"),
        (16, " //   ● 3  @ada"),
        (15, " //       ● 3"),
        (10, " //     ●"),
        (7, "     ●"),
    ];
    for (width, top) in rows {
        let row = &screen(&mut app, width, 20)[0];
        assert!(row.starts_with(top), "{width}: {row:?}");
    }
    let top = &screen(&mut app, 100, 20)[0];
    assert!(top.ends_with("● 3  @ada"), "{top:?}");
    assert_eq!(top.chars().count(), 99, "{top:?}");
}

#[test]
fn the_logs_are_the_feed_box_top_edge_with_the_open_one_slashed() {
    let mut app = app();
    press(&mut app, "2");
    let rows = screen(&mut app, 120, 12);
    assert!(rows[1].is_empty(), "{rows:#?}");
    assert!(
        rows[2].starts_with(" ╭─ following ── /latest ── trending ───"),
        "{rows:#?}"
    );
    let buffer = common::buffer(&mut app.0, 120, 12, app.1);
    let bold = ratatui::style::Modifier::BOLD;
    assert_eq!(buffer[(17, 2)].symbol(), "/");
    assert_eq!(buffer[(17, 2)].fg, theme::ACCENT);
    assert_eq!(buffer[(18, 2)].fg, theme::FOREGROUND);
    assert!(buffer[(18, 2)].modifier.contains(bold));
    assert_eq!(buffer[(4, 2)].fg, theme::FAINT);
    assert!(!buffer[(4, 2)].modifier.contains(bold));
    assert_eq!(buffer[(2, 2)].fg, theme::PRIMARY);

    let rows = screen(&mut app, 100, 12);
    assert_eq!(
        rows[1],
        format!(" ─ following ── /latest ── trending {}", "─".repeat(63)),
        "a feed too narrow for its box keeps the edge as a rule"
    );
    let rows = screen(&mut app, 30, 12);
    assert_eq!(rows[1], format!(" ─ 1 ── /2 ── 3 {}", "─".repeat(13)));
}

#[test]
fn a_narrow_hint_line_keeps_help_and_drops_the_other_hints_from_the_end() {
    let mut app = app();
    press(&mut app, "2jj");
    let wide = screen(&mut app, 100, 20)[19].clone();
    assert_eq!(
        wide,
        " j/k move  enter open  s star  c commit  ? help  Q quit"
    );
    let rows = screen(&mut app, 44, 20);
    assert_eq!(
        rows[19], " j/k move  enter open  ? help  Q quit",
        "{rows:#?}"
    );
    let rows = screen(&mut app, 30, 20);
    assert_eq!(rows[19], " j/k move  ? help  Q quit", "{rows:#?}");
    let rows = screen(&mut app, 12, 20);
    assert_eq!(rows[19], " ? help", "{rows:#?}");
}

#[test]
fn a_narrow_footer_error_still_ends_in_help() {
    let at = Instant::now();
    let mut engine = boot(at);
    common::press(&mut engine, "2", at);
    let effects = common::reload(&mut engine, at);
    let [Effect::Run(ticket, Call::Page(Log::Latest))] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    engine.update(Event::Reply(*ticket, Err(Error::Unexpected(502))), at);
    let rows = common::screen(&mut engine, 120, 20, at + secs(1));
    assert_eq!(
        rows[19],
        " fatal: the site is in maintenance · your log is still here · retrying in 29s  ? help",
        "{rows:#?}"
    );
    for width in [44u16, 30] {
        let rows = common::screen(&mut engine, width, 20, at + secs(1));
        assert!(rows[19].starts_with(" fatal: "), "{width}: {rows:#?}");
        assert!(rows[19].ends_with("  ? help"), "{width}: {rows:#?}");
        assert!(
            rows[19].chars().count() <= usize::from(width) - 2,
            "{width}: the spinner cells stay free: {rows:#?}"
        );
    }
}

#[test]
fn a_wide_terminal_fetches_the_selected_commits_thread_into_a_pane_once_the_selection_rests() {
    let at = Instant::now();
    let mut engine = boot(at);
    common::press(&mut engine, "2", at);
    let pane = |rows: &[String]| -> Vec<String> {
        rows.iter()
            .map(|row| row.chars().skip(72).collect::<String>())
            .collect()
    };
    let thread_calls = |effects: &[Effect]| {
        common::runs(effects)
            .into_iter()
            .filter(|call| matches!(call, Call::Commit(_) | Call::Replies { .. }))
            .count()
    };
    let rows = common::screen(&mut engine, 120, 40, at);
    assert!(
        rows.iter().all(|row| !row
            .chars()
            .take(72)
            .collect::<String>()
            .contains("@grace · 10m")),
        "the reply leaves the feed: {rows:#?}"
    );
    assert!(
        contains(&pane(&rows), "switched the feed to cursorPaginate(20)"),
        "the pane shows the selected commit at once: {rows:#?}"
    );
    assert_eq!(
        thread_calls(&engine.pump(at)),
        0,
        "moving past sends nothing"
    );

    let rested = at + std::time::Duration::from_millis(300);
    let effects = engine.pump(rested);
    assert_eq!(thread_calls(&effects), 2, "{effects:?}");
    common::answer(&mut engine, effects, rested);
    let rows = common::screen(&mut engine, 120, 40, rested);
    assert!(
        contains(&pane(&rows), "│ did you keep an index on created_at?"),
        "{rows:#?}"
    );

    common::press(&mut engine, "j", rested);
    common::screen(&mut engine, 100, 40, rested);
    assert_eq!(
        thread_calls(&engine.pump(rested + secs(1))),
        0,
        "a narrow terminal has no pane to fetch for"
    );
}

#[test]
fn the_selected_commit_is_a_card_of_even_rows_with_padding_above_and_below() {
    let mut app = app();
    press(&mut app, "2");
    let buffer = common::buffer(&mut app.0, 80, 30, app.1);
    let tinted: Vec<(u16, u16, u16)> = (1..29)
        .filter_map(|y| {
            let xs: Vec<u16> = (0..80)
                .filter(|&x| {
                    ![Color::Reset, theme::BACKGROUND, theme::MUTED].contains(&buffer[(x, y)].bg)
                })
                .collect();
            Some((y, *xs.first()?, *xs.last()?))
        })
        .collect();
    let row = |y: u16| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>();
    let header = tinted
        .iter()
        .position(|&(y, _, _)| row(y).contains("@ada"))
        .expect("the header is on the card");
    assert_eq!(header, 1, "one padding row above the header: {tinted:?}");
    assert!(
        row(tinted[tinted.len() - 1].0).trim().starts_with('│'),
        "the gap below pads the card"
    );
    let (_, left, right) = tinted[0];
    assert!(
        tinted.iter().all(|&(_, l, r)| (l, r) == (left, right)),
        "every card row spans the same width: {tinted:?}"
    );
    assert!(
        usize::from(right) + 1 >= row(tinted[header].0).trim_end().chars().count(),
        "the card reaches past the text"
    );
}

#[test]
fn z_covers_the_screen_with_a_drifting_wordmark_until_any_key() {
    let mut app = app();
    press(&mut app, "2");
    let feed = screen(&mut app, 80, 30);
    press(&mut app, "z");
    let ink = |rows: &[String]| -> Vec<(usize, usize)> {
        rows.iter()
            .enumerate()
            .flat_map(|(y, row)| {
                row.chars()
                    .enumerate()
                    .filter(|(_, ch)| "█▀▄".contains(*ch))
                    .map(move |(x, _)| (x, y))
            })
            .collect()
    };
    let first = screen(&mut app, 80, 30);
    assert!(!contains(&first, "@ada"), "the feed is hidden: {first:#?}");
    assert!(!ink(&first).is_empty(), "the wordmark is drawn: {first:#?}");
    let later = common::screen(&mut app.0, 80, 30, app.1 + secs(2));
    assert_ne!(ink(&first), ink(&later), "the wordmark drifts");
    press(&mut app, "j");
    assert_eq!(
        screen(&mut app, 80, 30),
        feed,
        "the first key only brings the feed back"
    );
}

#[test]
fn the_screensaver_wordmark_melts_into_life_while_a_new_one_drifts_in() {
    let mut app = app();
    press(&mut app, "z");
    let solid = |rows: &[String]| -> Vec<(usize, usize)> {
        let cells: Vec<_> = rows
            .iter()
            .enumerate()
            .flat_map(|(y, row)| {
                row.chars()
                    .enumerate()
                    .filter(|(_, ch)| *ch == '█')
                    .map(move |(x, _)| (x, y))
            })
            .collect();
        let left = cells.iter().map(|(x, _)| *x).min().unwrap_or(0);
        let top = cells.iter().map(|(_, y)| *y).min().unwrap_or(0);
        cells
            .into_iter()
            .map(|(x, y)| (x - left, y - top))
            .collect()
    };
    let ink = |rows: &[String]| {
        rows.iter()
            .flat_map(|row| row.chars())
            .filter(|ch| *ch != ' ')
            .count()
    };
    let at = |ms: u64| app.1 + Duration::from_millis(ms);
    let first = common::screen(&mut app.0, 80, 30, at(0));
    let logos = [
        solid(&first),
        solid(&common::screen(&mut app.0, 80, 30, at(110))),
    ];
    let wordmarks = |rows: &[String]| {
        let grid: Vec<Vec<char>> = rows.iter().map(|row| row.chars().collect()).collect();
        let solid_at = |x: usize, y: usize| grid.get(y).and_then(|row| row.get(x)) == Some(&'█');
        (0..30)
            .flat_map(|top| (0..80).map(move |left| (left, top)))
            .filter(|&(left, top)| {
                logos
                    .iter()
                    .any(|logo| logo.iter().all(|&(x, y)| solid_at(left + x, top + y)))
            })
            .count()
    };
    assert_eq!(
        wordmarks(&common::screen(&mut app.0, 80, 30, at(3_000))),
        2,
        "the wordmark melts as a new one appears"
    );
    assert!(
        ink(&common::screen(&mut app.0, 80, 30, at(6_000))) > ink(&first),
        "life spreads around the new wordmark"
    );
    for s in 0..120 {
        let frame = common::screen(&mut app.0, 80, 30, at(s * 500));
        assert!(wordmarks(&frame) >= 1, "a wordmark is on screen at {s}");
    }
}

fn voted_poll(log: Log) -> LogPage {
    let mut page = page(log);
    for row in &mut page.rows {
        if let Some(poll) = row.media.poll.as_mut() {
            poll.has_voted = true;
            poll.total_votes = Some(12);
            for (option, (percentage, voted)) in
                poll.options.iter_mut().zip([(75, true), (25, false)])
            {
                option.percentage = Some(percentage);
                option.voted = voted;
            }
        }
    }
    page
}

fn bar(rows: &[String], label: &str) -> String {
    let row = rows
        .iter()
        .find(|row| row.contains(label) && row.contains('%'))
        .unwrap_or_else(|| panic!("{label} has a bar: {rows:#?}"));
    let from = row.find(['▇', '░']).unwrap();
    row[from..].split(label).next().unwrap().trim().to_owned()
}

#[test]
fn poll_bars_add_up_to_their_percentages() {
    let at = Instant::now();
    let mut app = (boot_serving(at, voted_poll), at);
    press(&mut app, "2");
    let rows = screen(&mut app, 100, 120);
    assert_eq!(bar(&rows, "tabs"), "▇▇▇▇▇▇▇▇░░  75%", "{rows:#?}");
    assert_eq!(bar(&rows, "spaces"), "▇▇░░░░░░░░  25%", "{rows:#?}");
}

#[test]
fn a_poll_fills_its_bars_the_first_time_the_cursor_lands_on_it() {
    let at = Instant::now();
    let mut engine = boot_serving(at, voted_poll);
    common::press(&mut engine, "2jjjjjjj", at);
    let tabs = |engine: &mut Engine, after: u64| {
        bar(
            &common::screen(engine, 100, 60, at + Duration::from_millis(after)),
            "tabs",
        )
    };
    assert_eq!(tabs(&mut engine, 0), "░░░░░░░░░░   0%");
    let half = tabs(&mut engine, 300);
    assert!(
        half != "░░░░░░░░░░   0%" && half != "▇▇▇▇▇▇▇▇░░  75%",
        "midway: {half}"
    );
    assert_eq!(tabs(&mut engine, 2000), "▇▇▇▇▇▇▇▇░░  75%");

    common::press(&mut engine, "kj", at + secs(3));
    assert_eq!(
        tabs(&mut engine, 3000),
        "▇▇▇▇▇▇▇▇░░  75%",
        "it fills only once"
    );
}
