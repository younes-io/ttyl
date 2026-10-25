//! Builds the docs site from the Markdown in `docs/` into `target/site`, drawing every screen
//! it shows with the app itself. Run `cargo test --test site`.

mod common;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{DynamicImage, Rgb, RgbImage};
use pulldown_cmark::{Event as MdEvent, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use ttyl::api::{Call, Error, Reply};
use ttyl::attach;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::keys::{self, Scope};
use ttyl::model::{
    Filter, GroupPage, Log, LogPage, NotificationPage, Profile, Row, RowPage, Segment,
};
use ttyl::theme;
use unicode_width::UnicodeWidthStr;

/// One screen the docs show: how the app gets there, and what it must show once it has.
struct Shot {
    name: &'static str,
    start: Start,
    steps: &'static [Step],
    alt: &'static str,
    shows: &'static str,
}

enum Start {
    SignedIn,
    SignedOut,
}

enum Step {
    Keys(&'static str),
    Tap(KeyCode),
    Ctrl(char),
    Paste(&'static str),
    DropPicture,
}

const WIDE: (u16, u16) = (120, 34);

/// dev-null.sh is a brand, so prose draws its name with the wordmark from the dev-null.sh brand kit.
const DEV_NULL: &str = "/dev/null";
const WORDMARK: &str =
    "<img class=\"wordmark\" src=\"dev-null-wordmark-dark.svg\" alt=\"/dev/null\">";
const NARROW: (u16, u16) = (52, 34);

/// The body of grace's commit in `fixtures/log-latest.json`, whose segments the preview answers with.
const FOOTGUN: &str = "today's footgun:\n\n```php\n$name = 'devnull';\necho \"<b>$name</b>\";\n```\nescape your output.";

const SHOTS: &[Shot] = &[
    Shot {
        name: "home",
        start: Start::SignedIn,
        steps: &[Step::Keys("2j"), Step::Tap(KeyCode::Enter)],
        alt: "ttyl with the latest log on the left and the open thread of the selected commit on the right.",
        shows: "thread",
    },
    Shot {
        name: "signin",
        start: Start::SignedOut,
        steps: &[],
        alt: "The first screen of ttyl, which asks you to press enter to sign in.",
        shows: "Sign in to read your logs.",
    },
    Shot {
        name: "latest",
        start: Start::SignedIn,
        steps: &[Step::Keys("2jj")],
        alt: "The latest log, one commit per card, with the selected commit's thread previewed beside it.",
        shows: "/latest",
    },
    Shot {
        name: "thread",
        start: Start::SignedIn,
        steps: &[Step::Keys("2jjjj"), Step::Tap(KeyCode::Enter)],
        alt: "A thread: the commit at the top and its replies under it.",
        shows: "margaret",
    },
    Shot {
        name: "compose",
        start: Start::SignedIn,
        steps: &[
            Step::Keys("2cwhich one do you reach for first?"),
            Step::Ctrl('p'),
            Step::Keys("grep"),
            Step::Tap(KeyCode::Enter),
            Step::Keys("ripgrep"),
        ],
        alt: "The composer open under the log, holding a new commit with a two-option poll.",
        shows: "ripgrep",
    },
    Shot {
        name: "attach",
        start: Start::SignedIn,
        steps: &[
            Step::Keys("2cnew desk, same bugs."),
            Step::DropPicture,
            Step::Tap(KeyCode::Tab),
            Step::Keys("a desk with two monitors"),
        ],
        alt: "The composer with an image attached and its description typed under it.",
        shows: "desk",
    },
    Shot {
        name: "preview",
        start: Start::SignedIn,
        steps: &[Step::Keys("2c"), Step::Paste(FOOTGUN), Step::Ctrl('r')],
        alt: "The preview of a draft, showing its code block in color as the commit would show it.",
        shows: "escape your output",
    },
    Shot {
        name: "vote",
        start: Start::SignedIn,
        steps: &[Step::Keys("2jjjjjjjv")],
        alt: "A poll asking tabs or spaces, open for a vote, with tabs picked.",
        shows: "spaces",
    },
    Shot {
        name: "images",
        start: Start::SignedIn,
        steps: &[Step::Keys("2jjjjjji")],
        alt: "The image viewer showing the first of a commit's two images, drawn with half blocks.",
        shows: "1 of 2",
    },
    Shot {
        name: "notifications",
        start: Start::SignedIn,
        steps: &[Step::Keys("gn")],
        alt: "Notifications: replies, quotes, stars, mentions and follows, newest first.",
        shows: "notifications",
    },
    Shot {
        name: "profile",
        start: Start::SignedIn,
        steps: &[Step::Keys("2jjjjp")],
        alt: "Margaret's profile: her bio and counts above the list of her commits.",
        shows: "@margaret",
    },
    Shot {
        name: "starred",
        start: Start::SignedIn,
        steps: &[Step::Keys("gs")],
        alt: "The commits you starred, newest star first.",
        shows: "starred",
    },
    Shot {
        name: "help",
        start: Start::SignedIn,
        steps: &[Step::Keys("2?")],
        alt: "The keys overlay that ? opens, listing every key for the screen you are on.",
        shows: "keys",
    },
];

fn ctrl(ch: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
}

fn tap(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// A stand-in for the fixture commit's first image, which lives on dev-null.sh: a desk with two monitors.
fn picture() -> DynamicImage {
    let (width, height) = (800, 600);
    let wall = Rgb([0x1d, 0x22, 0x33]);
    let desk = Rgb([0x7a, 0x4e, 0x2d]);
    let frame = Rgb([0x0e, 0x0e, 0x10]);
    let screen = Rgb([0x16, 0x1b, 0x2e]);
    let lines = [
        Rgb([0x7a, 0xa2, 0xf7]),
        Rgb([0xff, 0x8a, 0x4c]),
        Rgb([0x51, 0xcf, 0x66]),
        Rgb([0xda, 0x77, 0xf2]),
    ];
    let mut image = RgbImage::from_pixel(width, height, wall);
    let mut fill = |x0: u32, y0: u32, x1: u32, y1: u32, color: Rgb<u8>| {
        for y in y0..y1 {
            for x in x0..x1 {
                image.put_pixel(x, y, color);
            }
        }
    };
    fill(0, 430, width, height, desk);
    for left in [90, 430] {
        fill(left, 110, left + 280, 330, frame);
        fill(left + 12, 122, left + 268, 318, screen);
        for row in 0..9 {
            let indent = [0, 20, 20, 40, 40, 20, 0, 20, 0][row];
            let length = [150, 120, 180, 90, 140, 110, 60, 170, 100][row];
            let y = 136 + 20 * row as u32;
            fill(
                left + 28 + indent,
                y,
                left + 28 + indent + length,
                y + 8,
                lines[(row + left as usize / 430) % lines.len()],
            );
        }
        fill(left + 125, 330, left + 155, 410, frame);
        fill(left + 80, 410, left + 200, 430, frame);
    }
    fill(250, 470, 550, 500, Rgb([0x2a, 0x2a, 0x2e]));
    DynamicImage::ImageRgb8(image)
}

/// Answers `call` the way dev-null.sh would, from the fixtures.
fn serve(call: &Call) -> Result<Reply, Error> {
    Ok(match call {
        Call::Me => Reply::Account(common::account(3)),
        Call::Page(log) => Reply::Page(common::page(*log)),
        Call::Older { log, .. } => Reply::Page(LogPage {
            rows: Vec::new(),
            next_cursor: None,
            ..common::page(*log)
        }),
        Call::Commit(_) | Call::Replies { .. } => return common::from_world(call),
        Call::User(username) => Reply::User(Box::new(profile(username))),
        Call::Commits { username, .. } => Reply::Rows(RowPage {
            rows: latest()
                .into_iter()
                .filter(|row| &row.commit.author.username == username)
                .collect(),
            next_cursor: None,
        }),
        Call::Starred { .. } => Reply::Rows(RowPage {
            rows: latest()
                .into_iter()
                .filter(|row| row.commit.author.username != "ada")
                .map(|row| Row {
                    starred: true,
                    ..row
                })
                .collect(),
            next_cursor: None,
        }),
        Call::Notifications { .. } => Reply::Notifications(
            NotificationPage::parse(&fixture("notifications"), Filter::All).unwrap(),
        ),
        Call::Group { .. } => {
            Reply::Group(GroupPage::parse(&fixture("notifications-group")).unwrap())
        }
        Call::Preview(text) => Reply::Preview(segments(text)),
        other => panic!("the docs never ask dev-null.sh for {other:?}"),
    })
}

fn fixture(name: &str) -> String {
    fs::read_to_string(root().join(format!("fixtures/{name}.json"))).unwrap()
}

fn latest() -> Vec<Row> {
    common::page(Log::Latest).rows
}

/// What the server renders `text` as: the segments of the fixture commit with that body.
fn segments(text: &str) -> Vec<Segment> {
    let page: serde_json::Value = serde_json::from_str(&fixture("log-latest")).unwrap();
    let commit = page["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["body"] == text)
        .unwrap_or_else(|| panic!("no fixture commit says {text:?}"));
    serde_json::from_value(commit["body_segments"].clone()).unwrap()
}

fn profile(username: &str) -> Profile {
    let row = latest()
        .into_iter()
        .find(|row| row.commit.author.username == username)
        .unwrap_or_else(|| panic!("no fixture commit by {username}"));
    let json = serde_json::json!({
        "data": {
            "id": format!("u-{username}"),
            "username": username,
            "name": row.commit.author.name,
            "avatar_url": null,
            "html_url": format!("https://dev-null.sh/@{username}"),
            "lane": 1,
            "bio": "writes down what the locks protect.",
            "company": null,
            "location": "Arlington, VA",
            "website": "https://example.com",
            "github_url": format!("https://github.com/{username}"),
            "created_at": "2025-03-14T00:00:00Z",
            "commits_count": 42,
            "followers_count": 1337,
            "following_count": 7,
        },
        "is_following": true,
        "follows_you": false,
        "pinned_commit": null,
    });
    Profile::parse(&json.to_string()).unwrap()
}

/// Feeds the engine everything it asked for until it asks for nothing more.
fn settle(engine: &mut Engine, mut effects: Vec<Effect>, at: Instant) {
    loop {
        effects.extend(engine.pump(at));
        if effects.is_empty() {
            return;
        }
        for effect in std::mem::take(&mut effects) {
            let event = match effect {
                Effect::Run(ticket, call) => Event::Reply(ticket, serve(&call)),
                Effect::Fetch(url) => Event::Fetched(url, Ok(picture())),
                Effect::Read(paths, pasted) => Event::Read(
                    paths
                        .into_iter()
                        .map(|path| {
                            let look = attach::look(&path);
                            (path, look)
                        })
                        .collect(),
                    pasted,
                ),
                Effect::Draft(_) => continue,
                other => panic!("the docs never need {other:?}"),
            };
            effects.extend(engine.update(event, at));
        }
    }
}

fn shoot(shot: &Shot, picture: &Path) -> Engine {
    let at = Instant::now();
    let mut engine = match shot.start {
        Start::SignedIn => common::boot(at),
        Start::SignedOut => {
            let mut engine = Engine::new("ttyl".to_owned(), at);
            let effects = engine.pump(at);
            let [Effect::Run(me, Call::Me)] = effects.as_slice() else {
                panic!("the engine asks for /me first: {effects:?}");
            };
            let refused = Error::Api {
                status: 401,
                code: Some("device_not_approved".to_owned()),
                message: "device not approved".to_owned(),
                retry_after: None,
            };
            engine.update(Event::Reply(*me, Err(refused)), at);
            engine
        }
    };
    settle(&mut engine, Vec::new(), at);
    for step in shot.steps {
        let events = match step {
            Step::Keys(keys) => keys.chars().map(|ch| tap(KeyCode::Char(ch))).collect(),
            Step::Tap(code) => vec![tap(*code)],
            Step::Ctrl(ch) => vec![ctrl(*ch)],
            Step::Paste(text) => vec![Event::Paste((*text).to_owned())],
            Step::DropPicture => vec![Event::Paste(picture.display().to_string())],
        };
        for event in events {
            let effects = engine.update(event, at);
            settle(&mut engine, effects, at);
        }
    }
    engine
}

fn hex(color: Color) -> Option<String> {
    let (r, g, b) = match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("the app draws only RGB colors, not {other:?}"),
    };
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn css(cell: &ratatui::buffer::Cell) -> String {
    let mut css = String::new();
    if let Some(fg) = hex(cell.fg).filter(|fg| Some(fg) != hex(theme::FOREGROUND).as_ref()) {
        let _ = write!(css, "color:{fg};");
    }
    if let Some(bg) = hex(cell.bg).filter(|bg| Some(bg) != hex(theme::BACKGROUND).as_ref()) {
        let _ = write!(css, "background:{bg};");
    }
    for (modifier, rule) in [
        (Modifier::BOLD, "font-weight:bold;"),
        (Modifier::ITALIC, "font-style:italic;"),
        (Modifier::UNDERLINED, "text-decoration:underline;"),
    ] {
        if cell.modifier.contains(modifier) {
            css.push_str(rule);
        }
    }
    css
}

/// A half-block cell as a blank painted in two colors. The page puts a little space between
/// lines for reading, which would show as stripes across a picture drawn with the glyphs.
fn half_block(cell: &ratatui::buffer::Cell) -> Option<(String, &'static str)> {
    let fg = hex(cell.fg).or_else(|| hex(theme::FOREGROUND)).unwrap();
    let bg = hex(cell.bg).or_else(|| hex(theme::BACKGROUND)).unwrap();
    let (top, bottom) = match cell.symbol() {
        "\u{2580}" => (fg, bg),
        "\u{2584}" => (bg, fg),
        _ => return None,
    };
    Some((
        format!("background:linear-gradient({top} 50%,{bottom} 50%);"),
        " ",
    ))
}

fn html(buffer: &Buffer, alt: &str, class: &str) -> String {
    let area = buffer.area;
    let mut out = format!(
        "<pre class=\"term {class}\" style=\"--cols:{}\" role=\"img\" aria-label=\"{}\">",
        area.width,
        escape(alt)
    );
    for y in 0..area.height {
        let mut runs: Vec<(String, String)> = Vec::new();
        let mut x = 0;
        while x < area.width {
            let cell = &buffer[(x, y)];
            let (style, symbol) = half_block(cell).unwrap_or_else(|| (css(cell), cell.symbol()));
            match runs.last_mut() {
                Some((last, text)) if *last == style => text.push_str(symbol),
                _ => runs.push((style, symbol.to_owned())),
            }
            x += u16::try_from(symbol.width().max(1)).unwrap();
        }
        for (style, text) in runs {
            if style.is_empty() {
                out.push_str(&escape(&text));
            } else {
                let _ = write!(out, "<span style=\"{style}\">{}</span>", escape(&text));
            }
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

const CELL_WIDTH: f32 = 8.4;
const CELL_HEIGHT: f32 = 17.0;
const TITLE_BAR: f32 = 30.0;
const MARGIN: f32 = 32.0;
const INSET: f32 = 14.0;

struct Fill {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: String,
}

/// `buffer` as an SVG of a terminal window, for the README, which cannot show the HTML screens.
fn svg(buffer: &Buffer, alt: &str) -> String {
    let area = buffer.area;
    let background = hex(theme::BACKGROUND).unwrap();
    let foreground = hex(theme::FOREGROUND).unwrap();
    let window_width = f32::from(area.width) * CELL_WIDTH + 2.0 * INSET;
    let window_height = f32::from(area.height) * CELL_HEIGHT + TITLE_BAR + INSET;
    let (width, height) = (window_width + 2.0 * MARGIN, window_height + 2.0 * MARGIN);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"{alt}\" xml:space=\"preserve\">\
         <title>{alt}</title>\
         <defs><filter id=\"shadow\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"130%\">\
         <feDropShadow dx=\"0\" dy=\"12\" stdDeviation=\"14\" flood-opacity=\"0.45\"/></filter></defs>\
         <rect x=\"{MARGIN}\" y=\"{MARGIN}\" width=\"{window_width}\" height=\"{window_height}\" rx=\"10\" \
         fill=\"{background}\" stroke=\"#2a2a2e\" filter=\"url(#shadow)\"/>",
        alt = escape(alt)
    );
    for (i, color) in ["#ff5f57", "#febc2e", "#28c840"].into_iter().enumerate() {
        let cx = MARGIN + 18.0 + 20.0 * i as f32;
        let _ = write!(
            out,
            "<circle cx=\"{cx}\" cy=\"{}\" r=\"6\" fill=\"{color}\"/>",
            MARGIN + TITLE_BAR / 2.0
        );
    }
    let left = MARGIN + INSET;
    let top = MARGIN + TITLE_BAR;
    let _ = write!(
        out,
        "<g font-family=\"ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono',monospace\" \
         font-size=\"14\" fill=\"{foreground}\">"
    );
    for y in 0..area.height {
        let row_top = top + f32::from(y) * CELL_HEIGHT;
        let mut x = 0;
        let mut texts: Vec<(u16, u16, String, String)> = Vec::new();
        let mut fills: Vec<Fill> = Vec::new();
        while x < area.width {
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            let cells = u16::try_from(symbol.width().max(1)).unwrap();
            let fg = hex(cell.fg).unwrap_or_else(|| foreground.clone());
            let bg = hex(cell.bg).unwrap_or_else(|| background.clone());
            let cell_left = left + f32::from(x) * CELL_WIDTH;
            let cell_width = f32::from(cells) * CELL_WIDTH;
            let mut paint = |color: &str, y: f32, height: f32| match fills.last_mut() {
                Some(last)
                    if last.color == color
                        && last.y == y
                        && last.height == height
                        && (last.x + last.width - cell_left).abs() < 0.01 =>
                {
                    last.width += cell_width;
                }
                _ => fills.push(Fill {
                    x: cell_left,
                    y,
                    width: cell_width,
                    height,
                    color: color.to_owned(),
                }),
            };
            match symbol {
                "\u{2580}" | "\u{2584}" => {
                    let (upper, lower) = if symbol == "\u{2580}" {
                        (&fg, &bg)
                    } else {
                        (&bg, &fg)
                    };
                    paint(upper, row_top, CELL_HEIGHT / 2.0);
                    paint(lower, row_top + CELL_HEIGHT / 2.0, CELL_HEIGHT / 2.0);
                }
                _ => {
                    if bg != background {
                        paint(&bg, row_top, CELL_HEIGHT);
                    }
                    let mut style = String::new();
                    if fg != foreground {
                        let _ = write!(style, " fill=\"{fg}\"");
                    }
                    if cell.modifier.contains(Modifier::BOLD) {
                        style.push_str(" font-weight=\"bold\"");
                    }
                    if cell.modifier.contains(Modifier::ITALIC) {
                        style.push_str(" font-style=\"italic\"");
                    }
                    if cell.modifier.contains(Modifier::UNDERLINED) {
                        style.push_str(" text-decoration=\"underline\"");
                    }
                    match texts.last_mut() {
                        Some((_, width, last, text)) if *last == style => {
                            text.push_str(symbol);
                            *width += cells;
                        }
                        _ => texts.push((x, cells, style, symbol.to_owned())),
                    }
                }
            }
            x += cells;
        }
        for fill in fills {
            let _ = write!(
                out,
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{}\"/>",
                fill.x,
                fill.y,
                fill.width + 0.3,
                fill.height + 0.3,
                fill.color
            );
        }
        for (x, style, text) in texts.iter().flat_map(|(x, _, style, text)| {
            words(text).map(move |(offset, word)| (x + offset, style, word))
        }) {
            let _ = write!(
                out,
                "<text x=\"{:.1}\" y=\"{:.1}\" textLength=\"{:.1}\" lengthAdjust=\"spacingAndGlyphs\"{style}>{}</text>",
                left + f32::from(x) * CELL_WIDTH,
                row_top + CELL_HEIGHT * 0.76,
                f32::from(u16::try_from(text.width()).unwrap()) * CELL_WIDTH,
                escape(text)
            );
        }
    }
    out.push_str("</g></svg>");
    out
}

/// The parts of `text` between runs of two or more spaces, with the cell each starts at. An SVG
/// viewer drops the spaces at the end of a text, which would stretch the rest across them.
fn words(text: &str) -> impl Iterator<Item = (u16, &str)> {
    let mut cell = 0;
    let mut rest = text;
    std::iter::from_fn(move || {
        let start = rest.len() - rest.trim_start().len();
        cell += rest[..start].width();
        rest = &rest[start..];
        if rest.is_empty() {
            return None;
        }
        let end = rest.find("  ").unwrap_or(rest.len());
        let word = rest[..end].trim_end();
        let at = cell;
        cell += rest[..end].width();
        rest = &rest[end..];
        Some((u16::try_from(at).unwrap(), word))
    })
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The figure for `shot`, with the wide and narrow screens of which CSS shows one, and the
/// wide screen as an SVG.
fn figure(shot: &Shot, picture: &Path) -> (String, String) {
    let mut engine = shoot(shot, picture);
    let at = Instant::now();
    let wide = common::buffer(&mut engine, WIDE.0, WIDE.1, at);
    let text: String = wide.content().iter().map(|cell| cell.symbol()).collect();
    assert!(
        text.contains(shot.shows),
        "the {} screen no longer shows {:?}:\n{}",
        shot.name,
        shot.shows,
        rows(&wide)
    );
    let narrow = common::buffer(&mut engine, NARROW.0, NARROW.1, at);
    let figure = format!(
        "<figure class=\"screen\" id=\"screen-{}\">{}{}</figure>",
        shot.name,
        html(&wide, shot.alt, "wide"),
        html(&narrow, shot.alt, "narrow")
    );
    (figure, svg(&wide, shot.alt))
}

fn rows(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One page of the site: `docs/<name>.md`, and its place in the key bar.
struct Page {
    name: &'static str,
    key: char,
    label: &'static str,
}

const PAGES: &[Page] = &[
    Page {
        name: "index",
        key: '1',
        label: "home",
    },
    Page {
        name: "start",
        key: '2',
        label: "start",
    },
    Page {
        name: "read",
        key: '3',
        label: "read",
    },
    Page {
        name: "write",
        key: '4',
        label: "write",
    },
    Page {
        name: "react",
        key: '5',
        label: "react",
    },
    Page {
        name: "people",
        key: '6',
        label: "people",
    },
    Page {
        name: "keys",
        key: '7',
        label: "keys",
    },
    Page {
        name: "settings",
        key: '8',
        label: "settings",
    },
];

/// The key table's scopes in the order a reader meets them.
const SCOPES: &[Scope] = &[
    Scope::List,
    Scope::Log,
    Scope::Commit,
    Scope::Global,
    Scope::Always,
    Scope::Thread,
    Scope::Composer,
    Scope::Preview,
    Scope::Secret,
    Scope::Rules,
    Scope::Vote,
    Scope::Viewer,
    Scope::Inbox,
    Scope::Group,
    Scope::People,
    Scope::Profile,
    Scope::Person,
    Scope::Starred,
    Scope::Confirm,
    Scope::Report,
    Scope::Details,
    Scope::Help,
    Scope::Gate,
    Scope::Splash,
];

fn heading(scope: Scope) -> &'static str {
    match scope {
        Scope::List => "In any list",
        Scope::Log => "In a log",
        Scope::Commit => "On the selected commit",
        Scope::Global => "Wherever you read",
        Scope::Always => "Everywhere",
        Scope::Thread => "In a thread",
        Scope::Composer => "While writing",
        Scope::Preview => "In the preview",
        Scope::Secret => "When a draft looks like it holds a secret",
        Scope::Rules => "When a draft breaks the commit rules",
        Scope::Vote => "While voting",
        Scope::Viewer => "In the image viewer",
        Scope::Inbox => "In notifications",
        Scope::Group => "In a group of notifications",
        Scope::People => "In a list of people",
        Scope::Profile => "On a profile",
        Scope::Person => "On a person in a profile",
        Scope::Starred => "In starred commits",
        Scope::Confirm => "When ttyl asks you to confirm",
        Scope::Report => "While picking a report reason",
        Scope::Details => "While adding report details",
        Scope::Help => "In the keys overlay",
        Scope::Gate => "While you approve the terminal",
        Scope::Splash => "While ttyl cannot reach dev-null.sh",
    }
}

/// Every key the app knows, from the table its `?` overlay reads.
fn keys_reference() -> String {
    for binding in keys::TABLE {
        assert!(
            SCOPES.contains(&binding.scope),
            "the keys page leaves out {:?}; add it to SCOPES",
            binding.scope
        );
    }
    let mut out = String::new();
    for &scope in SCOPES {
        let id = format!("{scope:?}").to_lowercase();
        let _ = write!(
            out,
            "<section class=\"panel\" id=\"{id}\" aria-labelledby=\"{id}-title\">\
             <h2 id=\"{id}-title\">{}</h2><dl class=\"keys\">",
            heading(scope)
        );
        for (chords, help, _) in keys::help(&[scope], &|_| true) {
            let _ = write!(
                out,
                "<div><dt>{}</dt><dd>{}</dd></div>",
                kbd(&chords),
                escape(help)
            );
        }
        out.push_str("</dl></section>\n");
    }
    out
}

/// `j/↓` as two keys, `/` alone as one.
fn kbd(chords: &str) -> String {
    let parts: Vec<&str> = chords.split('/').collect();
    let parts = if parts.iter().any(|part| part.is_empty()) {
        vec![chords]
    } else {
        parts
    };
    parts
        .iter()
        .map(|part| format!("<kbd>{}</kbd>", escape(part)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn href(page: &Page) -> String {
    if page.name == "index" {
        "./".to_owned()
    } else {
        format!("{}.html", page.name)
    }
}

/// The key bar every page carries, marking the current page as the app marks the current log.
fn nav(current: &Page) -> String {
    let mut out = String::new();
    for page in PAGES {
        let href = href(page);
        if page.name == current.name {
            let _ = write!(
                out,
                "<a href=\"{href}\" aria-current=\"page\" data-key=\"{}\"><span class=\"slash\">/</span>{}</a>",
                page.key, page.label
            );
        } else {
            let _ = write!(
                out,
                "<a href=\"{href}\" data-key=\"{0}\"><kbd>{0}</kbd> {1}</a>",
                page.key, page.label
            );
        }
    }
    out
}

/// A page's Markdown as HTML, with each paragraph that is only `{{marker}}` replaced by what
/// `fill` makes of it, and the text of its first heading.
fn markdown(source: &str, mut fill: impl FnMut(&str) -> String) -> (String, String) {
    let mut events: Vec<MdEvent> = Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_HEADING_ATTRIBUTES,
    )
    .collect();
    for at in 0..events.len() {
        let slug = match &events[at] {
            MdEvent::Start(Tag::Heading { id: None, .. }) => slug(&events[at + 1..]),
            _ => continue,
        };
        if let MdEvent::Start(Tag::Heading { id, .. }) = &mut events[at] {
            *id = Some(slug.into());
        }
    }
    let mut kept = Vec::with_capacity(events.len());
    let mut title = String::new();
    let mut in_title = false;
    let mut in_code = false;
    let mut at = 0;
    while at < events.len() {
        if let [
            MdEvent::Start(Tag::Paragraph),
            MdEvent::Text(text),
            MdEvent::End(TagEnd::Paragraph),
            ..,
        ] = &events[at..]
            && let Some(marker) = text
                .strip_prefix("{{")
                .and_then(|rest| rest.strip_suffix("}}"))
        {
            kept.push(MdEvent::Html(fill(marker).into()));
            at += 3;
            continue;
        }
        match &events[at] {
            MdEvent::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) if title.is_empty() => {
                in_title = true;
            }
            MdEvent::End(TagEnd::Heading(HeadingLevel::H1)) => in_title = false,
            MdEvent::Text(text) | MdEvent::Code(text) if in_title => title.push_str(text),
            MdEvent::Start(Tag::CodeBlock(_)) => in_code = true,
            MdEvent::End(TagEnd::CodeBlock) => in_code = false,
            _ => {}
        }
        match &events[at] {
            MdEvent::Text(text) if !in_code => {
                for (nth, part) in text.split(DEV_NULL).enumerate() {
                    if nth > 0 {
                        kept.push(MdEvent::InlineHtml(WORDMARK.into()));
                    }
                    if !part.is_empty() {
                        kept.push(MdEvent::Text(part.to_owned().into()));
                    }
                }
            }
            event => kept.push(event.clone()),
        }
        at += 1;
    }
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, kept.into_iter());
    (out, title)
}

/// The anchor a heading gets from its text, the way GitHub makes one: "View images" becomes
/// `view-images`.
fn slug(heading: &[MdEvent]) -> String {
    let mut out = String::new();
    for event in heading {
        match event {
            MdEvent::End(TagEnd::Heading(_)) => break,
            MdEvent::Text(text) | MdEvent::Code(text) => out.push_str(text),
            _ => {}
        }
    }
    out.to_lowercase()
        .chars()
        .filter_map(|ch| match ch {
            ' ' => Some('-'),
            ch if ch.is_alphanumeric() || ch == '-' => Some(ch),
            _ => None,
        })
        .collect()
}

/// Every link from one page to another lands on a page that exists, and on an anchor that
/// page has.
fn check_links(files: &[(String, Vec<u8>)]) {
    let html = |name: &str| {
        files
            .iter()
            .find(|(file, _)| file == name)
            .map(|(_, bytes)| std::str::from_utf8(bytes).unwrap())
    };
    for (file, bytes) in files.iter().filter(|(file, _)| file.ends_with(".html")) {
        let text = std::str::from_utf8(bytes).unwrap();
        for link in text.split("href=\"").skip(1) {
            let link = &link[..link.find('"').unwrap()];
            if link.contains("://") || link.starts_with("data:") {
                continue;
            }
            let (target, anchor) = link.split_once('#').unwrap_or((link, ""));
            let target = match target {
                "" => file.as_str(),
                "./" => "index.html",
                target => target,
            };
            let page = html(target)
                .unwrap_or_else(|| panic!("{file} links to {link}, which the site does not have"));
            assert!(
                anchor.is_empty() || page.contains(&format!("id=\"{anchor}\"")),
                "{file} links to {link}, but {target} has no id {anchor:?}"
            );
        }
    }
}

fn page_html(template: &str, title: &str, nav: &str, main: &str) -> String {
    let mut out = template.to_owned();
    for (marker, value) in [("{{title}}", escape(title)), ("{{nav}}", nav.to_owned())] {
        assert!(out.contains(marker), "template.html has no {marker}");
        out = out.replace(marker, &value);
    }
    let (before, after) = out
        .split_once("{{main}}")
        .expect("template.html has no {{main}}");
    format!("{before}{main}{after}")
}

fn site(picture: &Path) -> Vec<(String, Vec<u8>)> {
    let docs = root().join("docs");
    let template = fs::read_to_string(docs.join("template.html")).unwrap();
    for entry in fs::read_dir(&docs).unwrap() {
        let path = entry.unwrap().path();
        if let Some(name) = path
            .file_stem()
            .filter(|_| path.extension() == Some("md".as_ref()))
        {
            assert!(
                PAGES.iter().any(|page| name == page.name),
                "{} is not in PAGES, so no page links to it",
                path.display()
            );
        }
    }
    let mut unused: Vec<&str> = SHOTS.iter().map(|shot| shot.name).collect();
    let mut files = Vec::new();
    let mut svgs = Vec::new();
    for page in PAGES {
        let file = format!("{}.md", page.name);
        let source = fs::read_to_string(docs.join(&file)).unwrap();
        let (main, title) = markdown(&source, |marker| match marker.split_once(' ') {
            Some(("screen", name)) => {
                let shot = SHOTS
                    .iter()
                    .find(|shot| shot.name == name)
                    .unwrap_or_else(|| panic!("{file}: no screen is called {name:?}"));
                unused.retain(|unused| *unused != name);
                let (figure, svg) = figure(shot, picture);
                svgs.push((format!("screen-{name}.svg"), svg.into_bytes()));
                figure
            }
            None if marker == "keys" => keys_reference(),
            _ => panic!("{file}: unknown marker {{{{{marker}}}}}"),
        });
        assert!(!title.is_empty(), "{file} starts without a # title");
        assert!(
            !main.contains("{{"),
            "{file}: a marker must be alone on its line"
        );
        let html = page_html(&template, &title, &nav(page), &main);
        files.push((format!("{}.html", page.name), html.into_bytes()));
    }
    assert!(unused.is_empty(), "no page shows the screens {unused:?}");
    files.append(&mut svgs);
    files.push((
        "style.css".to_owned(),
        fs::read(docs.join("style.css")).unwrap(),
    ));
    files.push(("desk.png".to_owned(), fs::read(picture).unwrap()));
    for asset in [
        "dev-null-wordmark-dark.svg",
        "dev-null-wordmark-light.svg",
        "install.sh",
    ] {
        files.push((asset.to_owned(), fs::read(docs.join(asset)).unwrap()));
    }
    check_links(&files);
    files
}

#[test]
fn the_docs_site_builds() {
    // SAFETY: this binary runs this one test, before the engine starts any thread.
    unsafe { std::env::set_var("TTYL_ICONS", "plain") };
    let picture_path = common::scratch("site").join("desk.png");
    picture().save(&picture_path).unwrap();
    let files = site(&picture_path);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .unwrap()
        .join("site");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).unwrap();
    for (name, bytes) in files {
        fs::write(out.join(name), bytes).unwrap();
    }
}
