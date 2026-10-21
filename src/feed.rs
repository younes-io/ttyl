//! Lays a log page out as terminal lines, the way the web's commit feed draws it: one node per commit on a
//! graph rail, day markers on chronological logs, and a root node once the log has no older page.

use std::collections::HashSet;
use std::mem;
use std::ops::Range;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::code;
use crate::model::{Commit, CommitId, Entry, Log, LogPage, Segment, Status};
use crate::theme;

const RAIL_WIDTH: usize = 2;
const TAB: &str = "    ";

pub struct Feed {
    pub lines: Vec<Line<'static>>,
    /// The lines each of the page's entries occupies, in page order.
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
    if page.entries.is_empty() {
        empty(&mut out, page);
        return Feed {
            lines: out.lines,
            entries: Vec::new(),
        };
    }

    let mut entries = Vec::with_capacity(page.entries.len());
    let mut last_day = None;
    for (index, entry) in page.entries.iter().enumerate() {
        if index > 0 {
            out.gap();
        }
        if let (false, Entry::Commit(commit)) = (page.is_ranked, entry) {
            let day = commit
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
        match entry {
            Entry::Commit(commit) => commit_lines(
                &mut out,
                commit,
                selected == Some(index),
                expanded.contains(&commit.id),
                now,
            ),
            Entry::Tombstone(_) => out.push(
                Span::styled("○ ", Style::new().fg(theme::RAIL)),
                vec![Span::styled(
                    "[deleted] This commit was deleted.",
                    theme::muted(),
                )],
            ),
        }
        entries.push(start..out.lines.len());
    }

    let last_lane = page.entries.iter().rev().find_map(|entry| match entry {
        Entry::Commit(commit) => Some(commit.author.lane),
        Entry::Tombstone(_) => None,
    });
    if let (None, Some(lane)) = (&page.next_cursor, last_lane) {
        out.gap();
        out.push(
            Span::styled("○ ", Style::new().fg(theme::lane(lane))),
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

    fn push(&mut self, rail: Span<'static>, mut spans: Vec<Span<'static>>) {
        spans.insert(0, rail);
        self.lines.push(Line::from(spans));
    }

    fn gap(&mut self) {
        self.push(Span::styled("│", Style::new().fg(theme::RAIL)), Vec::new());
    }
}

fn commit_lines<Tz: TimeZone>(
    out: &mut Writer,
    commit: &Commit,
    selected: bool,
    expanded: bool,
    now: &DateTime<Tz>,
) {
    let lane = Style::new().fg(theme::lane(commit.author.lane));
    let rail = if selected {
        Span::styled("┃ ", lane)
    } else {
        Span::styled("│ ", Style::new().fg(theme::RAIL))
    };
    let header = header(commit, out.content(), now);
    out.push(Span::styled("● ", lane), header);

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

    let mut footer = Vec::new();
    if let Some(replies) = commit.replies_count {
        footer.push(Span::styled(format!("↩ {replies}"), theme::muted()));
    }
    if let Some(stars) = commit.stars_count {
        if !footer.is_empty() {
            footer.push(Span::raw("   "));
        }
        let style = if commit.starred == Some(true) {
            Style::new().fg(theme::AMBER)
        } else {
            theme::muted()
        };
        footer.push(Span::styled(format!("★ {stars}"), style));
    }
    if !footer.is_empty() {
        out.push(rail, footer);
    }
}

fn header<Tz: TimeZone>(commit: &Commit, width: usize, now: &DateTime<Tz>) -> Vec<Span<'static>> {
    let author = &commit.author;
    let bold = Style::new()
        .fg(theme::FOREGROUND)
        .add_modifier(Modifier::BOLD);
    let mut spans = match &author.name {
        Some(name) => vec![
            Span::styled(name.clone(), bold),
            Span::styled(format!(" @{}", author.username), theme::muted()),
        ],
        None => vec![Span::styled(format!("@{}", author.username), bold)],
    };
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
            for ch in run.chars() {
                let ch_width = ch.width().unwrap_or(0);
                if used + ch_width > width {
                    out.push(mem::take(&mut line));
                    used = 0;
                }
                append(&mut line, ch.encode_utf8(&mut [0; 4]), *style);
                used += ch_width;
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
            for ch in span.text.replace('\t', TAB).chars() {
                let ch_width = ch.width().unwrap_or(0);
                if used + ch_width > inner {
                    out.push(row(mem::take(&mut spans), used));
                    used = 0;
                }
                append(&mut spans, ch.encode_utf8(&mut [0; 4]), style);
                used += ch_width;
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
    lines.push(vec![Span::styled(command, theme::muted())]);
    lines.push(Vec::new());
    wrap(
        &[(message.to_owned(), Style::new().fg(theme::FOREGROUND))],
        out.width,
        &mut lines,
    );

    for entry in &page.suggestions {
        let Entry::Commit(commit) = entry else {
            continue;
        };
        let author = &commit.author;
        lines.push(Vec::new());
        lines.push(vec![
            Span::styled("● ", Style::new().fg(theme::lane(author.lane))),
            Span::styled(
                author
                    .name
                    .clone()
                    .unwrap_or_else(|| author.username.clone()),
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" @{}", author.username), theme::muted()),
        ]);
        let first = commit.body.lines().next().unwrap_or_default();
        lines.push(vec![
            Span::raw("  "),
            Span::styled(truncate(first, out.width - RAIL_WIDTH), theme::muted()),
        ]);
    }
    out.lines.extend(lines.into_iter().map(Line::from));
}

fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        out.push(ch);
        used += ch_width;
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
