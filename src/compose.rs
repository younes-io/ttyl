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
use crate::emoji::{self, Emoji};
use crate::fatal;
use crate::feed;
use crate::popup;
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
const PICKS_SHOWN: usize = 8;
const PICK_HINT: &str = " tab insert ";
/// The border, a space and the two cells of the emoji come before a name in the picker.
const PICK_INDENT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    Secret,
    Rules,
}

struct Note {
    wide: String,
    narrow: Option<&'static str>,
    style: Style,
}

enum Row {
    Text(Range<usize>),
    Fold,
}

/// The emoji whose names match the `:name` before the cursor, all of them, in rank order.
struct Picker {
    at: usize,
    query: String,
    matches: Vec<Emoji>,
    selected: usize,
    scroll: usize,
}

impl Picker {
    /// Moves the selection one row, wrapping at the ends. The text never changes.
    fn step(&mut self, down: bool) -> bool {
        let count = self.matches.len();
        self.selected = (self.selected + if down { 1 } else { count - 1 }) % count;
        false
    }
}

pub struct Composer {
    picker: Option<Picker>,
    dismissed: Option<usize>,
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
            picker: None,
            dismissed: None,
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

    pub fn typed(&mut self, event: KeyEvent) -> bool {
        let before = self.text.text().to_owned();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        if let Some(picker) = &mut self.picker {
            match event.code {
                KeyCode::Up => return picker.step(false),
                KeyCode::Down => return picker.step(true),
                KeyCode::Char('p') if ctrl => return picker.step(false),
                KeyCode::Char('n') if ctrl => return picker.step(true),
                KeyCode::Tab | KeyCode::Enter if !ctrl && !alt => {
                    let (at, glyph) = (picker.at, picker.matches[picker.selected].glyph);
                    self.text.replace_before(at, glyph);
                    self.pick();
                    return self.edited(before);
                }
                _ => {}
            }
        }
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
        self.pick();
        self.edited(before)
    }

    /// Hides the emoji picker until the `:name` it was showing is gone, and says whether it was open.
    pub fn dismiss(&mut self) -> bool {
        let Some(picker) = self.picker.take() else {
            return false;
        };
        self.dismissed = Some(picker.at);
        true
    }

    fn pick(&mut self) {
        let before = &self.text.text()[..self.text.cursor()];
        let Some(at) = emoji::trigger(before) else {
            self.picker = None;
            self.dismissed = None;
            return;
        };
        let query = &before[at + 1..];
        if self.dismissed == Some(at)
            || self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.at == at && picker.query == query)
        {
            return;
        }
        let matches = emoji::search(query);
        self.picker = (!matches.is_empty()).then(|| Picker {
            at,
            query: query.to_owned(),
            matches,
            selected: 0,
            scroll: 0,
        });
    }

    pub fn paste(&mut self, pasted: &str) -> bool {
        if self.prompt == Some(Prompt::Rules) {
            return false;
        }
        let before = self.text.text().to_owned();
        self.text.insert(pasted);
        self.pick();
        self.edited(before)
    }

    /// Takes the text back from an editor, which ends a file with a newline the body never wants.
    pub fn replace(&mut self, edited: &str) -> bool {
        let before = self.text.text().to_owned();
        self.text = Buffer::new(edited.trim_end_matches(['\n', '\r']));
        self.pick();
        self.edited(before)
    }

    pub fn note(&mut self, text: impl Into<String>, style: Style) {
        self.note = Some(Note {
            wide: text.into(),
            narrow: None,
            style,
        });
    }

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

    fn lines(&self, width: usize) -> Vec<Row> {
        let rows = self.text.rows(width);
        let text = self.text.text();
        let fold = body::fold_char(text).map(|at| {
            text.char_indices()
                .nth(at)
                .map_or(text.len(), |(byte, _)| byte)
        });
        let marker = fold.and_then(|fold| rows.iter().position(|row| fold <= row.end));
        let mut lines: Vec<Row> = rows.into_iter().map(Row::Text).collect();
        if let Some(at) = marker {
            lines.insert(at + 1, Row::Fold);
        }
        lines
    }

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
        let marker = lines.iter().position(|line| matches!(line, Row::Fold));
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
                Row::Text(range) => Line::raw(self.text.display(range)),
                Row::Fold => Line::styled(FOLDS, theme::muted()),
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), text_area);
        let x = u16::try_from(column).unwrap_or(u16::MAX);
        let y = u16::try_from(row - self.scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((
            text_area.x + x.min(text_area.width),
            text_area.y + y.min(text_area.height.saturating_sub(1)),
        ));
        if let Some(picker) = &mut self.picker {
            let (_, column) = self.text.place(self.width, picker.at);
            let colon = text_area.x + u16::try_from(column).unwrap_or(u16::MAX);
            draw_picker(frame, picker, colon, area.y);
        }
    }

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

/// Draws the picker on the rows above the composer, its names lined up under the typed name.
fn draw_picker(frame: &mut Frame, picker: &mut Picker, colon: u16, bottom: u16) {
    let screen = frame.area();
    let room = usize::from(bottom.saturating_sub(screen.y + 2 + 2 + 1));
    let shown = picker.matches.len().min(PICKS_SHOWN).min(room);
    if shown == 0 {
        return;
    }
    if picker.selected < picker.scroll {
        picker.scroll = picker.selected;
    } else if picker.selected >= picker.scroll + shown {
        picker.scroll = picker.selected + 1 - shown;
    }
    let count = format!(" {}/{} ", picker.selected + 1, picker.matches.len());
    let query = format!(" :{} ", picker.query);
    let names = picker
        .matches
        .iter()
        .map(|emoji| emoji.name.len())
        .max()
        .unwrap_or(0);
    let width = (PICK_INDENT + names + 3)
        .max(query.width() + count.width() + 2)
        .max(PICK_HINT.width() + 2);
    let width = u16::try_from(width).unwrap_or(u16::MAX).min(screen.width);
    let x = (colon + 1)
        .saturating_sub(u16::try_from(PICK_INDENT + 1).unwrap_or(0))
        .max(screen.x)
        .min((screen.x + screen.width).saturating_sub(width + 1));
    let height = u16::try_from(shown + 2).unwrap_or(u16::MAX);
    let area = Rect {
        x,
        y: bottom - height - 1,
        width,
        height,
    };
    let block = Block::bordered()
        .title(Span::styled(
            query,
            Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
        ))
        .title(Line::styled(count, Style::new().fg(theme::FAINT)).right_aligned())
        .title_bottom(Line::styled(PICK_HINT, Style::new().fg(theme::FAINT)).right_aligned());
    let inner = popup::raise(frame, area, block);
    let rows: Vec<Line> = picker
        .matches
        .iter()
        .enumerate()
        .skip(picker.scroll)
        .take(shown)
        .map(|(at, emoji)| pick_row(emoji, &picker.query, at == picker.selected, inner.width))
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

fn pick_row(emoji: &Emoji, query: &str, selected: bool, width: u16) -> Line<'static> {
    let (base, hit) = if selected {
        (
            popup::chosen(),
            popup::chosen().add_modifier(Modifier::UNDERLINED),
        )
    } else {
        let base = Style::new().fg(theme::MUTED_FOREGROUND);
        (
            base,
            base.fg(theme::FOREGROUND).add_modifier(Modifier::BOLD),
        )
    };
    let pad = 2usize.saturating_sub(emoji.glyph.width());
    let mut spans = vec![Span::styled(
        format!(" {}{} ", emoji.glyph, " ".repeat(pad)),
        base,
    )];
    let query = query.to_lowercase();
    let start = emoji.name.find(&query).unwrap_or(emoji.name.len());
    let end = (start + query.len()).min(emoji.name.len());
    spans.push(Span::styled(emoji.name[..start].to_owned(), base));
    spans.push(Span::styled(emoji.name[start..end].to_owned(), hit));
    spans.push(Span::styled(emoji.name[end..].to_owned(), base));
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::styled(
        " ".repeat(usize::from(width).saturating_sub(used)),
        base,
    ));
    Line::from(spans)
}

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

const BORDERS: usize = 2;
const CURSOR_CELL: usize = 1;

fn inner_width(width: u16) -> usize {
    usize::from(width).saturating_sub(BORDERS)
}

fn layout_width(width: u16) -> usize {
    inner_width(width).saturating_sub(CURSOR_CELL).max(1)
}
