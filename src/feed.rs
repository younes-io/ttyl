//! Lays a log page out as terminal lines, the way the web's commit feed draws it: one node per commit on a
//! graph rail, day markers on chronological logs, and a root node once the log has no older page.

use std::collections::HashSet;
use std::mem;
use std::ops::Range;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::bidi;
use crate::code;
use crate::model::{
    Commit, CommitId, Deletion, Lane, Log, LogPage, Media, Quote, Reply, Row, Segment, Shown,
    Status,
};
use crate::theme;

const RAIL_WIDTH: usize = 2;
const TAB: &str = "    ";

pub struct Feed {
    pub lines: Vec<Line<'static>>,
    pub entries: Vec<Entry>,
}

pub struct Entry {
    pub lines: Range<usize>,
    pub header: usize,
    pub lane: Lane,
    pub indent: usize,
}

impl Feed {
    /// The lines in `window`, with the selected commit lifted onto a card `width` wide, tinted in its lane
    /// color with the gap above and below as padding, and its rail drawn in that color.
    pub fn window(
        &self,
        selected: Option<usize>,
        window: Range<usize>,
        width: u16,
    ) -> Vec<Line<'static>> {
        let mut lines = self.lines[window.clone()].to_vec();
        let Some(entry) = selected.and_then(|at| self.entries.get(at)) else {
            return lines;
        };
        let above = entry.header.checked_sub(1);
        let below = (entry.lines.end < self.lines.len()).then_some(entry.lines.end);
        let card = above
            .into_iter()
            .chain(entry.header..entry.lines.end)
            .chain(below);
        for at in card.filter(|at| window.contains(at)) {
            let line = &mut lines[at - window.start];
            let fill = usize::from(width).saturating_sub(line.width());
            line.spans.push(Span::raw(" ".repeat(fill)));
            line.style = line.style.bg(theme::tint(entry.lane));
        }
        if entry.header + 1 == entry.lines.end {
            if window.contains(&entry.header) {
                for span in &mut lines[entry.header - window.start].spans {
                    span.style = span.style.fg(theme::FOREGROUND);
                }
            }
            return lines;
        }
        let rail = Span::styled("┃ ", Style::new().fg(theme::lane(entry.lane)));
        let at_rail = usize::from(entry.indent > 0);
        for at in (entry.header + 1..entry.lines.end).filter(|at| window.contains(at)) {
            lines[at - window.start].spans[at_rail] = rail.clone();
        }
        lines
    }
}

pub fn older_failed(what: &str, retry: Duration) -> [Line<'static>; 2] {
    [
        Line::from(Span::styled("│", Style::new().fg(theme::RAIL))),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("fatal: could not load older {what}"),
                Style::new().fg(theme::DESTRUCTIVE),
            ),
            Span::styled(
                format!(" · retrying in {}s", retry.as_secs()),
                theme::muted(),
            ),
        ]),
    ]
}

/// Lays the page out one dense row per commit, and says which rows were cut short. With `inline_replies`
/// the newest reply sits under its commit, for terminals too narrow for the context pane.
pub fn layout<Tz: TimeZone>(
    page: &LogPage,
    width: u16,
    expanded: &HashSet<CommitId>,
    inline_replies: bool,
    now: &DateTime<Tz>,
) -> (Feed, Vec<bool>) {
    let mut out = Writer::new(width);
    if page.rows.is_empty() {
        empty(&mut out, page);
        let feed = Feed {
            lines: out.lines,
            entries: Vec::new(),
        };
        return (feed, Vec::new());
    }

    let mut entries = Vec::with_capacity(page.rows.len());
    let mut folded = Vec::with_capacity(page.rows.len());
    let mut last_day = None;
    for (index, row) in page.rows.iter().enumerate() {
        if index > 0 {
            out.gap();
        }
        let start = out.lines.len();
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
        let header = out.lines.len();
        let open = expanded.contains(&row.commit.id);
        folded.push(row_lines(&mut out, row, open, inline_replies, now));
        entries.push(Entry {
            lines: start..out.lines.len(),
            header,
            lane: row.commit.author.lane,
            indent: 0,
        });
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

    let feed = Feed {
        lines: out.lines,
        entries,
    };
    (feed, folded)
}

pub(crate) struct Writer {
    pub lines: Vec<Line<'static>>,
    width: usize,
    pub pad: usize,
}

impl Writer {
    pub fn new(width: u16) -> Self {
        Writer {
            lines: Vec::new(),
            width: usize::from(width).max(RAIL_WIDTH + 8),
            pad: 0,
        }
    }

    pub fn content(&self) -> usize {
        self.width.saturating_sub(self.pad + RAIL_WIDTH).max(8)
    }

    pub fn push(&mut self, rail: Span<'static>, spans: Vec<Span<'static>>) {
        self.node(rail.clone(), rail, spans);
    }

    /// Wraps `spans` to the content width, with `node` on the first line and `rail` on the rest.
    pub fn node(&mut self, node: Span<'static>, rail: Span<'static>, spans: Vec<Span<'static>>) {
        let pieces: Vec<_> = spans
            .into_iter()
            .map(|span| (span.content.into_owned(), span.style))
            .collect();
        let mut lines = Vec::new();
        wrap(&pieces, self.content(), &mut lines);
        for (index, line) in lines.into_iter().enumerate() {
            let rail = if index == 0 { &node } else { &rail };
            self.line(rail.clone(), line);
        }
    }

    pub fn line(&mut self, rail: Span<'static>, spans: Vec<Span<'static>>) {
        let mut spans = bidi::visual(spans);
        spans.insert(0, rail);
        if self.pad > 0 {
            spans.insert(0, Span::raw(" ".repeat(self.pad)));
        }
        self.lines.push(Line::from(spans));
    }

    pub fn gap(&mut self) {
        self.push(Span::styled("│", Style::new().fg(theme::RAIL)), Vec::new());
    }
}

pub(crate) fn rail() -> Span<'static> {
    Span::styled("│ ", Style::new().fg(theme::RAIL))
}

const BODY_LINES: usize = 4;

/// One commit as the feed shows it: a header carrying the counts, the body cut to `BODY_LINES` unless
/// `expanded`, and the quote, media and link each on one line. Returns whether the body was cut.
fn row_lines<Tz: TimeZone>(
    out: &mut Writer,
    row: &Row,
    expanded: bool,
    inline_replies: bool,
    now: &DateTime<Tz>,
) -> bool {
    let commit = &row.commit;
    if row.deletion == Some(Deletion::Done) {
        out.node(
            Span::styled("○ ", Style::new().fg(theme::RAIL)),
            rail(),
            vec![Span::styled(bad_object(&commit.short_hash), theme::muted())],
        );
        return false;
    }
    let lane = Style::new().fg(theme::lane(commit.author.lane));
    let rail = rail();
    let header = header_with(commit, out.content(), now, counts(row));
    out.node(Span::styled("● ", lane), rail.clone(), header);

    let mut text = body(&commit.body_segments, out.content());
    let cut = text.len().saturating_sub(BODY_LINES);
    let folded = !expanded && cut > 1;
    if folded {
        text.truncate(BODY_LINES);
    }
    for line in text {
        out.push(rail.clone(), line);
    }
    if folded {
        out.push(
            rail.clone(),
            vec![Span::styled(
                format!("+{cut} lines · space to expand"),
                theme::muted(),
            )],
        );
    }
    let width = out.content();
    let mut lines = Vec::new();
    web_only(&row.media, width, &mut lines);
    if commit.quoted_commit_id.is_some() {
        lines.push(quote_line(row.quoted_commit.as_ref(), width));
    }
    if let Some(preview) = &row.media.link_preview
        && let Some(title) = &preview.title
    {
        lines.push(fit(
            vec![
                Span::styled("⇗ ", theme::muted()),
                Span::styled(title.clone(), Style::new().fg(theme::PRIMARY)),
                Span::styled(format!(" · {}", domain(&preview.url)), theme::muted()),
            ],
            width,
        ));
    }
    if inline_replies && let Some(reply) = row.reply_preview.data.first() {
        lines.push(reply_line(reply, width, now));
    }
    for line in lines {
        out.line(rail.clone(), line);
    }
    folded
}

/// The header's tail: replies and stars when there are any, or that the commit is being deleted.
fn counts(row: &Row) -> Vec<Span<'static>> {
    if row.deletion == Some(Deletion::Running) {
        return vec![Span::styled(" · deleting…", theme::muted())];
    }
    let mut spans = Vec::new();
    if row.replies_count > 0 {
        spans.push(Span::styled(" · ", theme::muted()));
        spans.push(Span::styled(
            format!("↩ {}", row.replies_count),
            Style::new().fg(theme::PRIMARY),
        ));
    }
    if row.stars_count > 0 || row.starred {
        let star = if row.starred {
            Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD)
        } else {
            theme::muted()
        };
        let lead = if spans.is_empty() { " · " } else { "  " };
        spans.push(Span::styled(lead, theme::muted()));
        spans.push(Span::styled(format!("★ {}", row.stars_count), star));
    }
    spans
}

fn quote_line(quoted: Option<&Shown<Quote>>, width: usize) -> Vec<Span<'static>> {
    let mark = Span::styled("❝ ", theme::muted());
    match quoted {
        Some(Shown::Live(quote)) => one_liner("❝ ", &quote.commit, "", theme::muted(), width),
        Some(Shown::Deleted(tombstone)) => fit(
            vec![
                mark,
                Span::styled(bad_object(&tombstone.short_hash()), theme::muted()),
            ],
            width,
        ),
        None => vec![
            mark,
            Span::styled("This commit isn't available.", theme::muted()),
        ],
    }
}

pub(crate) fn content<Tz: TimeZone>(
    out: &mut Writer,
    rail: &Span<'static>,
    row: &Row,
    fold: bool,
    quoted: bool,
    now: &DateTime<Tz>,
) {
    let commit = &row.commit;
    let folded = commit.preview_segments.as_ref().filter(|_| fold);
    for line in body(folded.unwrap_or(&commit.body_segments), out.content()) {
        out.push(rail.clone(), line);
    }
    if folded.is_some() {
        out.push(
            rail.clone(),
            vec![Span::styled("show more", Style::new().fg(theme::PRIMARY))],
        );
    }
    let mut lines = Vec::new();
    web_only(&row.media, out.content(), &mut lines);
    for line in lines {
        out.line(rail.clone(), line);
    }
    if quoted {
        quote(out, rail, row.quoted_commit.as_ref(), now);
    }
    let mut lines = Vec::new();
    link_preview(&row.media, out.content(), &mut lines);
    for line in lines {
        out.line(rail.clone(), line);
    }
}

pub(crate) fn stats(out: &mut Writer, rail: &Span<'static>, row: &Row) {
    if row.deletion == Some(Deletion::Running) {
        out.push(
            rail.clone(),
            vec![Span::styled("deleting…", theme::muted())],
        );
        return;
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
}

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
        let note = format!("[{} · O to open in the browser]", parts.join(" and "));
        wrap(&[(note, theme::muted())], width, out);
    }
}

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
            lines.push(header(commit, inner, now, None));
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

pub(crate) fn bad_object(short_hash: &str) -> String {
    format!("fatal: bad object {short_hash}")
}

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
    let meta = format!(" · {}", relative_time(commit.created_at, now));
    one_liner(
        "  ● ",
        commit,
        &meta,
        Style::new().fg(theme::FOREGROUND),
        width,
    )
}

fn one_liner(
    node: &'static str,
    commit: &Commit,
    meta: &str,
    text: Style,
    width: usize,
) -> Vec<Span<'static>> {
    let author = &commit.author;
    let mut spans = vec![
        Span::styled(node, Style::new().fg(theme::lane(author.lane))),
        Span::styled(
            author.name.clone(),
            Style::new()
                .fg(theme::lane(author.lane))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" @{}{meta}  ", author.username), theme::muted()),
    ];
    spans.extend(one_line(&commit.body_segments, text));
    fit(spans, width)
}

pub(crate) fn one_line(segments: &[Segment], text: Style) -> Vec<Span<'static>> {
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

pub(crate) fn header<Tz: TimeZone>(
    commit: &Commit,
    width: usize,
    now: &DateTime<Tz>,
    mark: Option<&str>,
) -> Vec<Span<'static>> {
    let tail = mark
        .map(|mark| Span::styled(format!(" · {mark}"), theme::muted()))
        .into_iter()
        .collect();
    header_with(commit, width, now, tail)
}

fn header_with<Tz: TimeZone>(
    commit: &Commit,
    width: usize,
    now: &DateTime<Tz>,
    tail: Vec<Span<'static>>,
) -> Vec<Span<'static>> {
    let author = &commit.author;
    let bold = Style::new()
        .fg(theme::lane(author.lane))
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(author.name.clone(), bold),
        Span::styled(format!(" @{}", author.username), theme::muted()),
    ];
    spans.push(Span::styled(
        format!(" · {}", relative_time(commit.created_at, now)),
        theme::muted(),
    ));
    spans.extend(tail);
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
        spans.push(Span::styled(
            commit.short_hash.clone(),
            Style::new().fg(theme::FAINT),
        ));
        return spans;
    }
    fit(spans, width)
}

pub(crate) fn fit(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    if spans.iter().map(Span::width).sum::<usize>() <= width {
        return spans;
    }
    if width == 0 {
        return Vec::new();
    }
    let mut fitted = Vec::new();
    let mut room = width - 1;
    for span in spans {
        let span_width = span.width();
        if span_width <= room {
            room -= span_width;
            fitted.push(span);
            continue;
        }
        let mut kept = String::new();
        for grapheme in span.content.graphemes(true) {
            let grapheme_width = grapheme.width();
            if grapheme_width > room {
                break;
            }
            room -= grapheme_width;
            kept.push_str(grapheme);
        }
        kept.push('…');
        fitted.push(Span::styled(kept, span.style));
        break;
    }
    fitted
}

/// Prose wraps at word boundaries and code blocks hard-wrap, both to `width`. A text segment's trailing newline
/// before a block ends the line rather than adding a blank one, as `white-space: pre-wrap` renders it on the web.
pub(crate) fn body(segments: &[Segment], width: usize) -> Vec<Vec<Span<'static>>> {
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
                    for run in runs(&piece.replace('\t', TAB)) {
                        let style = if is_reference(run) {
                            Style::new().fg(theme::PRIMARY)
                        } else {
                            text
                        };
                        paragraph.push((run.to_owned(), style));
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

pub(crate) fn wrap(paragraph: &[(String, Style)], width: usize, out: &mut Vec<Vec<Span<'static>>>) {
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

/// A mention or a link, which the web draws as a link.
fn is_reference(run: &str) -> bool {
    let mention = run
        .strip_prefix('@')
        .is_some_and(|name| name.starts_with(|ch: char| ch.is_alphanumeric()));
    mention || run.starts_with("https://") || run.starts_with("http://")
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
    let text = |text: &str| (text.to_owned(), Style::new().fg(theme::FOREGROUND));
    let trending = || {
        (
            "Trending".to_owned(),
            Style::new()
                .fg(theme::PRIMARY)
                .add_modifier(Modifier::UNDERLINED),
        )
    };
    let (command, message) = match page.log {
        Log::Following if page.suggestions.is_empty() => (
            "fatal: your log does not have any commits yet",
            vec![
                text(
                    "Your log shows your commits and commits from people you follow. Find someone worth following on ",
                ),
                trending(),
                text("."),
            ],
        ),
        Log::Following => (
            "fatal: your log does not have any commits yet",
            vec![text(
                "Your log shows your commits and commits from people you follow. Here is what a few developers committed lately:",
            )],
        ),
        Log::Latest | Log::Trending => (
            "fatal: no commits yet",
            vec![text(
                "Nobody has committed anything. Be the first. It's just plain text; you'll be fine.",
            )],
        ),
    };
    let mut lines = Vec::new();
    wrap(
        &[(command.to_owned(), theme::muted())],
        out.width,
        &mut lines,
    );
    lines.push(Vec::new());
    wrap(&message, out.width, &mut lines);

    if !page.suggestions.is_empty() {
        lines.push(Vec::new());
        for suggestion in &page.suggestions {
            lines.push(one_liner(
                "● ",
                &suggestion.commit,
                "",
                theme::muted(),
                out.width,
            ));
        }
        lines.push(Vec::new());
        let muted = |text: &str| (text.to_owned(), theme::muted());
        wrap(
            &[muted("Or browse "), trending(), muted(" for more.")],
            out.width,
            &mut lines,
        );
    }
    out.lines.extend(lines.into_iter().map(Line::from));
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

pub(crate) fn scroll(
    feed: &Feed,
    total: usize,
    selected: usize,
    offset: usize,
    height: usize,
) -> usize {
    let Some(entry) = feed.entries.get(selected) else {
        return 0;
    };
    let is_last = selected + 1 == feed.entries.len();
    let start = entry.lines.start;
    let end = if is_last { total } else { entry.lines.end };
    let offset = offset.min(total.saturating_sub(height));
    let offset = if start < offset {
        start
    } else if end > offset + height {
        (end - height).min(start)
    } else {
        offset
    };
    offset.max((entry.header + 1).saturating_sub(height))
}
