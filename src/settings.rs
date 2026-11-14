use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::api::Failure;
use crate::config::{self, Config, Key};
use crate::fatal;
use crate::feed;
use crate::keys::{Act, Action};
use crate::model::{Device, DeviceId, PeoplePage, User};
use crate::store::Store;
use crate::theme;

pub const TITLE: &str = "settings";

/// The widest key name, so values line up the way `git config -l | column -t` lines them up.
const NAME_WIDTH: usize = 16;
/// The widest value most settings show, so what each one changes lines up beside it.
const VALUE_WIDTH: usize = 16;

enum Load<T> {
    Loading,
    Ready(T),
    Failed(Failure),
}

impl<T> Load<T> {
    /// A failed reload keeps what was on screen.
    fn fail(&mut self, failure: Failure) {
        if !matches!(self, Load::Ready(_)) {
            *self = Load::Failed(failure);
        }
    }
}

/// The people listed so far, and the cursor of the page that comes after them.
struct Listed {
    users: Vec<User>,
    next: Option<String>,
}

fn listed(load: &Load<Listed>) -> &[User] {
    match load {
        Load::Ready(listed) => &listed.users,
        Load::Loading | Load::Failed(_) => &[],
    }
}

/// A row the cursor stops on.
#[derive(Debug, Clone, Copy)]
pub enum Row<'a> {
    Key(Key),
    Muted(&'a User),
    Blocked(&'a User),
    Device(&'a Device),
}

impl Row<'_> {
    fn spot(self) -> Spot {
        match self {
            Row::Key(key) => Spot::Key(key),
            Row::Muted(user) => Spot::Muted(user.handle.clone()),
            Row::Blocked(user) => Spot::Blocked(user.handle.clone()),
            Row::Device(device) => Spot::Device(device.id.clone()),
        }
    }

    fn is(self, spot: &Spot) -> bool {
        match (self, spot) {
            (Row::Key(key), Spot::Key(other)) => key == *other,
            (Row::Muted(user), Spot::Muted(handle))
            | (Row::Blocked(user), Spot::Blocked(handle)) => user.handle == *handle,
            (Row::Device(device), Spot::Device(id)) => device.id == *id,
            _ => false,
        }
    }
}

/// The row the cursor is on by what it shows, so it stays on it as rows above come and go.
#[derive(Debug)]
enum Spot {
    Key(Key),
    Muted(String),
    Blocked(String),
    Device(DeviceId),
}

/// The account's settings as one list under headings, the way `git config -l` lists them, with the
/// people the user mutes and blocks and the devices that sign in as them below.
pub struct SettingsScreen {
    /// Why `me/settings` didn't load, which leaves the region rows empty.
    region: Option<Failure>,
    muted: Load<Listed>,
    blocked: Load<Listed>,
    devices: Load<Vec<Device>>,
    /// The row the cursor is on.
    spot: Option<Spot>,
    /// Where `spot` was last found, where the cursor stays once that row goes.
    selected: usize,
    /// The first line on screen.
    top: usize,
}

impl Default for SettingsScreen {
    fn default() -> Self {
        SettingsScreen {
            region: None,
            muted: Load::Loading,
            blocked: Load::Loading,
            devices: Load::Loading,
            spot: None,
            selected: 0,
            top: 0,
        }
    }
}

impl SettingsScreen {
    /// Every row the cursor stops on, in the order they show.
    fn rows(&self, config: Config) -> Vec<Row<'_>> {
        let keys = (Key::ALL.into_iter())
            .filter(|key| config.has(*key))
            .map(Row::Key);
        keys.chain(listed(&self.muted).iter().map(Row::Muted))
            .chain(listed(&self.blocked).iter().map(Row::Blocked))
            .chain(self.devices().iter().map(Row::Device))
            .collect()
    }

    /// Where the cursor is among `rows`: on its row, or where that row was once it's gone.
    fn at(&self, rows: &[Row]) -> usize {
        (self.spot.as_ref())
            .and_then(|spot| rows.iter().position(|row| row.is(spot)))
            .unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)))
    }

    pub fn selected(&self, config: Config) -> Option<Row<'_>> {
        let rows = self.rows(config);
        rows.get(self.at(&rows)).copied()
    }

    /// The device the cursor is on.
    pub fn device(&self, config: Config) -> Option<&Device> {
        match self.selected(config)? {
            Row::Device(device) => Some(device),
            Row::Key(_) | Row::Muted(_) | Row::Blocked(_) => None,
        }
    }

    /// The person the cursor is on, among those the user mutes or blocks.
    pub fn person(&self, config: Config) -> Option<&User> {
        match self.selected(config)? {
            Row::Muted(user) | Row::Blocked(user) => Some(user),
            Row::Key(_) | Row::Device(_) => None,
        }
    }

    pub fn device_by(&self, id: &DeviceId) -> Option<&Device> {
        self.devices().iter().find(|device| device.id == *id)
    }

    fn devices(&self) -> &[Device] {
        match &self.devices {
            Load::Ready(devices) => devices,
            Load::Loading | Load::Failed(_) => &[],
        }
    }

    /// Takes in the page at `cursor` of the people the user mutes, or with `blocks` blocks. The
    /// first page replaces the list, as a reload starts over, and a later one must be the page the
    /// list waits for, so a page from before the reload doesn't land on the new list.
    pub fn land_people(&mut self, page: &PeoplePage, blocks: bool, cursor: Option<&str>) {
        let load = if blocks {
            &mut self.blocked
        } else {
            &mut self.muted
        };
        let users = page.people.iter().map(|person| person.user.clone());
        match (load, cursor) {
            (load, None) => {
                *load = Load::Ready(Listed {
                    users: users.collect(),
                    next: page.next_cursor.clone(),
                });
            }
            (Load::Ready(listed), Some(cursor)) if listed.next.as_deref() == Some(cursor) => {
                listed.users.extend(users);
                listed.next.clone_from(&page.next_cursor);
            }
            _ => {}
        }
    }

    /// The cursor of the page the list of people the user mutes, or with `blocks` blocks, waits for.
    pub fn awaited(&self, blocks: bool) -> Option<&str> {
        match if blocks { &self.blocked } else { &self.muted } {
            Load::Ready(listed) => listed.next.as_deref(),
            Load::Loading | Load::Failed(_) => None,
        }
    }

    pub fn fail_people(&mut self, failure: Failure, blocks: bool) {
        match blocks {
            true => self.blocked.fail(failure),
            false => self.muted.fail(failure),
        }
    }

    pub fn land_devices(&mut self, devices: Vec<Device>) {
        self.devices = Load::Ready(devices);
    }

    pub fn fail_devices(&mut self, failure: Failure) {
        self.devices.fail(failure);
    }

    pub fn fail_region(&mut self, failure: Failure) {
        self.region = Some(failure);
    }

    /// Takes out a device once dev-null.sh revoked it.
    pub fn revoked(&mut self, id: &DeviceId) {
        if let Load::Ready(devices) = &mut self.devices {
            devices.retain(|device| device.id != *id);
        }
    }

    /// Starts over, keeping the rows on screen until the new ones land.
    pub fn reload(&mut self) {
        self.region = None;
    }

    pub fn can(&self, action: Action, config: Config) -> bool {
        let rows = self.rows(config);
        let at = self.at(&rows);
        let selected = rows.get(at).copied();
        match action {
            Action::Back | Action::Refresh => true,
            Action::Down | Action::Bottom => at + 1 < rows.len(),
            Action::Up | Action::Top => at > 0,
            Action::Edit => matches!(selected, Some(Row::Key(key)) if config.has(key)),
            Action::Commit(Act::Mute) => matches!(selected, Some(Row::Muted(_))),
            Action::Commit(Act::Block) => matches!(selected, Some(Row::Blocked(_))),
            Action::Revoke => matches!(selected, Some(Row::Device(_))),
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action, config: Config) {
        let rows = self.rows(config);
        let last = rows.len().saturating_sub(1);
        let at = self.at(&rows);
        let at = match action {
            Action::Down => (at + 1).min(last),
            Action::Up => at.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => at,
        };
        self.spot = rows.get(at).map(|row| row.spot());
        self.selected = at;
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        fingerprint: Option<&str>,
        wall: &DateTime<Tz>,
    ) {
        let config = store.config();
        let rows = self.rows(config);
        let at = self.at(&rows);
        self.spot = rows.get(at).map(|row| row.spot());
        self.selected = at;
        let width = usize::from(area.width);
        let mut lines: Vec<Line> = Vec::new();
        let mut shown = 0..0;
        let mut at = 0;
        let mut row = |lines: &mut Vec<Line<'static>>, spans: Vec<Span<'static>>| {
            let selected = at == self.selected;
            let rail = match selected {
                true => Span::styled("│ ", Style::new().fg(theme::PRIMARY)),
                false => Span::raw("  "),
            };
            if selected {
                shown = lines.len()..lines.len() + 1;
            }
            at += 1;
            lines.push(cut(rail, spans, width, selected));
        };
        let heading = |lines: &mut Vec<Line<'static>>, text: String| {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::styled(text, theme::muted()));
        };
        let note = |lines: &mut Vec<Line<'static>>, text: String, style: Style| {
            lines.push(Line::styled(fatal::cut(&format!("  {text}"), width), style));
        };
        let fatal_style = Style::new().fg(theme::DESTRUCTIVE);
        let mut section = "";
        for key in Key::ALL {
            if key.section() != section {
                section = key.section();
                heading(&mut lines, section.to_owned());
                if !config.has(key) {
                    match &self.region {
                        Some(failure) => note(&mut lines, failed(failure), fatal_style),
                        None => note(&mut lines, "loading".to_owned(), theme::muted()),
                    }
                }
            }
            if config.has(key) {
                row(&mut lines, setting(config, key));
            }
        }
        for (blocks, load) in [(false, &self.muted), (true, &self.blocked)] {
            let (title, undone) = match blocks {
                true => ("blocked", "unblocked"),
                false => ("muted", "unmuted"),
            };
            match load {
                Load::Loading => {
                    heading(&mut lines, title.to_owned());
                    note(&mut lines, "loading".to_owned(), theme::muted());
                }
                Load::Failed(failure) => {
                    heading(&mut lines, title.to_owned());
                    note(&mut lines, failed(failure), fatal_style);
                }
                Load::Ready(Listed { users, .. }) if users.is_empty() => {
                    heading(&mut lines, title.to_owned());
                    note(&mut lines, "nobody".to_owned(), theme::muted());
                }
                Load::Ready(Listed { users, .. }) => {
                    heading(&mut lines, format!("{title} · {}", users.len()));
                    for user in users {
                        let still = match blocks {
                            true => store.blocking(&user.handle),
                            false => store.muting(&user.handle),
                        };
                        row(&mut lines, person(user, (!still).then_some(undone)));
                    }
                }
            }
        }
        match &self.devices {
            Load::Loading => {
                heading(&mut lines, "devices".to_owned());
                note(&mut lines, "loading".to_owned(), theme::muted());
            }
            Load::Failed(failure) => {
                heading(&mut lines, "devices".to_owned());
                note(&mut lines, failed(failure), fatal_style);
            }
            Load::Ready(devices) => {
                heading(&mut lines, format!("devices · {}", devices.len()));
                for device in devices {
                    let this = fingerprint == Some(device.fingerprint.as_str());
                    row(&mut lines, signer(device, this, wall));
                }
            }
        }
        let height = usize::from(area.height);
        if shown.start < self.top {
            // The heading above the first row of a section comes into view with it.
            self.top = shown.start.saturating_sub(1);
        } else if shown.end > self.top + height {
            self.top = shown.end - height;
        }
        if self.selected == 0 {
            self.top = 0;
        }
        let lines: Vec<Line> = lines.into_iter().skip(self.top).take(height).collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}

fn failed(failure: &Failure) -> String {
    match failure {
        Failure::Down => "fatal: the site is in maintenance".to_owned(),
        Failure::Offline(detail) | Failure::Refused(detail) => format!("fatal: {detail}"),
    }
}

/// A row cut to `width`, its first span bold while the cursor is on it.
fn cut(
    rail: Span<'static>,
    spans: Vec<Span<'static>>,
    width: usize,
    selected: bool,
) -> Line<'static> {
    let mut room = width.saturating_sub(rail.width());
    let mut line = vec![rail];
    for (at, span) in spans.into_iter().enumerate() {
        if room == 0 {
            break;
        }
        let text = fatal::cut(&span.content, room);
        room = room.saturating_sub(text.width());
        let style = match (at, selected) {
            (0, true) => span.style.add_modifier(Modifier::BOLD),
            _ => span.style,
        };
        line.push(Span::styled(text, style));
    }
    Line::from(line)
}

fn padded(name: &str) -> String {
    format!("{name:<NAME_WIDTH$}")
}

/// A key and its value, with what it changes beside it.
fn setting(config: Config, key: Key) -> Vec<Span<'static>> {
    let value = Style::new().fg(theme::FOREGROUND);
    let mut spans = vec![Span::styled(padded(key.short()), value)];
    match key {
        Key::Color => match config.color() {
            None => {
                let auto = config.automatic_lane();
                let lane = crate::model::Lane::Palette(auto);
                spans.push(Span::styled("● ", Style::new().fg(theme::lane(lane))));
                spans.push(Span::styled("auto", value));
                spans.push(Span::styled(
                    format!(" · {}", config::lane_name(lane)),
                    theme::muted(),
                ));
            }
            Some(lane) => {
                spans.push(Span::styled("● ", Style::new().fg(theme::lane(lane))));
                spans.push(Span::styled(config::lane_name(lane), value));
            }
        },
        Key::Language => match config.language() {
            None => spans.push(Span::styled("unset", theme::muted())),
            Some(code) => {
                spans.push(Span::styled(code.to_owned(), value));
                let named = (config.languages().iter()).find(|language| language.code == code);
                if let Some(language) = named {
                    spans.push(Span::styled(
                        format!(" · {}", language.name),
                        theme::muted(),
                    ));
                }
            }
        },
        _ => match config.get(key) {
            Some(shown) => spans.push(Span::styled(shown, value)),
            None => spans.push(Span::styled("unset", theme::muted())),
        },
    }
    if config.saving(key) {
        spans.push(Span::styled(" · saving", theme::muted()));
    }
    let value: usize = spans[1..].iter().map(Span::width).sum();
    let pad = VALUE_WIDTH.saturating_sub(value);
    spans.push(Span::styled(
        format!("{:pad$}  {}", "", key.about()),
        theme::muted(),
    ));
    spans
}

/// Someone the user mutes or blocks, marked when it was `undone` here.
fn person(user: &User, undone: Option<&str>) -> Vec<Span<'static>> {
    let handle = format!("@{}", user.handle);
    match undone {
        None => vec![
            Span::styled(padded(&handle), Style::new().fg(theme::FOREGROUND)),
            Span::styled(user.name.clone(), theme::muted()),
        ],
        Some(undone) => vec![
            Span::styled(
                padded(&handle),
                theme::muted().add_modifier(Modifier::CROSSED_OUT),
            ),
            Span::styled(undone.to_owned(), Style::new().fg(theme::AMBER)),
        ],
    }
}

/// The value enter saves while `key` is typed: the match tab moved to, else the closest one to
/// what's typed. Nothing typed unsets it.
pub fn chosen(config: Config, key: Key, text: &str, at: Option<usize>) -> String {
    let candidates = config.candidates(key, text);
    match at {
        Some(at) => candidates.get(at).map(|candidate| candidate.value.clone()),
        None if text.trim().is_empty() => Some(String::new()),
        None => candidates
            .into_iter()
            .next()
            .map(|candidate| candidate.value),
    }
    .unwrap_or_else(|| text.trim().to_owned())
}

/// The matches for what's typed in one row after it, the way vim's wildmenu lists them, scrolled so
/// the one enter takes shows.
pub fn menu(
    config: Config,
    key: Key,
    text: &str,
    at: Option<usize>,
    room: usize,
) -> Vec<Span<'static>> {
    let candidates = config.candidates(key, text);
    if text.trim().is_empty() && at.is_none() {
        let hint = match candidates.is_empty() {
            true => "  enter unsets it",
            false => "  enter unsets it · tab lists them",
        };
        return vec![Span::styled(fatal::cut(hint, room), theme::muted())];
    }
    if candidates.is_empty() {
        let hint = match key {
            Key::Language => "  dev-null.sh doesn't translate into that",
            _ => "  no match here; dev-null.sh checks it",
        };
        return vec![Span::styled(
            fatal::cut(hint, room),
            Style::new().fg(theme::AMBER),
        )];
    }
    let taken = at.unwrap_or(0).min(candidates.len() - 1);
    let gap = 2;
    let widths: Vec<usize> = candidates
        .iter()
        .map(|candidate| candidate.label.width() + gap)
        .collect();
    let room = room.saturating_sub(4);
    let mut start = 0;
    while start < taken && widths[start..=taken].iter().sum::<usize>() > room {
        start += 1;
    }
    let mut spans = vec![Span::styled(
        if start > 0 { "  <" } else { "   " },
        theme::muted(),
    )];
    let mut left = room;
    let mut end = start;
    for (index, candidate) in candidates.iter().enumerate().skip(start) {
        if widths[index] > left && index > start {
            break;
        }
        let label = fatal::cut(&candidate.label, left.saturating_sub(gap));
        left = left.saturating_sub(label.width() + gap);
        spans.push(Span::raw(" ".repeat(gap - 1)));
        spans.push(match index == taken {
            true => Span::styled(label, Style::new().fg(theme::BACKGROUND).bg(theme::PRIMARY)),
            false => Span::styled(label, theme::muted()),
        });
        end = index + 1;
    }
    if end < candidates.len() {
        spans.push(Span::styled(" >", theme::muted()));
    }
    spans
}

/// A device: its name, the app it signs in with, when it was last used and its fingerprint.
fn signer<Tz: TimeZone>(device: &Device, this: bool, wall: &DateTime<Tz>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        padded(&device.name),
        Style::new().fg(theme::FOREGROUND),
    )];
    if this {
        spans.push(Span::styled(
            "this device · ",
            Style::new().fg(theme::PRIMARY),
        ));
    }
    let used = match device.last_used_at {
        Some(at) => format!("used {}", feed::relative_time(at, wall)),
        None => "never used".to_owned(),
    };
    let app = device
        .app_name
        .as_deref()
        .map_or_else(String::new, |app| format!("{app} · "));
    spans.push(Span::styled(
        format!("{app}{used} · {}", device.fingerprint),
        theme::muted(),
    ));
    spans
}
