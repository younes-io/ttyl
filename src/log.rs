use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::feed;
use crate::keys::{Action, InputMode};
use crate::model::{CommitId, Log};
use crate::store::{Slot, Store};
use crate::theme;

pub const LOGS: [Log; 3] = [Log::Following, Log::Latest, Log::Trending];

pub struct LogScreen {
    active: Log,
    places: [Place; 3],
    expanded: HashSet<CommitId>,
}

#[derive(Default, Clone, Copy)]
struct Place {
    selected: usize,
    offset: usize,
}

impl Default for LogScreen {
    fn default() -> Self {
        LogScreen {
            active: Log::Following,
            places: [Place::default(); 3],
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
        retry: Option<Duration>,
        now: &DateTime<Tz>,
    ) {
        match store.slot(self.active) {
            Slot::Loading => frame.render_widget(Line::styled("Loading…", theme::muted()), area),
            Slot::Failed(message) => {
                let mut lines = vec![Line::styled(
                    format!("fatal: {message}"),
                    Style::new().fg(theme::DESTRUCTIVE),
                )];
                if let Some(retry) = retry {
                    lines.push(Line::styled(
                        format!("retrying in {}s · R retry now", retry.as_secs()),
                        theme::muted(),
                    ));
                }
                frame.render_widget(Paragraph::new(lines), area);
            }
            Slot::Loaded(_) => {
                let page = store.page(self.active).expect("a loaded slot has a page");
                let place = &mut self.places[index(self.active)];
                place.selected = place.selected.min(page.rows.len().saturating_sub(1));
                let selected = (!page.rows.is_empty()).then_some(place.selected);
                let feed = feed::layout(&page, area.width, selected, &self.expanded, now);
                place.offset = scroll(
                    &feed,
                    place.selected,
                    place.offset,
                    usize::from(area.height),
                );
                frame.render_widget(
                    Paragraph::new(feed.lines)
                        .scroll((u16::try_from(place.offset).unwrap_or(u16::MAX), 0)),
                    area,
                );
            }
        }
    }
}

fn scroll(feed: &feed::Feed, selected: usize, offset: usize, height: usize) -> usize {
    let Some(range) = feed.entries.get(selected) else {
        return 0;
    };
    let offset = if selected == 0 {
        0
    } else {
        offset.min(feed.lines.len().saturating_sub(height))
    };
    if range.start < offset {
        return range.start;
    }
    if range.end > offset + height {
        return (range.end - height).min(range.start);
    }
    offset
}
