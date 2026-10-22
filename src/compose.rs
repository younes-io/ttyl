use std::ops::Range;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::body;
use crate::draft::Target;
use crate::fatal;
use crate::feed;
use crate::text::Buffer;
use crate::theme;

const MIN_ROWS: usize = 3;
const TITLE: usize = 1;
const RESTORED: &str = "stash@{0}: restored your unsent draft";
const FOLDS: &str = "the feed folds your commit here";
const SENDING: &str = "committing…";
const COUNT_FROM: isize = 200;
const ASK: &str = "possible secret detected. commit anyway? [y/N]";
const ASK_SHORT: &str = "possible secret. commit anyway? [y/N]";
const RULES: [&str; 2] = [
    "Your first commit has to come from the web,",
    "so you can read the commit rules first.",
];
const SAVED: &str = "Your draft is saved.";

/// A question the panel puts to the user in place of editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// The body looks like it holds a secret, and the next key says whether to send it anyway.
    Secret,
    /// The server takes a first commit only from the web, where the commit rules are.
    Rules,
}

/// A status line message, with a shorter wording for a narrow panel.
struct Note {
    wide: String,
    narrow: Option<&'static str>,
    style: Style,
}

pub struct Composer {
    target: Target,
    title: String,
    text: Buffer,
    restored: bool,
    note: Option<Note>,
    prompt: Option<Prompt>,
    width: usize,
    scroll: usize,
}

impl Composer {
    pub fn open(target: Target, handle: Option<&str>, draft: Option<&str>) -> Self {
        let title = match (&target, handle) {
            (Target::Reply(_), Some(handle)) => format!("Reply to @{handle}"),
            (Target::Quote(_), Some(handle)) => format!("Quote @{handle}"),
            _ => "commit".to_owned(),
        };
        Composer {
            target,
            title,
            text: Buffer::new(draft.unwrap_or("")),
            restored: draft.is_some(),
            note: None,
            prompt: None,
            width: 80,
            scroll: 0,
        }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn text(&self) -> &str {
        self.text.text()
    }

    /// Applies one key and says whether the text changed, which is what the draft on disk follows.
    pub fn typed(&mut self, event: KeyEvent) -> bool {
        let before = self.text.text().to_owned();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        let text = &mut self.text;
        match event.code {
            KeyCode::Char('w') if ctrl => text.delete_word(),
            KeyCode::Char('u') if ctrl => text.delete_line(),
            KeyCode::Char(ch) if !ctrl && !alt => text.insert(ch.encode_utf8(&mut [0; 4])),
            KeyCode::Enter if !ctrl && !alt => text.insert("\n"),
            KeyCode::Tab => text.insert("\t"),
            KeyCode::Backspace => text.backspace(),
            KeyCode::Delete => text.delete(),
            KeyCode::Left if ctrl => text.word_left(),
            KeyCode::Right if ctrl => text.word_right(),
            KeyCode::Left => text.left(),
            KeyCode::Right => text.right(),
            KeyCode::Up => text.up(self.width),
            KeyCode::Down => text.down(self.width),
            KeyCode::Home => text.home(),
            KeyCode::End => text.end(),
            _ => {}
        }
        self.edited(before)
    }

    pub fn paste(&mut self, pasted: &str) -> bool {
        if self.prompt == Some(Prompt::Rules) {
            return false;
        }
        let before = self.text.text().to_owned();
        self.text.insert(pasted);
        self.edited(before)
    }

    /// Takes the text back from an editor, which ends a file with a newline the body never wants.
    pub fn replace(&mut self, edited: &str) -> bool {
        let before = self.text.text().to_owned();
        self.text = Buffer::new(edited.trim_end_matches(['\n', '\r']));
        self.edited(before)
    }

    /// Shows `text` on the status line until the next edit.
    pub fn note(&mut self, text: impl Into<String>, style: Style) {
        self.note = Some(Note {
            wide: text.into(),
            narrow: None,
            style,
        });
    }

    /// Like `note`, with `narrow` in place of `wide` when the panel has no room for it.
    pub fn note_or(&mut self, wide: &str, narrow: &'static str, style: Style) {
        self.note = Some(Note {
            wide: wide.to_owned(),
            narrow: Some(narrow),
            style,
        });
    }

    pub fn ask(&mut self, prompt: Prompt) {
        self.prompt = Some(prompt);
    }

    pub fn prompt(&self) -> Option<Prompt> {
        self.prompt
    }

    pub fn answered(&mut self) {
        self.prompt = None;
    }

    fn edited(&mut self, before: String) -> bool {
        let changed = self.text.text() != before;
        if changed {
            self.restored = false;
            self.note = None;
            self.prompt = None;
        }
        changed
    }

    /// The text rows at `width`, with `None` for the line that marks where the feed folds.
    fn lines(&self, width: usize) -> Vec<Option<Range<usize>>> {
        let rows = self.text.rows(width);
        let text = self.text.text();
        let fold = body::fold(text).map(|at| {
            text.char_indices()
                .nth(at)
                .map_or(text.len(), |(byte, _)| byte)
        });
        let marker = fold.and_then(|fold| rows.iter().position(|row| fold <= row.end));
        let mut lines: Vec<Option<Range<usize>>> = rows.into_iter().map(Some).collect();
        if let Some(at) = marker {
            lines.insert(at + 1, None);
        }
        lines
    }

    /// Rows the panel takes on a `screen` tall terminal: a title, the text, and the status rows.
    pub fn height(&self, width: u16, screen: u16) -> u16 {
        let status = self.status(inner_width(width)).len();
        let most = (usize::from(screen) / 2)
            .saturating_sub(TITLE + status)
            .max(MIN_ROWS);
        let rows = match self.prompt {
            Some(Prompt::Rules) => rules(inner_width(width)).len(),
            _ => self.lines(layout_width(width)).len(),
        };
        u16::try_from(rows.clamp(MIN_ROWS, most) + TITLE + status).unwrap_or(u16::MAX)
    }

    /// Draws the panel; `sending` says a commit from this target is on its way.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, sending: bool) {
        self.width = layout_width(area.width);
        let title = if sending { SENDING } else { &self.title };
        let block = Block::new()
            .borders(Borders::TOP)
            .border_style(theme::muted())
            .title(Span::styled(
                format!(" {title} "),
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let status = self.status(inner_width(area.width));
        let status_rows = u16::try_from(status.len()).unwrap_or(u16::MAX);
        let text_area = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(status_rows),
            ..inner
        };
        let status_area = Rect {
            y: text_area.y + text_area.height,
            height: status_rows,
            ..text_area
        };
        frame.render_widget(Paragraph::new(status), status_area);
        if self.prompt == Some(Prompt::Rules) {
            let copy = rules(usize::from(text_area.width));
            frame.render_widget(Paragraph::new(copy), text_area);
            return;
        }
        let lines = self.lines(self.width);
        let (row, column) = self.text.row_and_column(self.width);
        let marker = lines.iter().position(Option::is_none);
        let row = row + usize::from(marker.is_some_and(|at| row >= at));
        let shown = usize::from(text_area.height);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + shown {
            self.scroll = row + 1 - shown;
        }
        let lines: Vec<Line> = lines
            .iter()
            .skip(self.scroll)
            .take(shown)
            .map(|line| match line {
                Some(range) => Line::raw(self.text.display(range)),
                None => Line::styled(FOLDS, theme::muted()),
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), text_area);
        let x = u16::try_from(column).unwrap_or(u16::MAX);
        let y = u16::try_from(row - self.scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((
            text_area.x + x.min(text_area.width),
            text_area.y + y.min(text_area.height.saturating_sub(1)),
        ));
    }

    /// The rows under the text: the note, wrapped when it has no shorter wording, with the counter on the last.
    fn status(&self, width: usize) -> Vec<Line<'static>> {
        if self.prompt == Some(Prompt::Rules) {
            return vec![Line::styled(SAVED, theme::muted())];
        }
        let left = body::left(self.text.text());
        let count = if left <= COUNT_FROM {
            format!("{left} left")
        } else {
            String::new()
        };
        let count_style = if left < 0 {
            Style::new().fg(theme::DESTRUCTIVE)
        } else {
            theme::muted()
        };
        let room = width.saturating_sub(count.width() + usize::from(!count.is_empty()));
        let (wide, narrow, style) = match &self.note {
            _ if self.prompt == Some(Prompt::Secret) => {
                (ASK, Some(ASK_SHORT), Style::new().fg(theme::AMBER))
            }
            Some(note) => (note.wide.as_str(), note.narrow, note.style),
            None if self.restored => (RESTORED, None, theme::muted()),
            None => ("", None, theme::muted()),
        };
        let mut rows = Vec::new();
        match narrow {
            Some(narrow) => rows.push(vec![Span::styled(
                fatal::cut(fit(wide, narrow, room), room),
                style,
            )]),
            None => feed::wrap(&[(wide.to_owned(), style)], room.max(1), &mut rows),
        }
        if let Some(last) = rows.last_mut() {
            let used: usize = last.iter().map(Span::width).sum();
            last.push(Span::raw(
                " ".repeat(width.saturating_sub(used + count.width())),
            ));
            last.push(Span::styled(count, count_style));
        }
        rows.into_iter().map(Line::from).collect()
    }
}

/// The first-commit copy, broken where the blueprint breaks it when the panel is wide enough.
fn rules(width: usize) -> Vec<Line<'static>> {
    let style = Style::new().fg(theme::FOREGROUND);
    if RULES.iter().all(|line| line.width() <= width) {
        return RULES
            .iter()
            .map(|line| Line::styled(*line, style))
            .collect();
    }
    let mut rows = Vec::new();
    feed::wrap(&[(RULES.join(" "), style)], width.max(1), &mut rows);
    rows.into_iter().map(Line::from).collect()
}

fn fit<'a>(wide: &'a str, narrow: &'a str, room: usize) -> &'a str {
    if wide.width() <= room { wide } else { narrow }
}

fn inner_width(width: u16) -> usize {
    usize::from(width).saturating_sub(2)
}

/// The panel's inner width less one cell, so the cursor after a full row still has a cell.
fn layout_width(width: u16) -> usize {
    usize::from(width).saturating_sub(3).max(1)
}
