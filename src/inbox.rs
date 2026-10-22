use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::keys::Action;
use crate::model::Filter;
use crate::store::Store;
use crate::theme;

pub const TITLE: &str = "notifications";

#[derive(Default)]
pub struct Inbox {
    filter: Filter,
}

fn index(filter: Filter) -> usize {
    Filter::ALL
        .iter()
        .position(|each| *each == filter)
        .expect("every filter is listed")
}

impl Inbox {
    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn can(&self, action: Action, _store: &Store) -> bool {
        matches!(
            action,
            Action::Back | Action::NextFilter | Action::PreviousFilter
        )
    }

    pub fn act(&mut self, action: Action, _store: &Store) {
        let at = index(self.filter);
        let count = Filter::ALL.len();
        match action {
            Action::NextFilter => self.filter = Filter::ALL[(at + 1) % count],
            Action::PreviousFilter => self.filter = Filter::ALL[(at + count - 1) % count],
            _ => {}
        }
    }

    /// The filters under the top line, the active one bold, with single gaps when narrow.
    pub fn filter_bar(&self, width: u16) -> Line<'static> {
        let wide: usize = Filter::ALL.iter().map(|each| each.slug().len() + 2).sum();
        let gap = if wide <= usize::from(width) + 1 {
            "  "
        } else {
            " "
        };
        let spans = Filter::ALL.iter().enumerate().flat_map(|(at, filter)| {
            let lead = if at == 0 { " " } else { gap };
            let style = if *filter == self.filter {
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::muted()
            };
            [Span::raw(lead), Span::styled(filter.slug(), style)]
        });
        Line::from(spans.collect::<Vec<_>>())
    }
}
