use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Wrap};

use crate::api::{Call, Failure};
use crate::clock::Timers;
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::{self, Action, Link};
use crate::log;
use crate::model::{Circle, CommitId, Log, LogPage, Person, Profile, Row, User};
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
    expanded: HashSet<CommitId>,
    folded: Vec<bool>,
    /// Their pinned commit, put first by the profile rather than by the server.
    pinned: Option<CommitId>,
}

impl Paged {
    fn items(&self) -> &[Item] {
        self.loaded
            .as_ref()
            .map_or(&[], |(items, _)| items.as_slice())
    }

    fn can(&self, action: Action) -> bool {
        match action {
            Action::Down | Action::Up | Action::Top | Action::Bottom => !self.items().is_empty(),
            Action::Expand => {
                self.folded.get(self.selected) == Some(&true)
                    || self.commit().is_some_and(|id| self.expanded.contains(id))
            }
            _ => false,
        }
    }

    fn commit(&self) -> Option<&CommitId> {
        match self.items().get(self.selected)? {
            Item::Commit(id) => Some(id),
            Item::Person(_) => None,
        }
    }

    fn act(&mut self, action: Action) {
        let last = self.items().len().saturating_sub(1);
        match action {
            Action::Down => self.selected = (self.selected + 1).min(last),
            Action::Up => self.selected = self.selected.saturating_sub(1),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            Action::Expand => {
                if let Some(id) = self.commit().cloned()
                    && !self.expanded.remove(&id)
                {
                    self.expanded.insert(id);
                }
            }
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
            self.pinned = None;
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
    /// Where the selected card was last drawn, for a box about it to open beside.
    card: Option<Rect>,
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
            card: None,
        }
    }

    /// Where the selected card was last drawn.
    pub fn card(&self) -> Option<Rect> {
        self.card
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
        self.pin();
    }

    /// Puts their pinned commit first in their commits, once both have come, so every commit key works on it.
    fn pin(&mut self) {
        let pinned = self
            .profile
            .as_ref()
            .and_then(|profile| profile.pinned.as_ref());
        let pinned = pinned.map(|row| row.commit.id.clone());
        let list = &mut self.lists[Tab::Commits.index()];
        let Some((items, _)) = &mut list.loaded else {
            return;
        };
        if list.pinned == pinned {
            return;
        }
        let kept = items
            .get(list.selected)
            .filter(|_| list.selected > 0)
            .cloned();
        if list.pinned.take().is_some() {
            items.remove(0);
        }
        if let Some(id) = pinned {
            items.retain(|item| !matches!(item, Item::Commit(each) if *each == id));
            items.insert(0, Item::Commit(id.clone()));
            list.pinned = Some(id);
        }
        list.selected = kept
            .and_then(|kept| items.iter().position(|item| item.same(&kept)))
            .unwrap_or(0);
    }

    pub fn fail_header(&mut self, failure: Failure) {
        self.failure = Some(failure);
    }

    pub fn land(&mut self, tab: Tab, cursor: Option<&str>, items: Vec<Item>, next: Option<String>) {
        self.lists[tab.index()].land(cursor, items, next);
        if tab == Tab::Commits {
            self.pin();
        }
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

    /// Where `link` goes for the person this profile is about, when they have one.
    pub fn link(&self, link: Link) -> Option<String> {
        let person = &self.profile.as_ref()?.person;
        let given = |url: &Option<String>| url.clone().filter(|url| !url.is_empty());
        match link {
            Link::Website => given(&person.website).map(|site| {
                if site.contains("://") {
                    site
                } else {
                    format!("https://{site}")
                }
            }),
            Link::GitHub => given(&person.github_url),
            Link::Page => Some(person.user.html_url.clone()),
        }
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.opens().is_some(),
            Action::Link(link) => self.link(link).is_some(),
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

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        timers: &Timers,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        self.card = None;
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
        // One frame holds the header and the list, with the tabs as the rule between them.
        let edge = Style::new().fg(log::edge(true));
        let panel = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(edge);
        let inside = panel.inner(area);
        frame.render_widget(panel, area);
        let links: Vec<(String, String)> = [Link::Website, Link::GitHub, Link::Page]
            .into_iter()
            .filter_map(|link| {
                let key = keys::key_for(Action::Link(link))?;
                Some((self.link(link)?, key))
            })
            .collect();
        let header = header(profile, &links, store, inside.width.saturating_sub(2));
        let height = (header.len() as u16 + 2).min(inside.height.saturating_sub(8));
        let [top, rule, list] = Layout::vertical([
            Constraint::Length(height),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inside);
        frame.render_widget(
            Paragraph::new(header).block(Block::new().padding(Padding::new(1, 1, 1, 1))),
            top,
        );
        let names = Tab::ALL.map(Tab::slug);
        let active = Tab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        let tabs = log::tabs(&names, active, true, usize::from(rule.width));
        let fill = usize::from(rule.width).saturating_sub(tabs.width());
        let mut spans = vec![Span::styled("├", edge)];
        spans.extend(tabs.spans);
        spans.push(Span::styled("─".repeat(fill), edge));
        spans.push(Span::styled("┤", edge));
        frame.render_widget(
            Line::from(spans),
            Rect {
                x: area.x,
                width: area.width,
                ..rule
            },
        );
        let area = list.inner(Margin::new(1, 0));

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
        let (feed, folded) = if ids.is_empty() {
            (people(items, store, area.width), Vec::new())
        } else {
            let mut rows = store.rows_of(&ids);
            if list.pinned.is_some() {
                let rest = rows.split_off(1.min(rows.len()));
                pinned_first(rows, rest, area.width, &list.expanded, wall)
            } else {
                feed::layout(&page(rows, false), area.width, &list.expanded, true, wall)
            }
        };
        list.folded = folded;
        list.selected = list.selected.min(feed.entries.len().saturating_sub(1));
        let selected = Some(list.selected);
        let total = feed.lines.len();
        let height = usize::from(area.height);
        list.offset = feed::scroll(&feed, total, list.selected, list.offset, height);
        let shown = list.offset.min(total)..(list.offset + height).min(total);
        let rows = feed.card_rows(selected, &shown);
        let at = list.selected;
        self.card = feed::card_area(area, &rows);
        let lines = feed.window(selected, shown, area.width);
        frame.render_widget(Paragraph::new(&lines[..]), area);
        let Some(id) = ids.get(at) else {
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

/// Their commits as a page laid out on its own.
fn page(rows: Vec<Row>, is_ranked: bool) -> LogPage {
    LogPage {
        log: Log::Latest,
        rows,
        next_cursor: Some(String::new()),
        is_ranked,
        new_commits_count: None,
        suggestions: Vec::new(),
    }
}

/// Their pinned commit under its own label in place of a day, then the rest by day.
fn pinned_first<Tz: TimeZone>(
    pinned: Vec<Row>,
    rest: Vec<Row>,
    width: u16,
    expanded: &HashSet<CommitId>,
    wall: &DateTime<Tz>,
) -> (Feed, Vec<bool>) {
    let mut out = Writer::new(width);
    let label = Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD);
    out.push(
        Span::styled("┆ ", theme::muted()),
        vec![Span::styled("pinned", label)],
    );
    out.gap();
    let mut entries = Vec::new();
    let mut folded = Vec::new();
    for (rows, is_ranked) in [(pinned, true), (rest, false)] {
        if rows.is_empty() {
            continue;
        }
        if !entries.is_empty() {
            out.gap();
        }
        let (feed, cut) = feed::layout(&page(rows, is_ranked), width, expanded, true, wall);
        let shift = out.lines.len();
        entries.extend(feed.entries.into_iter().map(|entry| Entry {
            lines: entry.lines.start + shift..entry.lines.end + shift,
            header: entry.header + shift,
            ..entry
        }));
        out.lines.extend(feed.lines);
        folded.extend(cut);
    }
    (
        Feed {
            lines: out.lines,
            entries,
        },
        folded,
    )
}

/// A link as people write it, without its scheme.
fn bare(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

/// Who they are and their standing with you, what they say about themselves, where they are,
/// where to find them with the key that opens each, and their counts.
fn header(
    profile: &Profile,
    links: &[(String, String)],
    store: &Store,
    width: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let person = &profile.person;
    let user = &person.user;
    let faint = Style::new().fg(theme::FAINT);
    let quiet = Style::new().fg(theme::MUTED_FOREGROUND);

    let mut chips = Vec::new();
    if user.username == store.me.username {
        chips.push(Span::styled(" you ", quiet.bg(theme::MUTED)));
    } else if store
        .following(&user.username)
        .unwrap_or(profile.is_following)
    {
        chips.push(Span::styled(
            " following ",
            Style::new()
                .fg(theme::BACKGROUND)
                .bg(theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if profile.follows_you {
        if !chips.is_empty() {
            chips.push(Span::raw(" "));
        }
        chips.push(Span::styled(" follows you ", quiet.bg(theme::MUTED)));
    }
    let mut name = vec![Span::styled(
        user.name.clone(),
        Style::new()
            .fg(theme::lane(user.lane))
            .add_modifier(Modifier::BOLD),
    )];
    if user.name != user.username {
        name.push(Span::styled(format!("  @{}", user.username), quiet));
    }

    let given = |fact: &Option<String>| fact.clone().filter(|fact| !fact.is_empty());
    let mut facts = Vec::new();
    if let Some(company) = given(&person.company) {
        facts.push(vec![Span::styled(company, quiet)]);
    }
    if let Some(location) = given(&person.location) {
        facts.push(vec![Span::styled(location, quiet)]);
    }
    facts.push(vec![Span::styled(
        format!("Joined {}", person.created_at.format("%B %Y")),
        quiet,
    )]);

    let links: Vec<Vec<Span<'static>>> = links
        .iter()
        .map(|(url, key)| {
            vec![
                Span::styled(format!("{} ", theme::icons().link), faint),
                Span::styled(bare(url), Style::new().fg(theme::PRIMARY)),
                Span::styled(format!(" {key}"), quiet),
            ]
        })
        .collect();

    let counts: Vec<(String, &str)> = [
        (person.commits_count, "commits"),
        (person.followers_count, "followers"),
        (person.following_count, "following"),
    ]
    .into_iter()
    .filter_map(|(count, noun)| Some((grouped(count?), noun)))
    .collect();
    let bold = Style::new()
        .fg(theme::FOREGROUND)
        .add_modifier(Modifier::BOLD);
    let bio = |width: usize, lines: &mut Vec<Vec<Span<'static>>>| {
        if let Some(bio) = person.bio.as_ref().filter(|bio| !bio.is_empty()) {
            lines.push(Vec::new());
            feed::wrap(
                &[(bio.clone(), Style::new().fg(theme::FOREGROUND))],
                width,
                lines,
            );
        }
    };

    let mut lines = Vec::new();
    if width >= SIDE * 2 + GUTTER {
        // Who they are on the left, their standing and counts in a column on the right.
        let left = width - SIDE - GUTTER;
        let mut body = vec![feed::fit(name, left)];
        bio(left, &mut body);
        body.push(Vec::new());
        body.extend(flow(facts, faint, left));
        body.extend(flow(links, faint, left));
        let mut side = vec![right(chips, SIDE), Vec::new()];
        let digits = counts.iter().map(|(count, _)| count.len()).max();
        for (count, noun) in &counts {
            let pad = digits.unwrap_or(0) - count.len();
            side.push(right(
                vec![
                    Span::styled(format!("{}{count}", " ".repeat(pad)), bold),
                    Span::styled(format!("  {noun:<9}"), quiet),
                ],
                SIDE,
            ));
        }
        for row in 0..body.len().max(side.len()) {
            let mut line = body.get(row).cloned().unwrap_or_default();
            let used = Line::from(line.clone()).width();
            line.push(Span::raw(" ".repeat(left + GUTTER - used)));
            line.extend(side.get(row).cloned().unwrap_or_default());
            lines.push(line);
        }
    } else {
        lines.push(feed::fit(name, width));
        lines.push(right(chips, width));
        bio(width, &mut lines);
        lines.push(Vec::new());
        lines.extend(flow(facts, faint, width));
        lines.extend(flow(links, faint, width));
        let counts = counts
            .into_iter()
            .map(|(count, noun)| {
                vec![
                    Span::styled(count, bold),
                    Span::styled(format!(" {noun}"), quiet),
                ]
            })
            .collect();
        lines.extend(flow(counts, faint, width));
    }

    lines.into_iter().map(Line::from).collect()
}

/// The width of the header's right column, which fits both chips side by side.
const SIDE: usize = 26;
const GUTTER: usize = 4;

/// Spans pushed to the right edge of `width`.
fn right(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let used = Line::from(spans.clone()).width();
    let mut out = vec![Span::raw(" ".repeat(width.saturating_sub(used)))];
    out.extend(spans);
    out
}

/// Groups of spans laid along lines of `width`, with a faint dot between groups on a line.
fn flow(groups: Vec<Vec<Span<'static>>>, dot: Style, width: usize) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut used = 0;
    for group in groups {
        let size = Line::from(group.clone()).width();
        match lines.last_mut() {
            Some(line) if used + 5 + size <= width => {
                line.push(Span::styled("  ·  ", dot));
                line.extend(group);
                used += 5 + size;
            }
            _ => {
                lines.push(feed::fit(group, width));
                used = size;
            }
        }
    }
    lines
}

/// A count with thousands separators, as in 5,271.
fn grouped(count: u32) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
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
