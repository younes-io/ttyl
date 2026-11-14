use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::api::Failure;
use crate::at;
use crate::fatal;
use crate::feed;
use crate::keys::{self, Action};
use crate::model::{Scheduled, ScheduledId};
use crate::theme;

pub const TITLE: &str = "later";

enum Load {
    Loading,
    Ready(Vec<Scheduled>),
    Failed(Failure),
}

/// The commits waiting to go out, soonest first, as atq lists jobs.
pub struct QueueScreen {
    load: Load,
    selected: usize,
    /// The first row on screen.
    top: usize,
}

impl Default for QueueScreen {
    fn default() -> Self {
        QueueScreen {
            load: Load::Loading,
            selected: 0,
            top: 0,
        }
    }
}

impl QueueScreen {
    fn items(&self) -> &[Scheduled] {
        match &self.load {
            Load::Ready(items) => items,
            Load::Loading | Load::Failed(_) => &[],
        }
    }

    pub fn selected(&self) -> Option<&Scheduled> {
        self.items().get(self.selected)
    }

    /// Takes in the queue, keeping the selection on the commit it was on.
    pub fn land(&mut self, items: Vec<Scheduled>) {
        let id = self.selected().map(|item| item.id.clone());
        self.selected = (id.and_then(|id| items.iter().position(|item| item.id == id)))
            .unwrap_or(self.selected)
            .min(items.len().saturating_sub(1));
        self.load = Load::Ready(items);
    }

    /// A failed reload keeps the queue on screen.
    pub fn fail(&mut self, failure: Failure) {
        if !matches!(self.load, Load::Ready(_)) {
            self.load = Load::Failed(failure);
        }
    }

    /// Takes out one that went out or was canceled.
    pub fn remove(&mut self, id: &ScheduledId) {
        if let Load::Ready(items) = &mut self.load {
            items.retain(|item| item.id != *id);
            self.selected = self.selected.min(items.len().saturating_sub(1));
        }
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Back | Action::Refresh => true,
            Action::Down | Action::Bottom => self.selected + 1 < self.items().len(),
            Action::Up | Action::Top => self.selected > 0,
            Action::Unschedule | Action::SendNow => self.selected().is_some(),
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action) {
        let last = self.items().len().saturating_sub(1);
        self.selected = match action {
            Action::Down => (self.selected + 1).min(last),
            Action::Up => self.selected.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => self.selected,
        };
    }

    pub fn draw<Tz: TimeZone>(&mut self, frame: &mut Frame, area: Rect, wall: &DateTime<Tz>)
    where
        Tz::Offset: std::fmt::Display,
    {
        let items = match &self.load {
            Load::Loading => {
                frame.render_widget(Line::styled("loading the queue", theme::muted()), area);
                return;
            }
            Load::Failed(failure) => {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, None)).wrap(Wrap { trim: false }),
                    area,
                );
                return;
            }
            Load::Ready(items) => items,
        };
        let width = usize::from(area.width);
        if items.is_empty() {
            let empty = match keys::key_for(Action::Schedule) {
                Some(key) => {
                    format!("Nothing scheduled. In the composer, {key} sends a commit later.")
                }
                None => "Nothing scheduled.".to_owned(),
            };
            let mut lines = Vec::new();
            feed::wrap(&[(empty, theme::muted())], width, &mut lines);
            frame.render_widget(
                Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()),
                area,
            );
            return;
        }
        let mut lines: Vec<Line> = Vec::new();
        let mut shown = 0..0;
        for (at, item) in items.iter().enumerate() {
            if at > 0 {
                lines.push(Line::default());
            }
            let start = lines.len();
            let selected = at == self.selected;
            let rail = Span::styled(
                "│ ",
                Style::new().fg(if selected {
                    theme::PRIMARY
                } else {
                    theme::RAIL
                }),
            );
            let when = item.at.with_timezone(&wall.timezone());
            let mut head = vec![
                rail.clone(),
                Span::styled(
                    at::shown(&when),
                    Style::new()
                        .fg(theme::FOREGROUND)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" · {}", at::until(&when, wall)), theme::muted()),
            ];
            let mut extras = Vec::new();
            if item.poll {
                extras.push("poll".to_owned());
            }
            match item.images {
                0 => {}
                1 => extras.push("1 image".to_owned()),
                count => extras.push(format!("{count} images")),
            }
            if let Some(author) = &item.author {
                extras.push(format!("as @{author}"));
            }
            for extra in extras {
                head.push(Span::styled(format!(" · {extra}"), theme::muted()));
            }
            lines.push(Line::from(head));
            let room = width.saturating_sub(rail.width()).max(1);
            for paragraph in item.body.lines() {
                let mut rows = Vec::new();
                feed::wrap(
                    &[(paragraph.to_owned(), Style::new().fg(theme::FOREGROUND))],
                    room,
                    &mut rows,
                );
                for row in rows {
                    let mut spans = vec![rail.clone()];
                    spans.extend(row);
                    lines.push(Line::from(spans));
                }
            }
            if selected {
                shown = start..lines.len();
            }
        }
        let height = usize::from(area.height);
        if shown.start < self.top {
            self.top = shown.start;
        } else if shown.end > self.top + height {
            self.top = (shown.end - height).min(shown.start);
        }
        let lines: Vec<Line> = lines.into_iter().skip(self.top).take(height).collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}
