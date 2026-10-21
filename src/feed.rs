//! Lays a log page out as terminal lines, the way the web's commit feed draws it: one node per commit on a
//! graph rail, day markers on chronological logs, and a root node once the log has no older page.

use std::collections::HashSet;
use std::mem;
use std::ops::Range;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::code;
use crate::model::{
    Commit, CommitId, Log, LogPage, Media, Quote, Reply, Row, Segment, Shown, Status,
};
use crate::theme;

const RAIL_WIDTH: usize = 2;
const TAB: &str = "    ";

pub struct Feed {
    pub lines: Vec<Line<'static>>,
    /// The lines each of the page's rows occupies, in page order.
    pub entries: Vec<Range<usize>>,
}

pub fn layout<Tz: TimeZone>(
    page: &LogPage,
    width: u16,
    selected: Option<usize>,
    expanded: &HashSet<CommitId>,
    now: &DateTime<Tz>,
) -> Feed {
    let mut out = Writer {
        lines: Vec::new(),
        width: usize::from(width).max(RAIL_WIDTH + 8),
    };
    if page.rows.is_empty() {
        empty(&mut out, page);
        return Feed {
            lines: out.lines,
            entries: Vec::new(),
        };
    }

    let mut entries = Vec::with_capacity(page.rows.len());
    let mut last_day = None;
    for (index, row) in page.rows.iter().enumerate() {
        if index > 0 {
            out.gap();
        }
        if !page.is_ranked {
            let day = row
                .commit
                .created_at
                .with_timezone(&now.timezone())
                .date_naive();
            if last_day != Some(day) {
                out.push(
                    Span::styled("┆ ", theme::muted()),
                    vec![Span::styled(day_label(day, now), theme::muted())],
                );
                out.gap();
                last_day = Some(day);
            }
        }
        let start = out.lines.len();
        row_lines(
            &mut out,
            row,
            selected == Some(index),
            expanded.contains(&row.commit.id),
            now,
        );
        entries.push(start..out.lines.len());
    }

    let last_lane = page.rows.last().map(|row| row.commit.author.lane);
    if let (None, Some(lane)) = (&page.next_cursor, last_lane) {
        out.gap();
        out.node(
            Span::styled("○ ", Style::new().fg(theme::lane(lane))),
            Span::raw("  "),
            vec![Span::styled(
                "That's the root commit. There's nothing before it.",
                theme::muted(),
            )],
        );
    }

    Feed {
        lines: out.lines,
        entries,
    }
}

struct Writer {
    lines: Vec<Line<'static>>,
    width: usize,
}

impl Writer {
    fn content(&self) -> usize {
        self.width - RAIL_WIDTH
    }

    fn push(&mut self, rail: Span<'static>, spans: Vec<Span<'static>>) {
        self.node(rail.clone(), rail, spans);
    }

    /// Wraps `spans` to the content width, with `node` on the first line and `rail` on the rest.
    fn node(&mut self, node: Span<'static>, rail: Span<'static>, spans: Vec<Span<'static>>) {
        let pieces: Vec<_> = spans
            .into_iter()
            .map(|span| (span.content.into_owned(), span.style))
            .collect();
        let mut lines = Vec::new();
        wrap(&pieces, self.content(), &mut lines);
        for (index, mut line) in lines.into_iter().enumerate() {
            line.insert(
                0,
                if index == 0 {
                    node.clone()
                } else {
                    rail.clone()
                },
            );
            self.lines.push(Line::from(line));
        }
    }

    fn line(&mut self, rail: Span<'static>, mut spans: Vec<Span<'static>>) {
        spans.insert(0, rail);
        self.lines.push(Line::from(spans));
    }

    fn gap(&mut self) {
        self.push(Span::styled("│", Style::new().fg(theme::RAIL)), Vec::new());
    }
}

fn row_lines<Tz: TimeZone>(
    out: &mut Writer,
    row: &Row,
    selected: bool,
    expanded: bool,
    now: &DateTime<Tz>,
) {
    let commit = &row.commit;
    let lane = Style::new().fg(theme::lane(commit.author.lane));
    let rail = if selected {
        Span::styled("┃ ", lane)
    } else {
        Span::styled("│ ", Style::new().fg(theme::RAIL))
    };
    let header = header(commit, out.content(), now);
    out.node(Span::styled("● ", lane), rail.clone(), header);

    let folded = commit.preview_segments.as_ref().filter(|_| !expanded);
    for line in body(folded.unwrap_or(&commit.body_segments), out.content()) {
        out.push(rail.clone(), line);
    }
    if folded.is_some() {
        out.push(
            rail.clone(),
            vec![Span::styled("… show more", Style::new().fg(theme::PRIMARY))],
        );
    }
    let mut lines = Vec::new();
    web_only(&row.media, out.content(), &mut lines);
    for line in lines {
        out.line(rail.clone(), line);
    }
    if commit.quoted_commit_id.is_some() {
        quote(out, &rail, row.quoted_commit.as_ref(), now);
    }
    let mut lines = Vec::new();
    link_preview(&row.media, out.content(), &mut lines);
    for line in lines {
        out.line(rail.clone(), line);
    }

    let star = if row.starred {
        Style::new().fg(theme::AMBER)
    } else {
        theme::muted()
    };
    out.push(
        rail.clone(),
        vec![
            Span::styled(format!("↩ {}", row.replies_count), theme::muted()),
            Span::raw("   "),
            Span::styled(format!("★ {}", row.stars_count), star),
        ],
    );
    if let Some(reply) = row.reply_preview.data.first() {
        let width = out.content();
        out.line(rail, reply_line(reply, width, now));
    }
}

/// Images and polls are not in the API, so the feed only says they exist.
fn web_only(media: &Media, width: usize, out: &mut Vec<Vec<Span<'static>>>) {
    let mut parts = Vec::new();
    match media.images.len() {
        0 => {}
        1 => parts.push("1 image".to_owned()),
        count => parts.push(format!("{count} images")),
    }
    if media.poll.is_some() {
        parts.push("a poll".to_owned());
    }
    if !parts.is_empty() {
        let note = format!("[{} on the web]", parts.join(" and "));
        wrap(&[(note, theme::muted())], width, out);
    }
}

/// The web draws no card until the server has fetched the page's title.
fn link_preview(media: &Media, width: usize, out: &mut Vec<Vec<Span<'static>>>) {
    let Some(preview) = &media.link_preview else {
        return;
    };
    let Some(title) = &preview.title else {
        return;
    };
    wrap(
        &[
            (title.clone(), Style::new().fg(theme::FOREGROUND)),
            (format!(" · {}", domain(&preview.url)), theme::muted()),
        ],
        width,
        out,
    );
}

/// Mirrors the web's `hostname`, which drops a leading `www.`.
fn domain(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host)
}

const QUOTE_LINES: usize = 4;

fn quote<Tz: TimeZone>(
    out: &mut Writer,
    rail: &Span<'static>,
    quoted: Option<&Shown<Quote>>,
    now: &DateTime<Tz>,
) {
    let inner = out.content().saturating_sub(4).max(1);
    let mut lines = Vec::new();
    match quoted {
        Some(Shown::Live(quote)) => {
            let commit = &quote.commit;
            lines.push(header(commit, inner, now));
            let segments = commit
                .preview_segments
                .as_ref()
                .unwrap_or(&commit.body_segments);
            let mut text = body(segments, inner);
            if text.len() > QUOTE_LINES {
                text.truncate(QUOTE_LINES);
                text.push(vec![Span::styled("…", theme::muted())]);
            }
            lines.extend(text);
            web_only(&quote.media, inner, &mut lines);
            link_preview(&quote.media, inner, &mut lines);
        }
        Some(Shown::Deleted(tombstone)) => {
            for text in [
                bad_object(&tombstone.short_hash()),
                "This commit is no longer available.".to_owned(),
            ] {
                wrap(&[(text, theme::muted())], inner, &mut lines);
            }
        }
        None => wrap(
            &[("This commit isn't available.".to_owned(), theme::muted())],
            inner,
            &mut lines,
        ),
    }

    let border = Style::new().fg(theme::RAIL);
    let edge = "─".repeat(inner + 2);
    out.line(
        rail.clone(),
        vec![Span::styled(format!("╭{edge}╮"), border)],
    );
    for mut line in lines {
        let used: usize = line.iter().map(Span::width).sum();
        line.insert(0, Span::styled("│ ", border));
        line.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
        line.push(Span::styled(" │", border));
        out.line(rail.clone(), line);
    }
    out.line(
        rail.clone(),
        vec![Span::styled(format!("╰{edge}╯"), border)],
    );
}

fn bad_object(short_hash: &str) -> String {
    format!("fatal: bad object {short_hash}")
}

/// The newest reply, on one line under its commit, as the web previews it.
fn reply_line<Tz: TimeZone>(
    reply: &Shown<Reply>,
    width: usize,
    now: &DateTime<Tz>,
) -> Vec<Span<'static>> {
    let reply = match reply {
        Shown::Live(reply) => reply,
        Shown::Deleted(tombstone) => {
            return fit(
                vec![
                    Span::styled("  ○ ", Style::new().fg(theme::RAIL)),
                    Span::styled(bad_object(&tombstone.short_hash()), theme::muted()),
                ],
                width,
            );
        }
    };
    let commit = &reply.commit;
    let author = &commit.author;
    let mut spans = vec![
        Span::styled("  ● ", Style::new().fg(theme::lane(author.lane))),
        Span::styled(
            author.name.clone(),
            Style::new()
                .fg(theme::FOREGROUND)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " @{} · {}  ",
                author.username,
                relative_time(commit.created_at, now)
            ),
            theme::muted(),
        ),
    ];
    spans.extend(one_line(
        &commit.body_segments,
        Style::new().fg(theme::FOREGROUND),
    ));
    fit(spans, width)
}

/// A body on one line, the way the web's `truncate` shows it: runs of whitespace collapse to one space, and a
/// code block keeps only its first line.
fn one_line(segments: &[Segment], text: Style) -> Vec<Span<'static>> {
    let mut line = Vec::new();
    let mut gap = false;
    let mut add = |piece: &str, style: Style, gap: &mut bool| {
        if piece.is_empty() {
            return;
        }
        if mem::take(gap) && !line.is_empty() {
            append(&mut line, " ", text);
        }
        append(&mut line, piece, style);
    };
    for segment in segments {
        match segment {
            Segment::Text { text: body } => {
                for (index, word) in body.split(char::is_whitespace).enumerate() {
                    gap |= index > 0;
                    add(word, text, &mut gap);
                }
            }
            Segment::Code { code } => add(&code.replace('\t', TAB), theme::hl(None), &mut gap),
            Segment::Block { html, .. } => {
                gap = true;
                let lines = code::block_lines(html);
                let first = lines
                    .iter()
                    .find(|line| line.iter().any(|span| !span.text.trim().is_empty()));
                for span in first.into_iter().flatten() {
                    add(&span.text.replace('\t', TAB), theme::hl(span.hl), &mut gap);
                }
                gap = true;
            }
        }
    }
    line
}

fn header<Tz: TimeZone>(commit: &Commit, width: usize, now: &DateTime<Tz>) -> Vec<Span<'static>> {
    let author = &commit.author;
    let bold = Style::new()
        .fg(theme::FOREGROUND)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(author.name.clone(), bold),
        Span::styled(format!(" @{}", author.username), theme::muted()),
    ];
    spans.push(Span::styled(
        format!(" · {}", relative_time(commit.created_at, now)),
        theme::muted(),
    ));
    match commit.status {
        Status::Pending => spans.push(Span::styled("  in review", Style::new().fg(theme::AMBER))),
        Status::Rejected => spans.push(Span::styled(
            "  rejected",
            Style::new().fg(theme::DESTRUCTIVE),
        )),
        Status::Approved => {}
    }

    let used: usize = spans.iter().map(Span::width).sum();
    let hash = commit.short_hash.width();
    if used + 1 + hash <= width {
        spans.push(Span::raw(" ".repeat(width - used - hash)));
        spans.push(Span::styled(commit.short_hash.clone(), theme::muted()));
        return spans;
    }
    fit(spans, width)
}

fn fit(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut fitted = Vec::new();
    let mut room = width;
    for span in spans {
        let span_width = span.width();
        if span_width <= room {
            room -= span_width;
            fitted.push(span);
            continue;
        }
        fitted.push(Span::styled(truncate(&span.content, room), span.style));
        break;
    }
    fitted
}

/// Prose wraps at word boundaries and code blocks hard-wrap, both to `width`. A text segment's trailing newline
/// before a block ends the line rather than adding a blank one, as `white-space: pre-wrap` renders it on the web.
fn body(segments: &[Segment], width: usize) -> Vec<Vec<Span<'static>>> {
    let text = Style::new().fg(theme::FOREGROUND);
    let mut lines = Vec::new();
    let mut paragraph: Vec<(String, Style)> = Vec::new();
    for segment in segments {
        match segment {
            Segment::Text { text: body } => {
                for (index, piece) in body.split('\n').enumerate() {
                    if index > 0 {
                        wrap(&mem::take(&mut paragraph), width, &mut lines);
                    }
                    if !piece.is_empty() {
                        paragraph.push((piece.replace('\t', TAB), text));
                    }
                }
            }
            Segment::Code { code } => paragraph.push((code.replace('\t', TAB), theme::hl(None))),
            Segment::Block { html, .. } => {
                if !paragraph.is_empty() {
                    wrap(&mem::take(&mut paragraph), width, &mut lines);
                }
                block(html, width, &mut lines);
            }
        }
    }
    if !paragraph.is_empty() {
        wrap(&paragraph, width, &mut lines);
    }
    lines
}

fn wrap(paragraph: &[(String, Style)], width: usize, out: &mut Vec<Vec<Span<'static>>>) {
    let mut line = Vec::new();
    let mut used = 0;
    for (text, style) in paragraph {
        for run in runs(text) {
            let run_width = run.width();
            let blank = run.starts_with(char::is_whitespace);
            if used > 0 && used + run_width > width {
                out.push(mem::take(&mut line));
                used = 0;
                if blank {
                    continue;
                }
            }
            if run_width <= width {
                append(&mut line, run, *style);
                used += run_width;
                continue;
            }
            for grapheme in run.graphemes(true) {
                let grapheme_width = grapheme.width();
                if used + grapheme_width > width {
                    out.push(mem::take(&mut line));
                    used = 0;
                }
                append(&mut line, grapheme, *style);
                used += grapheme_width;
            }
        }
    }
    out.push(line);
}

fn runs(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let blank = rest.chars().next()?.is_whitespace();
        let end = rest
            .find(|ch: char| ch.is_whitespace() != blank)
            .unwrap_or(rest.len());
        let (run, after) = rest.split_at(end);
        rest = after;
        Some(run)
    })
}

fn append(line: &mut Vec<Span<'static>>, text: &str, style: Style) {
    match line.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(text),
        _ => line.push(Span::styled(text.to_owned(), style)),
    }
}

/// A code block fills the content width with the muted background, one column of padding on each side and a
/// blank row above and below, so it reads as a box.
fn block(html: &str, width: usize, out: &mut Vec<Vec<Span<'static>>>) {
    let inner = width.saturating_sub(2).max(1);
    let row = |mut spans: Vec<Span<'static>>, used: usize| {
        spans.insert(0, Span::styled(" ", theme::hl(None)));
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used + 1)),
            theme::hl(None),
        ));
        spans
    };
    out.push(row(Vec::new(), 0));
    for line in code::block_lines(html) {
        let mut spans = Vec::new();
        let mut used = 0;
        for span in &line {
            let style = theme::hl(span.hl);
            for grapheme in span.text.replace('\t', TAB).graphemes(true) {
                let grapheme_width = grapheme.width();
                if used + grapheme_width > inner {
                    out.push(row(mem::take(&mut spans), used));
                    used = 0;
                }
                append(&mut spans, grapheme, style);
                used += grapheme_width;
            }
        }
        out.push(row(spans, used));
    }
    out.push(row(Vec::new(), 0));
}

fn empty(out: &mut Writer, page: &LogPage) {
    let mut lines = Vec::new();
    let (command, message) = match page.log {
        Log::Following => (
            "fatal: your log does not have any commits yet",
            if page.suggestions.is_empty() {
                "Your log shows your commits and commits from people you follow. Find someone worth following on Trending."
            } else {
                "Your log shows your commits and commits from people you follow. Here is what a few developers committed lately:"
            },
        ),
        Log::Latest | Log::Trending => (
            "fatal: no commits yet",
            "Nobody has committed anything. Be the first. It's just plain text; you'll be fine.",
        ),
    };
    wrap(
        &[(command.to_owned(), theme::muted())],
        out.width,
        &mut lines,
    );
    lines.push(Vec::new());
    wrap(
        &[(message.to_owned(), Style::new().fg(theme::FOREGROUND))],
        out.width,
        &mut lines,
    );

    for suggestion in &page.suggestions {
        let commit = &suggestion.commit;
        let author = &commit.author;
        lines.push(Vec::new());
        lines.push(fit(
            vec![
                Span::styled("● ", Style::new().fg(theme::lane(author.lane))),
                Span::styled(
                    author.name.clone(),
                    Style::new()
                        .fg(theme::FOREGROUND)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" @{}", author.username), theme::muted()),
            ],
            out.width,
        ));
        // The web shows the body in one `truncate` line, where nowrap collapses newlines and runs of spaces.
        let body = commit.body.split_whitespace().collect::<Vec<_>>().join(" ");
        lines.push(vec![
            Span::raw("  "),
            Span::styled(truncate(&body, out.width - RAIL_WIDTH), theme::muted()),
        ]);
    }
    out.lines.extend(lines.into_iter().map(Line::from));
}

fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width + 1 > width {
            break;
        }
        out.push_str(grapheme);
        used += grapheme_width;
    }
    out.push('…');
    out
}

/// Mirrors the web's `formatRelativeTime`.
pub fn relative_time<Tz: TimeZone>(at: DateTime<Utc>, now: &DateTime<Tz>) -> String {
    let seconds = now.timestamp() - at.timestamp();
    match seconds {
        ..60 => "now".to_owned(),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        86_400..604_800 => format!("{}d", seconds / 86_400),
        _ => {
            let day = at.with_timezone(&now.timezone()).date_naive();
            if day.year() == now.year() {
                day.format("%b %-d").to_string()
            } else {
                day.format("%b %-d, %Y").to_string()
            }
        }
    }
}

/// Mirrors the web's `formatDayLabel`.
pub fn day_label<Tz: TimeZone>(day: NaiveDate, now: &DateTime<Tz>) -> String {
    match (now.date_naive() - day).num_days() {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ if day.year() == now.year() => day.format("%a, %b %-d").to_string(),
        _ => day.format("%a, %b %-d, %Y").to_string(),
    }
}
