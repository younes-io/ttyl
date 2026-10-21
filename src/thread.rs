use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::feed;
use crate::keys::Action;
use crate::model::CommitId;
use crate::store::Store;

pub struct ThreadScreen {
    focus: CommitId,
    selected: CommitId,
    opened_at: Instant,
}

impl ThreadScreen {
    pub fn new(focus: CommitId, now: Instant) -> Self {
        ThreadScreen {
            selected: focus.clone(),
            focus,
            opened_at: now,
        }
    }

    pub fn focus(&self) -> &CommitId {
        &self.focus
    }

    pub fn selected(&self) -> &CommitId {
        &self.selected
    }

    fn ids(&self, store: &Store) -> Vec<CommitId> {
        store
            .row(&self.focus)
            .map(|_| self.focus.clone())
            .into_iter()
            .collect()
    }

    pub fn can(&self, action: Action, store: &Store) -> bool {
        let ids = self.ids(store);
        match action {
            Action::Down | Action::Up | Action::Top | Action::Bottom | Action::Open => {
                !ids.is_empty()
            }
            Action::Back | Action::Show(_) | Action::NextLog => true,
            Action::Expand | Action::Refresh => false,
            Action::Quit | Action::Help | Action::Close | Action::SignIn | Action::StartOver => {
                false
            }
        }
    }

    /// The thread Enter opens: the selected commit's, unless it is this thread's focus.
    pub fn opens(&self, store: &Store) -> Option<CommitId> {
        self.ids(store)
            .contains(&self.selected)
            .then(|| self.selected.clone())
            .filter(|id| *id != self.focus)
    }

    pub fn act(&mut self, action: Action, store: &Store) {
        let ids = self.ids(store);
        let Some(last) = ids.len().checked_sub(1) else {
            return;
        };
        let at = ids.iter().position(|id| *id == self.selected).unwrap_or(0);
        let to = match action {
            Action::Down => (at + 1).min(last),
            Action::Up => at.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => return,
        };
        self.selected = ids[to].clone();
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        _now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let _ = self.opened_at;
        let width = usize::from(area.width);
        let lines: Vec<Line> = store.row(&self.focus).map_or_else(Vec::new, |row| {
            let mut lines = vec![Line::from(feed::header(&row.commit, width, wall))];
            lines.extend(
                feed::body(&row.commit.body_segments, width)
                    .into_iter()
                    .map(Line::from),
            );
            lines
        });
        frame.render_widget(Paragraph::new(lines), area);
    }
}
