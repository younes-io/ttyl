use std::ops::Range;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::attach::{
    Attachment, DESCRIPTION_LIMIT, Duration, Image, Look, MAX_IMAGES, MAX_OPTIONS, MIN_OPTIONS,
    OPTION_LIMIT, Poll,
};
use crate::body;
use crate::draft::{Draft, Target};
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
const OPTION_MARK: &str = "○ ";
/// An option shows how much room it has left once it gets this close to the limit.
const OPTION_COUNT_FROM: usize = 10;
const POLL_REMOVED: &str = "poll removed · ctrl-p brings it back";
const EITHER: &str = "a commit carries a poll or images, not both.";
/// A description shows how much room it has left once it gets this close to the limit.
const DESCRIPTION_COUNT_FROM: usize = 100;
/// A long file name gives way to the description before it.
const NAME_SHOWN: usize = 24;
const DESCRIBE: &str = "describe it for people who can't see it";

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

/// Where typing goes: the body, or a row of the poll under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Body,
    Option(usize),
    Duration,
    Image(usize),
}

#[derive(Clone)]
struct PollForm {
    options: Vec<Buffer>,
    duration: Duration,
}

impl PollForm {
    fn blank() -> Self {
        PollForm {
            options: vec![Buffer::default(); MIN_OPTIONS],
            duration: Duration::default(),
        }
    }

    fn from(poll: &Poll) -> Self {
        let mut options: Vec<Buffer> = (poll.options.iter())
            .take(MAX_OPTIONS)
            .map(|option| Buffer::new(&one_line(option, OPTION_LIMIT)))
            .collect();
        options.resize(options.len().max(MIN_OPTIONS), Buffer::default());
        PollForm {
            options,
            duration: poll.duration,
        }
    }

    fn poll(&self) -> Poll {
        Poll {
            options: (self.options.iter())
                .map(|option| option.text().to_owned())
                .collect(),
            duration: self.duration,
        }
    }

    /// The options, then the row that says how long it runs.
    fn rows(&self) -> usize {
        self.options.len() + 1
    }
}

/// An image picked to go with the commit, and what its file said when it was read.
struct Picked {
    path: PathBuf,
    description: Buffer,
    /// Not read yet, as when a draft is opened again.
    look: Option<Look>,
}

impl Picked {
    fn new(path: PathBuf, description: &str, look: Option<Look>) -> Self {
        Picked {
            path,
            description: Buffer::new(&one_line(description, DESCRIPTION_LIMIT)),
            look,
        }
    }

    fn image(&self) -> Image {
        Image {
            path: self.path.clone(),
            description: self.description.text().to_owned(),
        }
    }

    fn name(&self) -> String {
        name(&self.path)
    }

    /// Why this file can't go, once it has been read.
    fn refused(&self) -> Option<String> {
        match &self.look {
            Some(Look::Refused(refusal)) => Some(refusal.clone()),
            Some(Look::Gone) => Some("is gone from disk.".to_owned()),
            _ => None,
        }
    }

    /// Why the server would turn this image down, if it would.
    fn refusal(&self) -> Option<String> {
        match &self.look {
            Some(Look::Fine { .. }) => None,
            Some(Look::Refused(refusal)) => Some(refusal.clone()),
            Some(Look::Gone) => Some("is gone from disk.".to_owned()),
            None => Some("is still being read.".to_owned()),
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A size in the server's megabytes of 1,024,000 bytes, so a file it calls 10 MB reads 10 MB here.
fn size(bytes: u64) -> String {
    match bytes {
        0..1_024 => format!("{bytes} B"),
        1_024..1_024_000 => format!("{} KB", bytes / 1_024),
        _ => format!("{:.1} MB", bytes as f64 / 1_024_000.0),
    }
}

/// `text` on one line, cut to `limit` characters, the way an option takes typing and pastes.
fn one_line(text: &str, limit: usize) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(limit)
        .collect()
}

pub struct Composer {
    picker: Option<Picker>,
    dismissed: Option<usize>,
    target: Target,
    title: String,
    text: Buffer,
    poll: Option<PollForm>,
    /// The poll last taken off, so taking it off is never a loss.
    stash: Option<PollForm>,
    images: Vec<Picked>,
    focus: Focus,
    restored: bool,
    note: Option<Note>,
    prompt: Option<Prompt>,
    width: usize,
    scroll: usize,
}

impl Composer {
    pub fn open(target: Target, handle: Option<&str>, draft: Option<&Draft>) -> Self {
        let title = match (&target, handle) {
            (Target::Reply(_), Some(handle)) => format!("Reply to @{handle}"),
            (Target::Quote(_), Some(handle)) => format!("Quote @{handle}"),
            _ => "commit".to_owned(),
        };
        let poll = match draft.map(|draft| &draft.attachment) {
            Some(Attachment::Poll(poll)) if target == Target::New => Some(PollForm::from(poll)),
            _ => None,
        };
        let images = match draft.map(|draft| &draft.attachment) {
            Some(Attachment::Images(images)) => (images.iter())
                .take(MAX_IMAGES)
                .map(|image| Picked::new(image.path.clone(), &image.description, None))
                .collect(),
            _ => Vec::new(),
        };
        Composer {
            picker: None,
            dismissed: None,
            target,
            title,
            text: Buffer::new(draft.map_or("", |draft| draft.text.as_str())),
            poll,
            stash: None,
            images,
            focus: Focus::Body,
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

    /// What to keep if the composer closes now, empty poll rows and all.
    pub fn draft(&self) -> Draft {
        let attachment = match &self.poll {
            Some(form) => Attachment::Poll(form.poll()),
            None if !self.images.is_empty() => {
                Attachment::Images(self.images.iter().map(Picked::image).collect())
            }
            None => Attachment::None,
        };
        Draft {
            text: self.text.text().to_owned(),
            attachment,
        }
    }

    /// What goes to the server with the body.
    pub fn attachment(&self) -> Attachment {
        match &self.poll {
            Some(form) => Attachment::Poll(form.poll().sent()),
            None => self.draft().attachment,
        }
    }

    /// Why the server would turn this commit down, if it would. Images can go without a body.
    pub fn refusal(&self) -> Option<String> {
        let bare = !self.images.is_empty() && self.text.text().trim().is_empty();
        (!bare)
            .then(|| body::refusal(self.text.text()))
            .flatten()
            .or_else(|| self.poll.as_ref().and_then(|form| form.poll().refusal()))
            .or_else(|| {
                (self.images.iter().enumerate()).find_map(|(at, picked)| {
                    let refusal = picked.refusal()?;
                    let joint = if refusal.starts_with("is ") {
                        " "
                    } else {
                        ": "
                    };
                    Some(format!("image {}{joint}{refusal}", at + 1))
                })
            })
    }

    /// The images not read yet, which a draft brings back by path alone.
    pub fn unread(&self) -> Vec<PathBuf> {
        (self.images.iter())
            .filter(|picked| picked.look.is_none())
            .map(|picked| picked.path.clone())
            .collect()
    }

    /// Takes in what reading some files found. A file already picked is brought up to date, and a new one is
    /// added if the commit has room. When the files came from `pasted` text and none of them is there, the
    /// text was only text, and goes in as typed.
    pub fn read(&mut self, looks: Vec<(PathBuf, Look)>, pasted: Option<&str>) -> bool {
        if let Some(text) = pasted
            && looks.iter().all(|(_, look)| *look == Look::Gone)
        {
            return self.paste(text);
        }
        let before = self.draft();
        let mut added = Vec::new();
        let mut note = None;
        for (path, look) in looks {
            if let Some(picked) = self.images.iter_mut().find(|picked| picked.path == path) {
                picked.look = Some(look);
            } else if self.poll.is_some() {
                note = Some(EITHER.to_owned());
            } else if self.images.len() == MAX_IMAGES {
                note = Some(format!("a commit carries up to {MAX_IMAGES} images."));
            } else {
                added.push(name(&path));
                self.images.push(Picked::new(path, "", Some(look)));
            }
        }
        let changed = self.edited(before);
        if let Some(note) = note {
            self.note(note, Style::new().fg(theme::AMBER));
        } else if let [one] = added.as_slice() {
            self.note(
                format!("attached {one} · tab to describe it"),
                theme::muted(),
            );
        } else if !added.is_empty() {
            self.note(
                format!("attached {} images · tab to describe them", added.len()),
                theme::muted(),
            );
        }
        changed
    }

    /// Says why there's nothing to attach, as when the clipboard holds neither an image nor text.
    pub fn unattached(&mut self, why: &str) {
        self.note(why, Style::new().fg(theme::AMBER));
    }

    /// Only a new commit can carry a poll, and ctrl-p means the emoji picker's up while it's open.
    pub fn can_poll(&self) -> bool {
        self.target == Target::New && self.picker.is_none()
    }

    /// Puts a poll under the body, the one taken off last if there is one, or takes it off.
    pub fn toggle_poll(&mut self) -> bool {
        if !self.images.is_empty() {
            self.note(EITHER, Style::new().fg(theme::AMBER));
            return false;
        }
        let before = self.draft();
        match self.poll.take() {
            Some(form) => {
                self.stash = Some(form);
                self.focus = Focus::Body;
                self.edited(before);
                self.note(POLL_REMOVED, theme::muted());
            }
            None => {
                self.poll = Some(self.stash.take().unwrap_or_else(PollForm::blank));
                self.focus = Focus::Option(0);
                self.edited(before);
            }
        }
        true
    }

    pub fn typed(&mut self, event: KeyEvent) -> bool {
        let before = self.draft();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        match self.focus {
            Focus::Body => {}
            Focus::Option(at) => {
                self.typed_option(at, event.code, ctrl, alt);
                return self.edited(before);
            }
            Focus::Duration => {
                self.typed_duration(event.code);
                return self.edited(before);
            }
            Focus::Image(at) => {
                let removed = self.typed_image(at, event.code, ctrl, alt);
                let changed = self.edited(before);
                if let Some(name) = removed {
                    self.note(format!("removed {name}"), theme::muted());
                }
                return changed;
            }
        }
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
        let last_row = text.row_and_column(self.width).0 + 1 >= text.rows(self.width).len();
        match event.code {
            KeyCode::Down | KeyCode::Tab
                if self.poll.is_some() && (last_row || event.code == KeyCode::Tab) =>
            {
                self.focus = Focus::Option(0);
            }
            KeyCode::Down | KeyCode::Tab
                if !self.images.is_empty() && (last_row || event.code == KeyCode::Tab) =>
            {
                self.focus = Focus::Image(0);
            }
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

    fn typed_option(&mut self, at: usize, code: KeyCode, ctrl: bool, alt: bool) {
        let Some(form) = &mut self.poll else {
            return;
        };
        let count = form.options.len();
        let option = &mut form.options[at];
        let room = OPTION_LIMIT.saturating_sub(option.text().chars().count());
        match code {
            KeyCode::Up | KeyCode::BackTab if at == 0 => self.focus = Focus::Body,
            KeyCode::Up | KeyCode::BackTab => self.focus = Focus::Option(at - 1),
            KeyCode::Down | KeyCode::Tab if at + 1 < count => self.focus = Focus::Option(at + 1),
            KeyCode::Down | KeyCode::Tab => self.focus = Focus::Duration,
            KeyCode::Enter if at + 1 < count => self.focus = Focus::Option(at + 1),
            KeyCode::Enter if count < MAX_OPTIONS && !option.text().trim().is_empty() => {
                form.options.push(Buffer::default());
                self.focus = Focus::Option(at + 1);
            }
            KeyCode::Enter => self.focus = Focus::Duration,
            KeyCode::Backspace if option.is_empty() && count > MIN_OPTIONS => {
                form.options.remove(at);
                self.focus = Focus::Option(at.saturating_sub(1));
            }
            KeyCode::Char('w') if ctrl => option.delete_word(),
            KeyCode::Char('u') if ctrl => option.delete_line(),
            KeyCode::Char(ch) if !ctrl && !alt && room > 0 && !ch.is_control() => {
                option.insert(ch.encode_utf8(&mut [0; 4]));
            }
            KeyCode::Backspace => option.backspace(),
            KeyCode::Delete => option.delete(),
            KeyCode::Left if ctrl => option.word_left(),
            KeyCode::Right if ctrl => option.word_right(),
            KeyCode::Left => option.left(),
            KeyCode::Right => option.right(),
            KeyCode::Home => option.home(),
            KeyCode::End => option.end(),
            _ => {}
        }
    }

    /// Returns the name of the image the key removed, if it removed one.
    fn typed_image(&mut self, at: usize, code: KeyCode, ctrl: bool, alt: bool) -> Option<String> {
        let count = self.images.len();
        let picked = self.images.get_mut(at)?;
        let refused = picked.refused().is_some();
        let description = &mut picked.description;
        let room = DESCRIPTION_LIMIT.saturating_sub(description.text().chars().count());
        match code {
            KeyCode::Up | KeyCode::BackTab if at == 0 => self.focus = Focus::Body,
            KeyCode::Up | KeyCode::BackTab => self.focus = Focus::Image(at - 1),
            KeyCode::Down | KeyCode::Tab | KeyCode::Enter if at + 1 < count => {
                self.focus = Focus::Image(at + 1);
            }
            KeyCode::Tab | KeyCode::Enter => self.focus = Focus::Body,
            KeyCode::Backspace | KeyCode::Delete if description.is_empty() || refused => {
                let name = picked.name();
                self.images.remove(at);
                self.focus = match at.checked_sub(1) {
                    _ if at < self.images.len() => Focus::Image(at),
                    Some(before) => Focus::Image(before),
                    None => Focus::Body,
                };
                return Some(name);
            }
            _ if refused => {}
            KeyCode::Char('w') if ctrl => description.delete_word(),
            KeyCode::Char('u') if ctrl => description.delete_line(),
            KeyCode::Char(ch) if !ctrl && !alt && room > 0 && !ch.is_control() => {
                description.insert(ch.encode_utf8(&mut [0; 4]));
            }
            KeyCode::Backspace => description.backspace(),
            KeyCode::Delete => description.delete(),
            KeyCode::Left if ctrl => description.word_left(),
            KeyCode::Right if ctrl => description.word_right(),
            KeyCode::Left => description.left(),
            KeyCode::Right => description.right(),
            KeyCode::Home => description.home(),
            KeyCode::End => description.end(),
            _ => {}
        }
        None
    }

    fn typed_duration(&mut self, code: KeyCode) {
        let Some(form) = &mut self.poll else {
            return;
        };
        match code {
            KeyCode::Up | KeyCode::BackTab => self.focus = Focus::Option(form.options.len() - 1),
            KeyCode::Tab => self.focus = Focus::Body,
            KeyCode::Left => form.duration = form.duration.step(false),
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                form.duration = form.duration.step(true);
            }
            _ => {}
        }
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
        let before = self.draft();
        match (self.focus, &mut self.poll) {
            (Focus::Option(at), Some(form)) => {
                let option = &mut form.options[at];
                let room = OPTION_LIMIT.saturating_sub(option.text().chars().count());
                option.insert(&one_line(pasted, room));
            }
            (Focus::Duration, _) => {}
            (Focus::Image(at), _) => {
                if let Some(picked) = self.images.get_mut(at)
                    && picked.refused().is_none()
                {
                    let description = &mut picked.description;
                    let room = DESCRIPTION_LIMIT.saturating_sub(description.text().chars().count());
                    description.insert(&one_line(pasted, room));
                }
            }
            _ => {
                self.text.insert(pasted);
                self.pick();
            }
        }
        self.edited(before)
    }

    /// Takes the text back from an editor, which ends a file with a newline the body never wants.
    pub fn replace(&mut self, edited: &str) -> bool {
        let before = self.draft();
        self.text = Buffer::new(edited.trim_end_matches(['\n', '\r']));
        self.focus = Focus::Body;
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

    fn edited(&mut self, before: Draft) -> bool {
        let changed = self.draft() != before;
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

    /// The poll or images under the body, with a blank row between them.
    fn attached_rows(&self) -> usize {
        match &self.poll {
            _ if self.prompt == Some(Prompt::Rules) => 0,
            Some(form) => 1 + form.rows(),
            None if !self.images.is_empty() => 1 + self.images.len(),
            None => 0,
        }
    }

    pub fn height(&self, width: u16, screen: u16) -> u16 {
        let status = self.status(inner_width(width)).len();
        let poll = self.attached_rows();
        // A poll's question is often one line, so the options sit right under it.
        let least = if poll > 0 { 1 } else { MIN_ROWS };
        let most = (usize::from(screen) / 2)
            .saturating_sub(TITLE + status + poll)
            .max(least);
        let rows = match self.prompt {
            Some(Prompt::Rules) => rules(inner_width(width)).len(),
            _ => self.lines(layout_width(width)).len(),
        };
        u16::try_from(rows.clamp(least, most) + TITLE + status + poll).unwrap_or(u16::MAX)
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
        let poll_rows = u16::try_from(self.attached_rows()).unwrap_or(u16::MAX);
        let text_area = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(status_rows + poll_rows),
            ..inner
        };
        let poll_area = Rect {
            y: text_area.y + text_area.height,
            height: poll_rows,
            ..text_area
        };
        let status_area = Rect {
            y: poll_area.y + poll_area.height,
            height: status_rows,
            ..text_area
        };
        frame.render_widget(Paragraph::new(status), status_area);
        if poll_rows > 0 && self.poll.is_some() {
            self.draw_poll(frame, poll_area);
        } else if poll_rows > 0 {
            self.draw_images(frame, poll_area);
        }
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
        if self.focus == Focus::Body {
            let x = u16::try_from(column).unwrap_or(u16::MAX);
            let y = u16::try_from(row - self.scroll).unwrap_or(u16::MAX);
            frame.set_cursor_position((
                text_area.x + x.min(text_area.width),
                text_area.y + y.min(text_area.height.saturating_sub(1)),
            ));
        }
        if let Some(picker) = &mut self.picker {
            let (_, column) = self.text.place(self.width, picker.at);
            let colon = text_area.x + u16::try_from(column).unwrap_or(u16::MAX);
            draw_picker(frame, picker, colon, area.y);
        }
    }

    /// One row per option and a row for how long it runs, each with a hint for the focused one.
    fn draw_poll(&self, frame: &mut Frame, area: Rect) {
        let Some(form) = &self.poll else {
            return;
        };
        let width = usize::from(area.width);
        let room = width.saturating_sub(OPTION_MARK.width());
        let count = form.options.len();
        let mut lines = vec![Line::default()];
        for (at, option) in form.options.iter().enumerate() {
            let focused = self.focus == Focus::Option(at);
            let mark = if focused { theme::PRIMARY } else { theme::RAIL };
            let text = option.text();
            let typed = text.chars().count();
            let left = OPTION_LIMIT.saturating_sub(typed);
            let hint = match () {
                _ if !focused => String::new(),
                _ if left <= OPTION_COUNT_FROM => format!("{left} left"),
                _ if at + 1 == count && count < MAX_OPTIONS && !text.trim().is_empty() => {
                    "enter adds an option".to_owned()
                }
                _ if text.is_empty() && count > MIN_OPTIONS => "backspace removes it".to_owned(),
                _ => String::new(),
            };
            let (shown, style) = if text.is_empty() {
                (format!("option {}", at + 1), Style::new().fg(theme::FAINT))
            } else {
                (
                    if focused {
                        visible(text, option.cursor(), room)
                    } else {
                        fatal::cut(text, room)
                    },
                    Style::new().fg(theme::FOREGROUND),
                )
            };
            let mut spans = vec![
                Span::styled(OPTION_MARK, Style::new().fg(mark)),
                Span::styled(shown.clone(), style),
            ];
            let used = OPTION_MARK.width() + shown.width();
            if !hint.is_empty() && used + 2 + hint.width() <= width {
                spans.push(Span::raw(" ".repeat(width - used - hint.width())));
                spans.push(Span::styled(hint, theme::muted()));
            }
            lines.push(Line::from(spans));
            if focused {
                let before = &text[..option.cursor()];
                let column = before.width().min(room.saturating_sub(1));
                let x = u16::try_from(OPTION_MARK.width() + column).unwrap_or(u16::MAX);
                let y = u16::try_from(1 + at).unwrap_or(u16::MAX);
                frame.set_cursor_position((area.x + x, area.y + y));
            }
        }
        let focused = self.focus == Focus::Duration;
        let arrow = if focused {
            Style::new().fg(theme::ACCENT)
        } else {
            Style::new().fg(theme::FAINT)
        };
        let label = Style::new().fg(if focused {
            theme::FOREGROUND
        } else {
            theme::MUTED_FOREGROUND
        });
        let mut spans = vec![
            Span::raw(" ".repeat(OPTION_MARK.width())),
            Span::styled("lasts ", theme::muted()),
            Span::styled("‹ ", arrow),
            Span::styled(form.duration.label(), label),
            Span::styled(" ›", arrow),
        ];
        let used: usize = spans.iter().map(Span::width).sum();
        let hint = "← → change";
        if focused && used + 2 + hint.width() <= width {
            spans.push(Span::raw(" ".repeat(width - used - hint.width())));
            spans.push(Span::styled(hint, theme::muted()));
        }
        lines.push(Line::from(spans));
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// One row per image, its description on the left and what the file is on the right.
    fn draw_images(&self, frame: &mut Frame, area: Rect) {
        let width = usize::from(area.width);
        let icon = theme::icons().image;
        let mark_width = icon.width() + 1;
        let mut lines = vec![Line::default()];
        for (at, picked) in self.images.iter().enumerate() {
            let focused = self.focus == Focus::Image(at);
            let name = fatal::cut(&picked.name(), NAME_SHOWN);
            let mut spans = Vec::new();
            if let Some(refusal) = picked.refused() {
                spans.push(Span::styled(
                    format!("{icon} "),
                    Style::new().fg(theme::DESTRUCTIVE),
                ));
                let hint = if focused { "backspace removes it" } else { "" };
                let room = width.saturating_sub(mark_width + hint.width() + 2);
                let text = fatal::cut(&format!("{name} {refusal}"), room);
                let used = mark_width + text.width();
                spans.push(Span::styled(text, Style::new().fg(theme::DESTRUCTIVE)));
                if !hint.is_empty() && used + 2 + hint.width() <= width {
                    spans.push(Span::raw(" ".repeat(width - used - hint.width())));
                    spans.push(Span::styled(hint, theme::muted()));
                }
                lines.push(Line::from(spans));
                continue;
            }
            let description = picked.description.text();
            let left = DESCRIPTION_LIMIT.saturating_sub(description.chars().count());
            let tail = match &picked.look {
                _ if focused && left <= DESCRIPTION_COUNT_FROM => format!("{left} left"),
                Some(Look::Fine { probe, bytes }) => format!(
                    "{name} · {}×{} · {}",
                    probe.width,
                    probe.height,
                    size(*bytes)
                ),
                _ => name,
            };
            // A narrow composer gives the description the row, and the file's details make way.
            let tail = if width >= mark_width + 16 + 2 + tail.width() {
                tail
            } else {
                String::new()
            };
            let room =
                width.saturating_sub(mark_width + tail.width() + usize::from(!tail.is_empty()) * 2);
            let mark = if focused { theme::PRIMARY } else { theme::RAIL };
            spans.push(Span::styled(format!("{icon} "), Style::new().fg(mark)));
            let shown = if description.is_empty() {
                let placeholder = fatal::cut(DESCRIBE, room);
                spans.push(Span::styled(
                    placeholder.clone(),
                    Style::new().fg(theme::FAINT),
                ));
                placeholder
            } else {
                // A row being written follows its cursor, and the others show how they start.
                let shown = if focused {
                    visible(description, picked.description.cursor(), room)
                } else {
                    fatal::cut(description, room)
                };
                spans.push(Span::styled(
                    shown.clone(),
                    Style::new().fg(theme::FOREGROUND),
                ));
                shown
            };
            if !tail.is_empty() {
                let used = mark_width + shown.width();
                spans.push(Span::raw(
                    " ".repeat(width.saturating_sub(used + tail.width())),
                ));
                spans.push(Span::styled(tail, Style::new().fg(theme::FAINT)));
            }
            lines.push(Line::from(spans));
            if focused {
                let before = &description[..picked.description.cursor()];
                let column = before.width().min(room.saturating_sub(1));
                let x = u16::try_from(mark_width + column).unwrap_or(u16::MAX);
                let y = u16::try_from(1 + at).unwrap_or(u16::MAX);
                frame.set_cursor_position((area.x + x, area.y + y));
            }
        }
        frame.render_widget(Paragraph::new(lines), area);
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

/// The part of a one-line `text` that fits in `room` cells with the cursor at byte `cursor` in view.
fn visible(text: &str, cursor: usize, room: usize) -> String {
    if text.width() <= room {
        return text.to_owned();
    }
    let mut start = 0;
    while text[start..cursor].width() >= room {
        start += text[start..].chars().next().map_or(1, char::len_utf8);
    }
    let mut shown = String::new();
    for ch in text[start..].chars() {
        if shown.width() + UnicodeWidthChar::width(ch).unwrap_or(0) > room {
            break;
        }
        shown.push(ch);
    }
    shown
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
