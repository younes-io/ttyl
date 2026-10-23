use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::api::{Call, Failure};
use crate::clock::Timers;
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::Action;
use crate::model::{Circle, CommitId, Log, LogPage, Person, Profile, User};
use crate::store::Store;
use crate::theme;
use crate::thread::LOADING_NOTE_DELAY;

/// The lists under a profile's header, in the order Tab walks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Commits,
    Replies,
    Followers,
    Following,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Commits, Tab::Replies, Tab::Followers, Tab::Following];

    pub fn slug(self) -> &'static str {
        match self {
            Tab::Commits => "commits",
            Tab::Replies => "replies",
            Tab::Followers => "followers",
            Tab::Following => "following",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn call(self, username: &str, cursor: Option<String>) -> Call {
        let username = username.to_owned();
        match self {
            Tab::Commits | Tab::Replies => Call::Commits {
                username,
                replies: self == Tab::Replies,
                cursor,
            },
            Tab::Followers | Tab::Following => Call::People {
                username,
                circle: if self == Tab::Followers {
                    Circle::Followers
                } else {
                    Circle::Following
                },
                cursor,
            },
        }
    }

    /// Whose tab a list call fills, which tab, and the cursor it asked from.
    pub fn of(call: &Call) -> Option<(&str, Tab, Option<&str>)> {
        match call {
            Call::Commits {
                username,
                replies,
                cursor,
            } => {
                let tab = if *replies { Tab::Replies } else { Tab::Commits };
                Some((username, tab, cursor.as_deref()))
            }
            Call::People {
                username,
                circle,
                cursor,
            } => {
                let tab = match circle {
                    Circle::Followers => Tab::Followers,
                    Circle::Following => Tab::Following,
                };
                Some((username, tab, cursor.as_deref()))
            }
            _ => None,
        }
    }

    fn empty_copy(self, username: &str) -> String {
        match self {
            Tab::Commits => format!("@{username} has no commits yet."),
            Tab::Replies => format!("@{username} has no replies yet."),
            Tab::Followers => format!("@{username} has no followers yet."),
            Tab::Following => format!("@{username} isn't following anyone yet."),
        }
    }
}

#[derive(Clone)]
pub enum Item {
    Commit(CommitId),
    Person(Box<Person>),
}

impl Item {
    fn same(&self, other: &Item) -> bool {
        match (self, other) {
            (Item::Commit(a), Item::Commit(b)) => a == b,
            (Item::Person(a), Item::Person(b)) => a.user.id == b.user.id,
            _ => false,
        }
    }
}

/// What Enter on a row opens.
pub enum Opens {
    Thread(CommitId),
    Profile(String),
}

/// One tab's list for this visit, kept while other tabs are shown.
#[derive(Default)]
struct Paged {
    loaded: Option<(Vec<Item>, Option<String>)>,
    failure: Option<Failure>,
    asked_at: Option<Instant>,
    selected: usize,
    offset: usize,
}

impl Paged {
    fn items(&self) -> &[Item] {
        self.loaded
            .as_ref()
            .map_or(&[], |(items, _)| items.as_slice())
    }

    fn can(&self, action: Action) -> bool {
        matches!(
            action,
            Action::Down | Action::Up | Action::Top | Action::Bottom
        ) && !self.items().is_empty()
    }

    fn act(&mut self, action: Action) {
        let last = self.items().len().saturating_sub(1);
        match action {
            Action::Down => self.selected = (self.selected + 1).min(last),
            Action::Up => self.selected = self.selected.saturating_sub(1),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            _ => {}
        }
    }

    /// Notes the ask, and says whether this was the list's first.
    fn asked(&mut self, now: Instant) -> bool {
        let first = self.loaded.is_none() && self.asked_at.is_none();
        if first {
            self.asked_at = Some(now);
        }
        first
    }

    fn placeholder_at(&self) -> Option<Instant> {
        if self.loaded.is_some() || self.failure.is_some() {
            return None;
        }
        self.asked_at.map(|at| at + LOADING_NOTE_DELAY)
    }

    fn wants_older(&self) -> Option<String> {
        if self.failure.is_some() {
            return None;
        }
        let (items, cursor) = self.loaded.as_ref()?;
        let left = items.len().saturating_sub(self.selected + 1);
        (left <= AHEAD).then(|| cursor.clone())?
    }

    fn land(&mut self, cursor: Option<&str>, items: Vec<Item>, next_cursor: Option<String>) {
        self.failure = None;
        let Some(cursor) = cursor else {
            let kept = self.items().get(self.selected);
            self.selected = kept
                .and_then(|kept| items.iter().position(|item| item.same(kept)))
                .unwrap_or(0);
            self.loaded = Some((items, next_cursor));
            return;
        };
        let Some((loaded, next)) = &mut self.loaded else {
            return;
        };
        if next.as_deref() != Some(cursor) {
            return;
        }
        let before = loaded.len();
        for item in items {
            if !loaded.iter().any(|each| each.same(&item)) {
                loaded.push(item);
            }
        }
        *next = next_cursor.filter(|_| loaded.len() > before);
    }
}

/// Someone's profile: their header over the tab shown, each tab paged on its own.
pub struct ProfileScreen {
    username: String,
    profile: Option<Profile>,
    failure: Option<Failure>,
    asked_at: Instant,
    tab: Tab,
    lists: [Paged; Tab::ALL.len()],
}

impl ProfileScreen {
    pub fn new(username: String, now: Instant) -> Self {
        let mut lists: [Paged; Tab::ALL.len()] = Default::default();
        lists[Tab::Commits.index()].asked(now);
        ProfileScreen {
            username,
            profile: None,
            failure: None,
            asked_at: now,
            tab: Tab::Commits,
            lists,
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    fn list(&self) -> &Paged {
        &self.lists[self.tab.index()]
    }

    fn list_mut(&mut self) -> &mut Paged {
        &mut self.lists[self.tab.index()]
    }

    /// The first page of the tab shown.
    pub fn tab_call(&self) -> Call {
        self.tab.call(&self.username, None)
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.failure = None;
        self.profile = Some(profile);
    }

    pub fn fail_header(&mut self, failure: Failure) {
        self.failure = Some(failure);
    }

    pub fn land(&mut self, tab: Tab, cursor: Option<&str>, items: Vec<Item>, next: Option<String>) {
        self.lists[tab.index()].land(cursor, items, next);
    }

    pub fn fail_tab(&mut self, tab: Tab, failure: Failure) {
        self.lists[tab.index()].failure = Some(failure);
    }

    /// A failed reload over a header or rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        let header = self.profile.as_ref().and(self.failure.as_ref());
        let list = self.list();
        header.or(list.loaded.as_ref().and(list.failure.as_ref()))
    }

    pub fn people_tab(&self) -> bool {
        matches!(self.tab, Tab::Followers | Tab::Following)
    }

    pub fn selected(&self) -> Option<&Item> {
        self.list().items().get(self.list().selected)
    }

    /// The person this profile is about, once their header has come.
    pub fn user(&self) -> Option<&User> {
        self.profile.as_ref().map(|profile| &profile.person.user)
    }

    pub fn opens(&self) -> Option<Opens> {
        match self.selected()? {
            Item::Commit(id) => Some(Opens::Thread(id.clone())),
            Item::Person(person) => Some(Opens::Profile(person.user.username.clone())),
        }
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.opens().is_some(),
            Action::Back | Action::NextFilter | Action::PreviousFilter | Action::Refresh => true,
            _ => self.list().can(action),
        }
    }

    /// Moves within the tab or to another, and says whether the tab it lands on needs its first page.
    pub fn act(&mut self, action: Action, now: Instant) -> bool {
        let at = self.tab.index();
        let count = Tab::ALL.len();
        match action {
            Action::NextFilter => self.tab = Tab::ALL[(at + 1) % count],
            Action::PreviousFilter => self.tab = Tab::ALL[(at + count - 1) % count],
            _ => {
                self.list_mut().act(action);
                return false;
            }
        }
        self.list_mut().asked(now)
    }

    /// Clears what failed so R shows the loading line again where nothing is shown.
    pub fn reload(&mut self, now: Instant) {
        if self.profile.is_none() {
            self.failure = None;
            self.asked_at = now;
        }
        let list = self.list_mut();
        list.failure = None;
        if list.loaded.is_none() {
            list.asked_at = Some(now);
        }
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        if self.profile.is_none() && self.failure.is_none() {
            return Some(self.asked_at + LOADING_NOTE_DELAY);
        }
        self.list().placeholder_at()
    }

    /// The next page of the tab shown, once the selection nears its end.
    pub fn wants_older(&self) -> Option<Call> {
        let cursor = self.list().wants_older()?;
        Some(self.tab.call(&self.username, Some(cursor)))
    }

    /// The tabs under the top line, the shown one bold, with single gaps when narrow.
    pub fn tab_bar(&self, width: u16) -> Line<'static> {
        let wide: usize = Tab::ALL.iter().map(|each| each.slug().len() + 2).sum();
        let gap = if wide <= usize::from(width) + 1 {
            "  "
        } else {
            " "
        };
        let spans = Tab::ALL.iter().enumerate().flat_map(|(at, tab)| {
            let lead = if at == 0 { " " } else { gap };
            let style = if *tab == self.tab {
                Style::new()
                    .fg(theme::FOREGROUND)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::muted()
            };
            [Span::raw(lead), Span::styled(tab.slug(), style)]
        });
        Line::from(spans.collect::<Vec<_>>())
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        timers: &Timers,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let Some(profile) = &self.profile else {
            if let Some(failure) = &self.failure {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, None)).wrap(Wrap { trim: false }),
                    area,
                );
            } else if now >= self.asked_at + LOADING_NOTE_DELAY {
                let loading = format!("loading @{}", self.username);
                frame.render_widget(Line::styled(loading, theme::muted()), area);
            }
            return;
        };
        let header = header(profile, store, area.width);
        let height = (header.len() as u16).min(area.height / 2);
        let [top, area] =
            Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area);
        frame.render_widget(Paragraph::new(header), top);

        let tab = self.tab;
        let username = self.username.clone();
        let placeholder = self.list().placeholder_at();
        let list = self.list_mut();
        let Some((items, _)) = &list.loaded else {
            if let Some(failure) = &list.failure {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, None)).wrap(Wrap { trim: false }),
                    area,
                );
            } else if placeholder.is_some_and(|at| now >= at) {
                let loading = format!("loading {}", tab.slug());
                frame.render_widget(Line::styled(loading, theme::muted()), area);
            }
            return;
        };
        if items.is_empty() {
            let mut lines = Vec::new();
            feed::wrap(
                &[(tab.empty_copy(&username), theme::muted())],
                usize::from(area.width),
                &mut lines,
            );
            let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        let ids: Vec<CommitId> = items
            .iter()
            .filter_map(|item| match item {
                Item::Commit(id) => Some(id.clone()),
                Item::Person(_) => None,
            })
            .collect();
        let feed = if ids.is_empty() {
            people(items, store, area.width)
        } else {
            let page = LogPage {
                log: Log::Latest,
                rows: store.rows_of(&ids),
                next_cursor: Some(String::new()),
                is_ranked: false,
                new_commits_count: None,
                suggestions: Vec::new(),
            };
            feed::layout(&page, area.width, &HashSet::new(), true, wall).0
        };
        list.selected = list.selected.min(feed.entries.len().saturating_sub(1));
        let selected = Some(list.selected);
        let total = feed.lines.len();
        let height = usize::from(area.height);
        list.offset = feed::scroll(&feed, total, list.selected, list.offset, height);
        let shown = list.offset.min(total)..(list.offset + height).min(total);
        let rows = feed.card_rows(selected, &shown);
        let lines = feed.window(selected, shown, area.width);
        frame.render_widget(Paragraph::new(&lines[..]), area);
        let Some(id) = ids.get(list.selected) else {
            return;
        };
        for (spark, elapsed) in timers.sparks(id, now) {
            feed::spark(
                frame.buffer_mut(),
                area,
                &lines,
                rows.clone(),
                spark,
                elapsed,
            );
        }
    }
}

/// Who they are, what they say about themselves, their counts, and their pinned commit.
fn header(profile: &Profile, store: &Store, width: u16) -> Vec<Line<'static>> {
    let person = &profile.person;
    let user = &person.user;
    let dot = || Span::styled(" · ", theme::muted());
    let mut name = vec![
        Span::styled(
            user.name.clone(),
            Style::new()
                .fg(theme::lane(user.lane))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" @{}", user.username), theme::muted()),
    ];
    if profile.follows_you {
        name.extend([dot(), Span::styled("follows you", theme::muted())]);
    }
    if user.username != store.me.username
        && store
            .following(&user.username)
            .unwrap_or(profile.is_following)
    {
        name.extend([
            dot(),
            Span::styled("following", Style::new().fg(theme::PRIMARY)),
        ]);
    }
    let mut lines = vec![feed::fit(name, usize::from(width))];
    if let Some(bio) = person.bio.as_ref().filter(|bio| !bio.is_empty()) {
        feed::wrap(
            &[(bio.clone(), Style::new().fg(theme::FOREGROUND))],
            usize::from(width),
            &mut lines,
        );
    }
    let joined = format!("Joined {}", person.created_at.format("%B %Y"));
    let facts = [&person.company, &person.location, &person.website]
        .into_iter()
        .flatten()
        .filter(|fact| !fact.is_empty())
        .cloned()
        .chain([joined]);
    lines.push(feed::fit(joined_by_dots(facts), usize::from(width)));
    let counts = [
        (person.commits_count, "commits"),
        (person.followers_count, "followers"),
        (person.following_count, "following"),
    ]
    .into_iter()
    .filter_map(|(count, noun)| Some(format!("{} {noun}", count?)));
    lines.push(joined_by_dots(counts));
    if let Some(pinned) = &profile.pinned {
        let mut spans = vec![
            Span::styled("pinned ", theme::muted()),
            Span::styled(
                pinned.commit.short_hash.clone(),
                Style::new().fg(theme::PRIMARY),
            ),
            Span::raw(" "),
        ];
        spans.extend(feed::one_line(&pinned.commit.body_segments, theme::muted()));
        lines.push(feed::fit(spans, usize::from(width)));
    }
    lines.push(Vec::new());
    lines.into_iter().map(Line::from).collect()
}

fn joined_by_dots(parts: impl Iterator<Item = String>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for part in parts {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme::muted()));
        }
        spans.push(Span::styled(part, theme::muted()));
    }
    spans
}

/// One node per person, the way the inbox lays out a follow group.
fn people(items: &[Item], store: &Store, width: u16) -> Feed {
    let mut out = Writer::new(width);
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Item::Person(person) = item else {
            continue;
        };
        if index > 0 {
            out.gap();
        }
        let header = out.lines.len();
        let user = &person.user;
        let lane = Style::new().fg(theme::lane(user.lane));
        let mut spans = vec![
            Span::styled(user.name.clone(), lane.add_modifier(Modifier::BOLD)),
            Span::styled(format!(" @{}", user.username), theme::muted()),
        ];
        if person.follows_you == Some(true) {
            spans.push(Span::styled(" · follows you", theme::muted()));
        }
        if store
            .following(&user.username)
            .or(person.is_followed)
            .unwrap_or(false)
        {
            spans.push(Span::styled(" · following", theme::muted()));
        }
        out.line(Span::styled("○ ", lane), feed::fit(spans, out.content()));
        if let Some(bio) = person.bio.as_ref().filter(|bio| !bio.is_empty()) {
            let bio = vec![Span::styled(bio.replace('\n', " "), theme::muted())];
            out.line(feed::rail(), feed::fit(bio, out.content()));
        }
        entries.push(Entry {
            lines: header..out.lines.len(),
            header,
            lane: user.lane,
            indent: 0,
        });
    }
    Feed {
        lines: out.lines,
        entries,
    }
}
