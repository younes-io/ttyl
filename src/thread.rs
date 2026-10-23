use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::api::Call;
use crate::clock::{Timer, Timers};
use crate::engine::AHEAD;
use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::Action;
use crate::model::{CommitId, Lane, Row};
use crate::sched::Priority;
use crate::store::{Branch, Chain, Node, Store};
use crate::theme;

pub const LOADING_NOTE_DELAY: Duration = Duration::from_millis(300);
const MAX_DEPTH: usize = 3;
const INDENT: usize = 2;
const ANCESTOR_LINES: usize = 2;

pub struct ThreadScreen {
    focus: CommitId,
    selected: CommitId,
    at: usize,
    offset: usize,
    anchor: Option<Anchor>,
    moved: bool,
    expanded: HashSet<CommitId>,
    opened_at: Instant,
    near: Vec<CommitId>,
    laid_out: Option<LaidOut>,
    peek: bool,
    /// Where the selected card was last drawn, for a box about it to open beside.
    card: Option<Rect>,
}

struct Anchor {
    id: CommitId,
    line: usize,
}

struct LaidOut {
    inputs: Inputs,
    feed: Feed,
}

#[derive(PartialEq)]
struct Inputs {
    width: u16,
    generation: u64,
    expanded: HashSet<CommitId>,
    second: i64,
    pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Earlier(CommitId),
    Ancestor(CommitId),
    Focus(CommitId),
    Reply {
        id: CommitId,
        depth: usize,
        parent: CommitId,
    },
    Note {
        owner: CommitId,
        depth: usize,
        note: Note,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Note {
    Count(u32),
    Loading,
    Failed(Option<u64>),
    Reload(Option<u64>),
    More(u32),
    MoreFailed(Option<u64>),
}

impl Piece {
    fn id(&self) -> Option<&CommitId> {
        match self {
            Piece::Earlier(id)
            | Piece::Ancestor(id)
            | Piece::Focus(id)
            | Piece::Reply { id, .. } => Some(id),
            Piece::Note { .. } => None,
        }
    }

    fn indent(&self) -> usize {
        match self {
            Piece::Reply { depth, .. } | Piece::Note { depth, .. } => {
                INDENT * (*depth).min(MAX_DEPTH)
            }
            Piece::Earlier(_) | Piece::Ancestor(_) | Piece::Focus(_) => 0,
        }
    }
}

struct View<'a> {
    loading_note_due: bool,
    timers: Option<(&'a Timers, Instant)>,
}

impl View<'_> {
    fn countdown(&self, timer: Timer) -> Option<u64> {
        let (timers, now) = self.timers?;
        timers
            .remaining(&timer, now)
            .map(|left| left.as_secs_f64().ceil() as u64)
    }
}

const BARE: View<'static> = View {
    loading_note_due: false,
    timers: None,
};

impl ThreadScreen {
    pub fn new(focus: CommitId, now: Instant) -> Self {
        ThreadScreen {
            selected: focus.clone(),
            focus,
            at: 0,
            offset: 0,
            anchor: None,
            moved: false,
            expanded: HashSet::new(),
            opened_at: now,
            near: Vec::new(),
            laid_out: None,
            peek: false,
            card: None,
        }
    }

    /// A thread shown beside the feed, which marks no reply as selected.
    pub fn peek(focus: CommitId, now: Instant) -> Self {
        ThreadScreen {
            peek: true,
            ..ThreadScreen::new(focus, now)
        }
    }

    /// Where the selected card was last drawn.
    pub fn card(&self) -> Option<Rect> {
        self.card
    }

    pub fn opened_at(&self) -> Instant {
        self.opened_at
    }

    pub fn focus(&self) -> &CommitId {
        &self.focus
    }

    pub fn selected(&self) -> &CommitId {
        &self.selected
    }

    fn pieces(&self, store: &Store, view: &View) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let chain = store.thread(&self.focus).map(|thread| &thread.chain);
        if let Some(Chain::Known { ancestors, earlier }) = chain {
            pieces.extend(earlier.iter().cloned().map(Piece::Earlier));
            pieces.extend(ancestors.iter().cloned().map(Piece::Ancestor));
        }
        pieces.push(Piece::Focus(self.focus.clone()));
        let missing = matches!(chain, Some(Chain::Missing));
        let count = store.node(&self.focus).map(|node| node.replies_count());
        match store.replies(&self.focus) {
            None if view.loading_note_due && !missing && count != Some(0) => {
                pieces.push(Piece::Note {
                    owner: self.focus.clone(),
                    depth: 1,
                    note: Note::Loading,
                })
            }
            None => {}
            Some(_) => {
                let mut seen = HashSet::from([self.focus.clone()]);
                walk(store, &self.focus, 1, view, &mut seen, &mut pieces);
            }
        }
        pieces
    }

    fn index(&self, ids: &[&CommitId]) -> usize {
        ids.iter()
            .position(|id| **id == self.selected)
            .unwrap_or_else(|| self.at.min(ids.len().saturating_sub(1)))
    }

    pub fn can(&self, action: Action, store: &Store) -> bool {
        match action {
            Action::Down
            | Action::Up
            | Action::Top
            | Action::Bottom
            | Action::Back
            | Action::Show(_)
            | Action::NextLog
            | Action::Refresh => true,
            Action::Open => self.opens(store).is_some(),
            Action::Expand => {
                let pieces = self.pieces(store, &BARE);
                pieces
                    .iter()
                    .any(|piece| matches!(piece, Piece::Reply { id, .. } if *id == self.selected))
                    && store
                        .row(&self.selected)
                        .is_some_and(|row| row.commit.preview_segments.is_some())
            }
            Action::Quit
            | Action::Help
            | Action::Saver
            | Action::Close
            | Action::SignIn
            | Action::StartOver
            | Action::Confirm
            | Action::Pick(_)
            | Action::Choose(_)
            | Action::Next
            | Action::Previous
            | Action::Commit(_)
            | Action::Profile
            | Action::Compose
            | Action::Send
            | Action::Editor
            | Action::Poll
            | Action::Attach
            | Action::Site
            | Action::Link(_)
            | Action::Inbox
            | Action::NextFilter
            | Action::PreviousFilter => false,
        }
    }

    pub fn opens(&self, store: &Store) -> Option<CommitId> {
        self.pieces(store, &BARE)
            .into_iter()
            .find(|piece| piece.id() == Some(&self.selected))
            .filter(|piece| !matches!(piece, Piece::Focus(_)))
            .and_then(|piece| piece.id().cloned())
    }

    pub fn act(&mut self, action: Action, store: &Store) {
        if action == Action::Expand {
            if !self.expanded.remove(&self.selected) {
                self.expanded.insert(self.selected.clone());
            }
            return;
        }
        let pieces = self.pieces(store, &BARE);
        let ids: Vec<&CommitId> = pieces.iter().filter_map(Piece::id).collect();
        let last = ids.len() - 1;
        let at = self.index(&ids);
        let to = match action {
            Action::Down => (at + 1).min(last),
            Action::Up => at.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => return,
        };
        self.selected = ids[to].clone();
        self.at = to;
        self.moved = true;
    }

    pub fn lists(&self, store: &Store) -> Vec<CommitId> {
        let mut lists = vec![self.focus.clone()];
        for piece in self.pieces(store, &BARE) {
            if let Piece::Reply { id, .. } = piece
                && store.replies(&id).is_some()
            {
                lists.push(id);
            }
        }
        lists
    }

    pub fn wants(&self, store: &Store) -> Vec<(Call, Priority, Timer)> {
        let mut wants = Vec::new();
        let thread = store.thread(&self.focus);
        if thread.is_none_or(|thread| thread.failure.is_some()) {
            wants.push((
                Call::Commit(self.focus.clone()),
                Priority::Later,
                Timer::Commit(self.focus.clone()),
            ));
        }
        for owner in std::iter::once(&self.focus).chain(&self.near) {
            let stale = match store.replies(owner) {
                None | Some(Branch::Failed(_)) => true,
                Some(Branch::Listed(replies)) => replies.reload.is_some(),
            };
            if stale {
                wants.push((
                    Call::Replies {
                        id: owner.clone(),
                        cursor: None,
                    },
                    Priority::Later,
                    Timer::Replies(owner.clone()),
                ));
            }
        }
        for owner in self.lists(store) {
            if let Some(cursor) = store.refilling_replies(&owner) {
                wants.push((
                    Call::Replies {
                        id: owner.clone(),
                        cursor: Some(cursor.to_owned()),
                    },
                    Priority::Later,
                    Timer::More(owner),
                ));
            }
        }
        let pieces = self.pieces(store, &BARE);
        let ids: Vec<&CommitId> = pieces.iter().filter_map(Piece::id).collect();
        let at = self.index(&ids);
        let mut tail: usize = 0;
        for piece in &pieces {
            match piece {
                Piece::Note {
                    owner,
                    note: Note::More(_) | Note::MoreFailed(_),
                    ..
                } if tail.abs_diff(at) <= AHEAD => {
                    if let Some(Branch::Listed(replies)) = store.replies(owner)
                        && let Some(cursor) = &replies.next_cursor
                    {
                        wants.push((
                            Call::Replies {
                                id: owner.clone(),
                                cursor: Some(cursor.clone()),
                            },
                            Priority::Now,
                            Timer::More(owner.clone()),
                        ));
                    }
                }
                Piece::Note { .. } => {}
                _ => {
                    tail = ids
                        .iter()
                        .position(|id| Some(*id) == piece.id())
                        .unwrap_or(tail)
                }
            }
        }
        wants
    }

    pub fn placeholder_at(&self, store: &Store) -> Option<Instant> {
        store
            .replies(&self.focus)
            .is_none()
            .then_some(self.opened_at + LOADING_NOTE_DELAY)
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        timers: &Timers,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let view = View {
            loading_note_due: now >= self.opened_at + LOADING_NOTE_DELAY,
            timers: Some((timers, now)),
        };
        let pieces = self.pieces(store, &view);
        let inputs = Inputs {
            width: area.width,
            generation: store.generation(),
            expanded: self.expanded.clone(),
            second: wall.timestamp(),
            pieces,
        };
        let laid_out = match self.laid_out.take() {
            Some(laid_out) if laid_out.inputs == inputs => laid_out,
            _ => {
                let feed = layout(&inputs.pieces, store, area.width, &self.expanded, wall);
                LaidOut { inputs, feed }
            }
        };
        let LaidOut { inputs, feed } = &laid_out;
        let ids: Vec<&CommitId> = inputs.pieces.iter().filter_map(Piece::id).collect();
        let at = self.index(&ids);
        self.at = at;
        self.selected = ids[at].clone();
        let height = usize::from(area.height);
        let header = |at: usize| feed.entries.get(at).map_or(0, |entry| entry.header);
        if !self.moved {
            let focus = inputs
                .pieces
                .iter()
                .filter_map(Piece::id)
                .position(|id| *id == self.focus)
                .unwrap_or(0);
            self.offset = header(focus).saturating_sub(height / 4);
        } else if let Some(anchor) = &self.anchor
            && anchor.id == self.selected
        {
            self.offset = header(at).saturating_sub(anchor.line);
        }
        let total = feed.lines.len();
        self.offset = feed::scroll(feed, total, at, self.offset, height);
        self.anchor = Some(Anchor {
            id: self.selected.clone(),
            line: header(at).saturating_sub(self.offset),
        });
        let seen = self.offset.saturating_sub(height)..self.offset + 2 * height;
        self.near = inputs
            .pieces
            .iter()
            .filter_map(Piece::id)
            .zip(&feed.entries)
            .filter(|(id, entry)| {
                **id != self.focus
                    && seen.contains(&entry.header)
                    && store.node(id).is_some_and(|node| node.replies_count() > 0)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let end = (self.offset + height).min(total);
        let shown = self.offset.min(end)..end;
        let selected = (!self.peek).then_some(at);
        let rows = feed.card_rows(selected, &shown);
        self.card = feed::card_area(area, &rows);
        let lines = feed.window(selected, shown, area.width);
        frame.render_widget(Paragraph::new(&lines[..]), area);
        if selected.is_some() {
            for (spark, elapsed) in timers.sparks(&self.selected, now) {
                feed::spark(
                    frame.buffer_mut(),
                    area,
                    &lines,
                    rows.clone(),
                    spark,
                    elapsed,
                );
            }
        }
        self.laid_out = Some(laid_out);
    }
}

fn walk(
    store: &Store,
    owner: &CommitId,
    depth: usize,
    view: &View,
    seen: &mut HashSet<CommitId>,
    pieces: &mut Vec<Piece>,
) {
    let note = |note| Piece::Note {
        owner: owner.clone(),
        depth,
        note,
    };
    let replies = match store.replies(owner) {
        None => {
            let count = store.node(owner).map_or(0, |node| node.replies_count());
            if count > 0 {
                pieces.push(note(Note::Count(count)));
            }
            return;
        }
        Some(Branch::Failed(_)) => {
            pieces.push(note(Note::Failed(
                view.countdown(Timer::Replies(owner.clone())),
            )));
            return;
        }
        Some(Branch::Listed(replies)) => replies,
    };
    if replies.reload.is_some() {
        pieces.push(note(Note::Reload(
            view.countdown(Timer::Replies(owner.clone())),
        )));
    }
    for id in &replies.ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        pieces.push(Piece::Reply {
            id: id.clone(),
            depth,
            parent: owner.clone(),
        });
        walk(store, id, depth + 1, view, seen, pieces);
    }
    if replies.next_cursor.is_some() {
        pieces.push(note(match replies.more {
            Some(_) => Note::MoreFailed(view.countdown(Timer::More(owner.clone()))),
            None => {
                let count = store.node(owner).map_or(0, |node| node.replies_count());
                let listed = u32::try_from(replies.ids.len()).unwrap_or(u32::MAX);
                Note::More(count.saturating_sub(listed))
            }
        }));
    }
}

fn layout<Tz: TimeZone>(
    pieces: &[Piece],
    store: &Store,
    width: u16,
    expanded: &HashSet<CommitId>,
    now: &DateTime<Tz>,
) -> Feed {
    let mut out = Writer::new(width);
    let mut entries = Vec::new();
    let mut last_indent = None;
    for piece in pieces {
        let indent = piece.indent();
        if let Some(last) = last_indent {
            out.pad = indent.min(last);
            out.gap();
        }
        last_indent = Some(indent);
        out.pad = indent;
        let header = out.lines.len();
        let lane = draw_piece(&mut out, piece, store, expanded, now);
        if piece.id().is_some() {
            entries.push(Entry {
                lines: header..out.lines.len(),
                header,
                lane,
                indent,
            });
        }
    }
    Feed {
        lines: out.lines,
        entries,
    }
}

fn draw_piece<Tz: TimeZone>(
    out: &mut Writer,
    piece: &Piece,
    store: &Store,
    expanded: &HashSet<CommitId>,
    now: &DateTime<Tz>,
) -> Lane {
    let rail = feed::rail();
    let blank = Lane::Palette(0);
    match piece {
        Piece::Earlier(_) => {
            out.line(
                Span::styled("⋯ ", theme::muted()),
                vec![Span::styled(
                    "earlier commits · enter to open",
                    theme::muted(),
                )],
            );
            blank
        }
        Piece::Ancestor(id) => match store.node(id) {
            Some(Node::Live(row)) => {
                ancestor(out, &row, now);
                row.commit.author.lane
            }
            Some(Node::Gone(_)) | None => {
                tombstone(out, id);
                blank
            }
        },
        Piece::Focus(id) => {
            let missing = matches!(
                store.thread(id).map(|thread| &thread.chain),
                Some(Chain::Missing)
            );
            match (store.node(id), missing) {
                (Some(Node::Live(row)), false) => {
                    focus(out, &row, now);
                    row.commit.author.lane
                }
                (None, false) => {
                    out.line(
                        Span::raw("  "),
                        vec![Span::styled("Loading…", theme::muted())],
                    );
                    blank
                }
                _ => {
                    out.node(
                        Span::styled("○ ", Style::new().fg(theme::RAIL)),
                        rail.clone(),
                        vec![Span::styled(
                            feed::bad_object(&id.short_hash()),
                            Style::new().fg(theme::DESTRUCTIVE),
                        )],
                    );
                    out.push(
                        rail,
                        vec![Span::styled(
                            "This commit is no longer available.",
                            theme::muted(),
                        )],
                    );
                    blank
                }
            }
        }
        Piece::Reply { id, depth, parent } => match store.node(id) {
            Some(Node::Live(row)) => {
                reply(out, &row, *depth, parent, store, expanded.contains(id), now);
                row.commit.author.lane
            }
            Some(Node::Gone(_)) | None => {
                tombstone(out, id);
                blank
            }
        },
        Piece::Note { note, .. } => {
            let failed = |text: &str, retry: &Option<u64>| {
                let mut spans = vec![Span::styled(
                    text.to_owned(),
                    Style::new().fg(theme::DESTRUCTIVE),
                )];
                if let Some(secs) = retry {
                    spans.push(Span::styled(
                        format!(" · retrying in {secs}s"),
                        theme::muted(),
                    ));
                }
                spans
            };
            let spans = match note {
                Note::Count(1) => vec![Span::styled("⋯ 1 reply", theme::muted())],
                Note::Count(count) => {
                    vec![Span::styled(format!("⋯ {count} replies"), theme::muted())]
                }
                Note::Loading => vec![Span::styled("loading replies", theme::muted())],
                Note::More(0) => vec![Span::styled("⋯ more replies", theme::muted())],
                Note::More(count) => vec![Span::styled(
                    format!("⋯ {count} more replies"),
                    theme::muted(),
                )],
                Note::Failed(retry) => failed("fatal: could not load replies", retry),
                Note::Reload(retry) => failed("fatal: could not reload replies", retry),
                Note::MoreFailed(retry) => failed("fatal: could not load more replies", retry),
            };
            out.push(Span::raw("  "), spans);
            blank
        }
    }
}

fn tombstone(out: &mut Writer, id: &CommitId) {
    out.line(
        Span::styled("○ ", Style::new().fg(theme::RAIL)),
        vec![Span::styled(
            feed::bad_object(&id.short_hash()),
            theme::muted(),
        )],
    );
}

fn node(row: &Row) -> Span<'static> {
    Span::styled("● ", Style::new().fg(theme::lane(row.commit.author.lane)))
}

fn ancestor<Tz: TimeZone>(out: &mut Writer, row: &Row, now: &DateTime<Tz>) {
    let dim = |spans: Vec<Span<'static>>| -> Vec<Span<'static>> {
        spans
            .into_iter()
            .map(|span| {
                let style = span.style.fg(theme::MUTED_FOREGROUND);
                span.style(style)
            })
            .collect()
    };
    let commit = &row.commit;
    out.node(
        node(row),
        feed::rail(),
        dim(feed::header(commit, out.content(), now, None)),
    );
    let mut lines = feed::body(&commit.body_segments, out.content());
    if lines.len() > ANCESTOR_LINES {
        lines.truncate(ANCESTOR_LINES);
        if let Some(last) = lines.last_mut() {
            last.push(Span::raw("…"));
            *last = feed::fit(std::mem::take(last), out.content());
        }
    }
    for line in lines {
        out.line(feed::rail(), dim(line));
    }
}

fn focus<Tz: TimeZone>(out: &mut Writer, row: &Row, now: &DateTime<Tz>) {
    let rail = feed::rail();
    let commit = &row.commit;
    out.node(
        node(row),
        rail.clone(),
        feed::header(commit, out.content(), now, None),
    );
    feed::content(
        out,
        &rail,
        row,
        false,
        commit.quoted_commit_id.is_some(),
        now,
    );
    let at = commit.created_at.with_timezone(&now.timezone());
    out.push(
        rail.clone(),
        vec![Span::styled(
            format!(
                "{} · commit {}",
                at.naive_local().format("%-I:%M %p · %b %-d, %Y"),
                commit.short_hash
            ),
            theme::muted(),
        )],
    );
    feed::stats(out, &rail, row);
}

fn reply<Tz: TimeZone>(
    out: &mut Writer,
    row: &Row,
    depth: usize,
    parent: &CommitId,
    store: &Store,
    expanded: bool,
    now: &DateTime<Tz>,
) {
    let rail = feed::rail();
    let commit = &row.commit;
    let quotes_parent = commit.quoted_commit_id.as_ref() == Some(parent);
    let mark = quotes_parent.then_some("quoted");
    out.node(
        node(row),
        rail.clone(),
        feed::header(commit, out.content(), now, mark),
    );
    if depth > MAX_DEPTH {
        let verb = if quotes_parent {
            "quoting"
        } else {
            "replying to"
        };
        let whom = match store.node(parent) {
            Some(Node::Live(parent)) => format!("@{}", parent.commit.author.username),
            Some(Node::Gone(_)) | None => "[deleted]".to_owned(),
        };
        out.push(
            rail.clone(),
            vec![Span::styled(format!("↳ {verb} {whom}"), theme::muted())],
        );
    }
    let quoted = !quotes_parent && commit.quoted_commit_id.is_some() && row.quoted_commit.is_some();
    feed::content(out, &rail, row, !expanded, quoted, now);
    feed::stats(out, &rail, row);
}
