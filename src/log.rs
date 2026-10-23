use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Wrap};

use crate::clock::{Timer, Timers};
use crate::keys::Action;
use crate::model::{CommitId, Log};
use crate::store::{Slot, Store};
use crate::theme;
use crate::thread::{LOADING_NOTE_DELAY, ThreadScreen};
use crate::{fatal, feed};

pub const LOGS: [Log; 3] = [Log::Following, Log::Latest, Log::Trending];

pub struct Retry {
    pub page: Option<Duration>,
    pub older: Option<Duration>,
}

pub struct LogScreen {
    active: Log,
    places: [Place; 3],
    expanded: HashSet<CommitId>,
    peek: Option<ThreadScreen>,
}

#[derive(Default)]
struct Place {
    selected: usize,
    offset: usize,
    anchor: Option<Anchor>,
    laid_out: Option<LaidOut>,
}

struct LaidOut {
    inputs: Inputs,
    feed: feed::Feed,
    folded: Vec<bool>,
}

/// From this width the selected commit's thread gets a pane of its own beside the feed.
const WIDE: u16 = 110;
const TALL: u16 = 8;
/// How long the selection rests on a commit before the pane fetches its thread, so scrolling past
/// commits sends nothing.
const DWELL: Duration = LOADING_NOTE_DELAY;

#[derive(PartialEq)]
struct Inputs {
    width: u16,
    generation: u64,
    expanded: HashSet<CommitId>,
    second: i64,
}

struct Anchor {
    id: CommitId,
    line: usize,
}

/// What the top bar gives up, in order, when the row is too short.
#[derive(Clone, Copy, PartialEq)]
enum Cut {
    Brand,
    Handle,
    Number,
    Slugs,
}

const CUTS: [Cut; 4] = [Cut::Brand, Cut::Handle, Cut::Number, Cut::Slugs];

impl Default for LogScreen {
    fn default() -> Self {
        LogScreen {
            active: Log::Following,
            places: Default::default(),
            expanded: HashSet::new(),
            peek: None,
        }
    }
}

fn index(log: Log) -> usize {
    LOGS.iter()
        .position(|each| *each == log)
        .expect("every log has a tab")
}

impl LogScreen {
    /// The pane's thread, once the selection has rested on it long enough to fetch.
    pub fn peek(&self, store: &Store, now: Instant) -> Option<&ThreadScreen> {
        let peek = self.peek.as_ref()?;
        let selected = self.selected_id(self.active, store)?;
        (peek.focus() == selected && now >= peek.opened_at() + DWELL).then_some(peek)
    }

    pub fn peek_due(&self) -> Option<Instant> {
        self.peek.as_ref().map(|peek| peek.opened_at() + DWELL)
    }

    pub fn active(&self) -> Log {
        self.active
    }

    pub fn selected(&self) -> usize {
        self.places[index(self.active)].selected
    }

    pub fn selected_id<'a>(&self, log: Log, store: &'a Store) -> Option<&'a CommitId> {
        match store.slot(log) {
            Slot::Loaded(listing) => listing.ids.get(self.places[index(log)].selected),
            Slot::Loading | Slot::Failed(_) => None,
        }
    }

    pub fn anchor(&self, log: Log, store: &Store) -> Option<CommitId> {
        if self.places[index(log)].selected == 0 {
            return None;
        }
        self.selected_id(log, store).cloned()
    }

    pub fn reselect(&mut self, log: Log, id: Option<&CommitId>, store: &Store) {
        if let (Some(id), Slot::Loaded(listing)) = (id, store.slot(log)) {
            let place = &mut self.places[index(log)];
            place.selected = listing.ids.iter().position(|each| each == id).unwrap_or(0);
        }
    }

    fn ids<'a>(&self, store: &'a Store) -> &'a [CommitId] {
        match store.slot(self.active) {
            Slot::Loaded(listing) => &listing.ids,
            Slot::Loading | Slot::Failed(_) => &[],
        }
    }

    pub fn can(&self, action: Action, store: &Store) -> bool {
        let ids = self.ids(store);
        match action {
            Action::Down | Action::Up | Action::Top | Action::Bottom => !ids.is_empty(),
            Action::Expand => {
                let place = &self.places[index(self.active)];
                let folded = place
                    .laid_out
                    .as_ref()
                    .is_some_and(|laid_out| laid_out.folded.get(place.selected) == Some(&true));
                folded
                    || ids
                        .get(self.selected())
                        .is_some_and(|id| self.expanded.contains(id))
            }
            Action::Open => !ids.is_empty(),
            Action::Show(_) | Action::NextLog | Action::Refresh => true,
            Action::Back
            | Action::Quit
            | Action::Help
            | Action::Saver
            | Action::Close
            | Action::SignIn
            | Action::StartOver
            | Action::Confirm
            | Action::Pick(_)
            | Action::Commit(_)
            | Action::Compose
            | Action::Send
            | Action::Editor
            | Action::Site
            | Action::Inbox
            | Action::NextFilter
            | Action::PreviousFilter => false,
        }
    }

    pub fn act(&mut self, action: Action, store: &Store) {
        let last = self.ids(store).len().saturating_sub(1);
        let place = &mut self.places[index(self.active)];
        match action {
            Action::Down => place.selected = (place.selected + 1).min(last),
            Action::Up => place.selected = place.selected.saturating_sub(1),
            Action::Top => place.selected = 0,
            Action::Bottom => place.selected = last,
            Action::Expand => {
                if let Some(id) = self.ids(store).get(self.selected())
                    && !self.expanded.remove(id)
                {
                    self.expanded.insert(id.clone());
                }
            }
            Action::Show(log) => self.active = log,
            Action::NextLog => self.active = LOGS[(index(self.active) + 1) % LOGS.len()],
            _ => {}
        }
    }

    /// One row: the brand and the tabs on the left, unread and the handle on the right,
    /// giving up `CUTS` in order until it fits.
    pub fn top_bar(
        &self,
        store: &Store,
        title: Option<&str>,
        glow: bool,
        width: u16,
    ) -> Line<'static> {
        let width = usize::from(width);
        let (left, right) = (0..CUTS.len())
            .map(|cut| self.top_bar_as(&CUTS[..cut], store, title, glow))
            .find(|(left, right)| left.width() + 2 + right.width() <= width)
            .unwrap_or_else(|| self.top_bar_as(&CUTS, store, title, glow));
        let pad = width.saturating_sub(left.width() + right.width());
        let mut spans = left.spans;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right.spans);
        Line::from(spans)
    }

    fn top_bar_as(
        &self,
        cuts: &[Cut],
        store: &Store,
        title: Option<&str>,
        glow: bool,
    ) -> (Line<'static>, Line<'static>) {
        let kept = |cut| !cuts.contains(&cut);
        let mut left = Vec::new();
        if kept(Cut::Brand) {
            left.push(Span::styled(
                " /dev/null ",
                Style::new().fg(theme::BRAND).add_modifier(Modifier::BOLD),
            ));
        }
        let bold = Style::new()
            .fg(theme::FOREGROUND)
            .add_modifier(Modifier::BOLD);
        let lead = |index: usize| {
            if index == 0 && !kept(Cut::Brand) {
                " "
            } else {
                "  "
            }
        };
        if let Some(title) = title {
            left.push(Span::styled(format!("{}{title}", lead(0)), bold));
        }
        if title.is_none() {
            left.push(Span::raw(&lead(0)[1..]));
        }
        let pill = Style::new()
            .fg(theme::BACKGROUND)
            .bg(theme::PRIMARY)
            .add_modifier(Modifier::BOLD);
        for (index, log) in LOGS.iter().enumerate().filter(|_| title.is_none()) {
            let label = if kept(Cut::Slugs) {
                format!(" {} {} ", index + 1, log.slug())
            } else {
                format!(" {} ", index + 1)
            };
            let style = if *log == self.active {
                pill
            } else {
                theme::muted()
            };
            left.push(Span::styled(label, style));
        }
        let mut right = Vec::new();
        if store.unread > 0 {
            let lit = Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD);
            let (dot, number) = if glow {
                (lit, lit)
            } else {
                (Style::new().fg(theme::PRIMARY), theme::muted())
            };
            right.push(Span::styled("● ", dot));
            if kept(Cut::Number) {
                right.push(Span::styled(format!("{}  ", store.unread), number));
            }
        }
        if kept(Cut::Handle) {
            right.push(Span::styled(
                format!("@{} ", store.me.username),
                theme::muted(),
            ));
        }
        (Line::from(left), Line::from(right))
    }

    /// Whether a feed in this area shows the selected commit's thread beside it.
    pub fn has_pane(&self, area: Rect, store: &Store) -> bool {
        area.width >= WIDE && area.height >= TALL && !self.ids(store).is_empty()
    }

    /// On a wide terminal, returns the feed's box and the box beside it left for a thread.
    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        timers: &Timers,
        at: Instant,
        now: &DateTime<Tz>,
    ) -> Option<Panes> {
        let retry = Retry {
            page: timers.remaining(&Timer::Reload(self.active), at),
            older: timers.remaining(&Timer::Older(self.active), at),
        };
        match store.slot(self.active) {
            Slot::Loading => {
                frame.render_widget(Line::styled("Loading…", theme::muted()), area);
                None
            }
            Slot::Failed(failure) => {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, retry.page)).wrap(Wrap { trim: false }),
                    area,
                );
                None
            }
            Slot::Loaded(listing) => {
                let wide = self.has_pane(area, store);
                let (area, pane) = if wide {
                    let feed = area.width * 3 / 5;
                    let [feed, pane] =
                        Layout::horizontal([Constraint::Length(feed), Constraint::Fill(1)])
                            .spacing(1)
                            .areas(area);
                    (boxed(feed), Some(Panes { feed, pane }))
                } else {
                    (area, None)
                };
                let inputs = Inputs {
                    width: area.width,
                    generation: store.generation(),
                    expanded: self.expanded.clone(),
                    second: now.timestamp(),
                };
                let place = &mut self.places[index(self.active)];
                let laid_out = match place.laid_out.take() {
                    Some(laid_out) if laid_out.inputs == inputs => laid_out,
                    _ => {
                        let page = store.page(self.active).expect("a loaded slot has a page");
                        let (feed, folded) =
                            feed::layout(&page, area.width, &self.expanded, !wide, now);
                        LaidOut {
                            inputs,
                            feed,
                            folded,
                        }
                    }
                };
                let feed = &place.laid_out.insert(laid_out).feed;
                let tail = retry
                    .older
                    .map(|retry| feed::older_failed("commits", retry));
                let total = feed.lines.len() + tail.as_ref().map_or(0, |tail| tail.len());
                place.selected = place.selected.min(listing.ids.len().saturating_sub(1));
                let selected = (!listing.ids.is_empty()).then_some(place.selected);
                let id = selected.map(|at| listing.ids[at].clone());
                let start = |at: usize| feed.entries.get(at).map_or(0, |entry| entry.header);
                if let (Some(anchor), Some(id)) = (&place.anchor, &id)
                    && anchor.id == *id
                {
                    place.offset = start(place.selected).saturating_sub(anchor.line);
                }
                let height = usize::from(area.height);
                place.offset = feed::scroll(feed, total, place.selected, place.offset, height);
                place.anchor = id.clone().map(|id| Anchor {
                    id,
                    line: start(place.selected).saturating_sub(place.offset),
                });
                let end = (place.offset + height).min(total);
                let shown = place.offset.min(feed.lines.len())..end.min(feed.lines.len());
                let rows = feed.card_rows(selected, &shown);
                let mut lines = feed.window(selected, shown, area.width);
                if let Some(tail) = tail
                    && end > feed.lines.len()
                {
                    let from = place.offset.saturating_sub(feed.lines.len());
                    lines.extend_from_slice(&tail[from..end - feed.lines.len()]);
                }
                frame.render_widget(Paragraph::new(&lines[..]), area);
                if let Some((spark, elapsed)) = id.and_then(|id| timers.spark(&id, at)) {
                    feed::spark(frame.buffer_mut(), area, &lines, rows, spark, elapsed);
                }
                if pane.is_none() {
                    self.peek = None;
                }
                pane
            }
        }
    }

    pub fn draw_peek<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        pane: Rect,
        store: &Store,
        timers: &Timers,
        at: Instant,
        now: &DateTime<Tz>,
    ) {
        let Some(id) = self.selected_id(self.active, store).cloned() else {
            return;
        };
        let inner = draw_box(frame, pane, "thread", false);
        let peek = match self.peek.take() {
            Some(peek) if *peek.focus() == id => peek,
            _ => ThreadScreen::peek(id, at),
        };
        self.peek
            .insert(peek)
            .draw(frame, inner, store, timers, at, now);
    }
}

pub struct Panes {
    pub feed: Rect,
    pub pane: Rect,
}

fn frame_block(title: &str, focused: bool) -> Block<'static> {
    let (border, label) = if focused {
        (
            theme::PRIMARY,
            Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
        )
    } else {
        (theme::RAIL, Style::new().fg(theme::FAINT))
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Span::styled(format!(" {title} "), label))
        .padding(Padding::horizontal(1))
}

fn boxed(area: Rect) -> Rect {
    frame_block("", false).inner(area)
}

/// The box whose border is lit holds the keys.
pub fn draw_box(frame: &mut Frame, area: Rect, title: &str, focused: bool) -> Rect {
    let block = frame_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}
