use std::collections::HashSet;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::feed;
use crate::model::{CommitId, Entry, Log, LogPage};
use crate::theme;

const LOGS: [Log; 3] = [Log::Following, Log::Latest, Log::Trending];

pub struct App {
    tabs: [Tab; 3],
    active: usize,
    expanded: HashSet<CommitId>,
    pub quit: bool,
}

struct Tab {
    page: LogPage,
    selected: usize,
    offset: usize,
}

impl App {
    /// Takes the three log pages in tab order and opens on Following, the web's home log for a signed-in user.
    pub fn new(pages: [LogPage; 3]) -> Self {
        App {
            tabs: pages.map(|page| Tab {
                page,
                selected: 0,
                offset: 0,
            }),
            active: 0,
            expanded: HashSet::new(),
            quit: false,
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let tab = &mut self.tabs[self.active];
        let last = tab.page.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('Q') => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => tab.selected = (tab.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => tab.selected = tab.selected.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => tab.selected = 0,
            KeyCode::Char('G') | KeyCode::End => tab.selected = last,
            KeyCode::Char(' ') => {
                if let Some(Entry::Commit(commit)) = tab.page.entries.get(tab.selected)
                    && commit.preview_segments.is_some()
                    && !self.expanded.remove(&commit.id)
                {
                    self.expanded.insert(commit.id.clone());
                }
            }
            KeyCode::Char(digit @ '1'..='3') => self.active = usize::from(digit as u8 - b'1'),
            KeyCode::Tab => self.active = (self.active + 1) % LOGS.len(),
            _ => {}
        }
    }

    pub fn draw<Tz: TimeZone>(&mut self, frame: &mut Frame, now: &DateTime<Tz>) {
        let area = frame.area();
        frame.render_widget(
            Block::new().style(Style::new().bg(theme::BACKGROUND).fg(theme::FOREGROUND)),
            area,
        );
        let [top, _, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(self.tab_bar(), top);
        frame.render_widget(
            Line::styled(" j/k move  space expand  1-3 log  Q quit", theme::muted()),
            bottom,
        );

        let body = Rect {
            x: body.x + 1,
            width: body.width.saturating_sub(2),
            ..body
        };
        let tab = &mut self.tabs[self.active];
        let selected = (!tab.page.entries.is_empty()).then_some(tab.selected);
        let feed = feed::layout(&tab.page, body.width, selected, &self.expanded, now);
        tab.offset = scroll(&feed, tab.selected, tab.offset, usize::from(body.height));
        frame.render_widget(
            Paragraph::new(feed.lines).scroll((u16::try_from(tab.offset).unwrap_or(u16::MAX), 0)),
            body,
        );
    }

    fn tab_bar(&self) -> Line<'static> {
        let mut spans = vec![Span::styled(
            " /dev/null ",
            Style::new().fg(theme::BRAND).add_modifier(Modifier::BOLD),
        )];
        for (index, log) in LOGS.iter().enumerate() {
            let label = format!("  {} {}", index + 1, format!("{log:?}").to_lowercase());
            let style = if index == self.active {
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
}

/// Moves the viewport the least distance that shows the whole selected entry, or its top when it is taller than
/// the viewport. The first entry measures from the top so its day marker stays visible whenever the entry fits.
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
