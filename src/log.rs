use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::keys::Action;
use crate::model::{CommitId, Log};
use crate::store::{Slot, Store};
use crate::theme;
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
}

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
        }
    }
}

fn index(log: Log) -> usize {
    LOGS.iter()
        .position(|each| *each == log)
        .expect("every log has a tab")
}

impl LogScreen {
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
            Action::Expand => ids
                .get(self.selected())
                .and_then(|id| store.row(id))
                .is_some_and(|row| row.commit.preview_segments.is_some()),
            Action::Open => !ids.is_empty(),
            Action::Show(_) | Action::NextLog | Action::Refresh => true,
            Action::Back
            | Action::Quit
            | Action::Help
            | Action::Close
            | Action::SignIn
            | Action::StartOver
            | Action::Confirm
            | Action::Pick(_)
            | Action::Commit(_)
            | Action::Compose => false,
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
    pub fn top_bar(&self, store: &Store, width: u16) -> Line<'static> {
        let width = usize::from(width);
        let (left, right) = (0..CUTS.len())
            .map(|cut| self.top_bar_as(&CUTS[..cut], store))
            .find(|(left, right)| left.width() + 2 + right.width() <= width)
            .unwrap_or_else(|| self.top_bar_as(&CUTS, store));
        let pad = width.saturating_sub(left.width() + right.width());
        let mut spans = left.spans;
        spans.push(Span::raw(" ".repeat(pad)));
        spans.extend(right.spans);
        Line::from(spans)
    }

    fn top_bar_as(&self, cuts: &[Cut], store: &Store) -> (Line<'static>, Line<'static>) {
        let kept = |cut| !cuts.contains(&cut);
        let mut left = Vec::new();
        if kept(Cut::Brand) {
            left.push(Span::styled(
                " /dev/null ",
                Style::new().fg(theme::BRAND).add_modifier(Modifier::BOLD),
            ));
        }
        for (index, log) in LOGS.iter().enumerate() {
            let gap = if index == 0 && !kept(Cut::Brand) {
                " "
            } else {
                "  "
            };
            let label = if kept(Cut::Slugs) {
                format!("{gap}{} {}", index + 1, log.slug())
            } else {
                format!("{gap}{}", index + 1)
            };
            let style = if *log == self.active {
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::muted()
            };
            left.push(Span::styled(label, style));
        }
        let mut right = Vec::new();
        if store.unread > 0 {
            right.push(Span::styled("● ", Style::new().fg(theme::PRIMARY)));
            if kept(Cut::Number) {
                right.push(Span::styled(format!("{}  ", store.unread), theme::muted()));
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

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        retry: Retry,
        now: &DateTime<Tz>,
    ) {
        match store.slot(self.active) {
            Slot::Loading => frame.render_widget(Line::styled("Loading…", theme::muted()), area),
            Slot::Failed(failure) => frame.render_widget(
                Paragraph::new(fatal::screen(failure, retry.page)).wrap(Wrap { trim: false }),
                area,
            ),
            Slot::Loaded(listing) => {
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
                        let feed = feed::layout(&page, area.width, &self.expanded, now);
                        LaidOut { inputs, feed }
                    }
                };
                let feed = &place.laid_out.insert(laid_out).feed;
                let tail = retry.older.map(feed::older_failed);
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
                place.anchor = id.map(|id| Anchor {
                    id,
                    line: start(place.selected).saturating_sub(place.offset),
                });
                let end = (place.offset + height).min(total);
                let shown = place.offset.min(feed.lines.len())..end.min(feed.lines.len());
                let mut lines = feed.window(selected, shown);
                if let Some(tail) = tail
                    && end > feed.lines.len()
                {
                    let from = place.offset.saturating_sub(feed.lines.len());
                    lines.extend_from_slice(&tail[from..end - feed.lines.len()]);
                }
                frame.render_widget(Paragraph::new(lines), area);
            }
        }
    }
}
