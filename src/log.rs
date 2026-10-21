use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::keys::{Action, InputMode};
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
    minute: i64,
}

struct Anchor {
    id: CommitId,
    line: usize,
}

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

    pub fn input_mode(&self) -> InputMode {
        InputMode::Commands
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
            Action::Show(_) | Action::NextLog | Action::Refresh => true,
            Action::Quit | Action::Help | Action::Close | Action::SignIn | Action::StartOver => {
                false
            }
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

    pub fn tabs(&self) -> Line<'static> {
        let mut spans = vec![Span::styled(
            " /dev/null ",
            Style::new().fg(theme::BRAND).add_modifier(Modifier::BOLD),
        )];
        for (index, log) in LOGS.iter().enumerate() {
            let label = format!("  {} {}", index + 1, log.slug());
            let style = if *log == self.active {
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::muted()
            };
            spans.push(Span::styled(label, style));
        }
        Line::from(spans)
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
                    minute: now.timestamp().div_euclid(60),
                };
                let place = &mut self.places[index(self.active)];
                if place
                    .laid_out
                    .as_ref()
                    .is_none_or(|laid_out| laid_out.inputs != inputs)
                {
                    let page = store.page(self.active).expect("a loaded slot has a page");
                    let feed = feed::layout(&page, area.width, &self.expanded, now);
                    place.laid_out = Some(LaidOut { inputs, feed });
                }
                let feed = &place.laid_out.as_ref().expect("laid out above").feed;
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
                place.offset = scroll(feed, total, place.selected, place.offset, height);
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

fn scroll(feed: &feed::Feed, total: usize, selected: usize, offset: usize, height: usize) -> usize {
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
