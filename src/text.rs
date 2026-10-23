use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const TAB: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buffer {
    text: String,
    cursor: usize,
    goal: Option<usize>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let text = clean(text);
        let cursor = text.len();
        Buffer {
            text,
            cursor,
            goal: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn insert(&mut self, text: &str) {
        let text = clean(text);
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.settle();
    }

    pub fn left(&mut self) {
        self.cursor = self.before(self.cursor);
        self.goal = None;
    }

    pub fn right(&mut self) {
        self.cursor = self.after(self.cursor);
        self.goal = None;
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
        self.goal = None;
    }

    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |at| self.cursor + at);
        self.goal = None;
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start(self.cursor);
        self.goal = None;
    }

    pub fn word_right(&mut self) {
        let mut at = self.cursor;
        while at < self.text.len() && !self.wordy(at) {
            at = self.after(at);
        }
        while at < self.text.len() && self.wordy(at) {
            at = self.after(at);
        }
        self.cursor = at;
        self.goal = None;
    }

    pub fn backspace(&mut self) {
        let start = self.before(self.cursor);
        self.remove(start..self.cursor);
    }

    pub fn delete(&mut self) {
        let end = self.after(self.cursor);
        self.remove(self.cursor..end);
    }

    pub fn delete_word(&mut self) {
        let start = self.word_start(self.cursor);
        self.remove(start..self.cursor);
    }

    pub fn delete_line(&mut self) {
        let start = self.line_start(self.cursor);
        self.remove(start..self.cursor);
    }

    /// Swaps the text from `start` up to the cursor for `text`.
    pub fn replace_before(&mut self, start: usize, text: &str) {
        self.remove(start..self.cursor);
        self.insert(text);
    }

    pub fn up(&mut self, width: usize) {
        self.vertical(width, false);
    }

    pub fn down(&mut self, width: usize) {
        self.vertical(width, true);
    }

    pub fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut used = 0;
        for (at, grapheme) in self.text.grapheme_indices(true) {
            if grapheme == "\n" {
                rows.push(start..at);
                start = at + 1;
                used = 0;
                continue;
            }
            let cells = cells(grapheme);
            if used + cells > width && at > start {
                rows.push(start..at);
                start = at;
                used = 0;
            }
            used += cells;
        }
        rows.push(start..self.text.len());
        rows
    }

    pub fn row_and_column(&self, width: usize) -> (usize, usize) {
        self.place(width, self.cursor)
    }

    pub fn place(&self, width: usize, at: usize) -> (usize, usize) {
        let rows = self.rows(width);
        locate(&self.text, &rows, at)
    }

    pub fn display(&self, row: &Range<usize>) -> String {
        self.text[row.clone()].replace('\t', &" ".repeat(TAB))
    }

    fn vertical(&mut self, width: usize, down: bool) {
        let rows = self.rows(width);
        let (row, column) = locate(&self.text, &rows, self.cursor);
        let goal = *self.goal.get_or_insert(column);
        let target = if down {
            row + 1
        } else if let Some(row) = row.checked_sub(1) {
            row
        } else {
            self.cursor = 0;
            return;
        };
        let Some(range) = rows.get(target) else {
            self.cursor = self.text.len();
            return;
        };
        let mut at = range.start;
        let mut used = 0;
        for (offset, grapheme) in self.text[range.clone()].grapheme_indices(true) {
            let cells = cells(grapheme);
            if used + cells > goal {
                break;
            }
            used += cells;
            at = range.start + offset + grapheme.len();
        }
        self.cursor = at;
    }

    fn remove(&mut self, range: Range<usize>) {
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.settle();
    }

    fn settle(&mut self) {
        self.goal = None;
        if !self.boundary(self.cursor) {
            self.cursor = self.after(self.cursor);
        }
    }

    fn boundary(&self, at: usize) -> bool {
        at == self.text.len()
            || self
                .text
                .grapheme_indices(true)
                .any(|(start, _)| start == at)
    }

    fn before(&self, at: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(start, _)| start)
            .take_while(|start| *start < at)
            .last()
            .unwrap_or(0)
    }

    fn after(&self, at: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(start, grapheme)| start + grapheme.len())
            .find(|end| *end > at)
            .unwrap_or(self.text.len())
    }

    fn line_start(&self, at: usize) -> usize {
        self.text[..at].rfind('\n').map_or(0, |newline| newline + 1)
    }

    fn word_start(&self, from: usize) -> usize {
        let mut at = from;
        while at > 0 && !self.wordy(self.before(at)) {
            at = self.before(at);
        }
        while at > 0 && self.wordy(self.before(at)) {
            at = self.before(at);
        }
        at
    }

    fn wordy(&self, at: usize) -> bool {
        self.text[at..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
    }
}

fn locate(text: &str, rows: &[Range<usize>], at: usize) -> (usize, usize) {
    let row = rows.iter().rposition(|row| row.start <= at).unwrap_or(0);
    let start = rows[row].start;
    let column = text[start..at.max(start)].graphemes(true).map(cells).sum();
    (row, column)
}

fn cells(grapheme: &str) -> usize {
    if grapheme == "\t" {
        TAB
    } else {
        grapheme.width()
    }
}

fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
