use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, Datelike, TimeZone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Wrap};

use crate::api::{Call, Failure};
use crate::clock::Timers;
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::{self, Action, Link};
use crate::log;
use crate::model::{
    Activity, Circle, CommitId, Fold, Headline, Lane, Level, Log, LogPage, Organization,
    OrganizationPage, PALETTE_LEN, Person, Profile, Row, Scale, Streaks, Who,
};
use crate::store::Store;
use crate::theme;
use crate::thread::LOADING_NOTE_DELAY;

/// The lists under a profile's header. A person's are in the order Tab walks them; an
/// organization has its commits and the commits that mention it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Commits,
    Replies,
    Followers,
    Following,
    Mentions,
}

const PERSON_TABS: &[Tab] = &[Tab::Commits, Tab::Replies, Tab::Followers, Tab::Following];
const ORGANIZATION_TABS: &[Tab] = &[Tab::Commits, Tab::Mentions];
/// Every tab, one list each.
const TABS: usize = 5;

impl Tab {
    pub fn slug(self) -> &'static str {
        match self {
            Tab::Commits => "commits",
            Tab::Replies => "replies",
            Tab::Followers => "followers",
            Tab::Following => "following",
            Tab::Mentions => "mentions",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The tab's first page, or the next one from `cursor`. A first page of who someone follows
    /// also asks for the organizations they follow, which go above the people.
    fn calls(self, who: &Who, cursor: Option<String>) -> Vec<Call> {
        let first = cursor.is_none();
        let handle = who.handle().to_owned();
        let call = match (who, self) {
            (Who::Organization(_), Tab::Commits | Tab::Mentions) => Call::OrganizationCommits {
                handle: handle.clone(),
                mentions: self == Tab::Mentions,
                cursor,
            },
            (_, Tab::Commits | Tab::Replies | Tab::Mentions) => Call::Commits {
                handle: handle.clone(),
                replies: self == Tab::Replies,
                cursor,
            },
            (_, Tab::Followers | Tab::Following) => Call::People {
                handle: handle.clone(),
                circle: if self == Tab::Followers {
                    Circle::Followers
                } else {
                    Circle::Following
                },
                cursor,
            },
        };
        let mut calls = vec![call];
        if first && self == Tab::Following {
            calls.push(Call::FollowedOrganizations {
                handle,
                cursor: None,
            });
        }
        calls
    }

    /// Whose tab a list call fills, which tab, and the cursor it asked from.
    pub fn of(call: &Call) -> Option<(&str, Tab, Option<&str>)> {
        match call {
            Call::Commits {
                handle,
                replies,
                cursor,
            } => {
                let tab = if *replies { Tab::Replies } else { Tab::Commits };
                Some((handle, tab, cursor.as_deref()))
            }
            Call::OrganizationCommits {
                handle,
                mentions,
                cursor,
            } => {
                let tab = if *mentions {
                    Tab::Mentions
                } else {
                    Tab::Commits
                };
                Some((handle, tab, cursor.as_deref()))
            }
            Call::People {
                handle,
                circle,
                cursor,
            } => {
                let tab = match circle {
                    Circle::Followers => Tab::Followers,
                    Circle::Following => Tab::Following,
                };
                Some((handle, tab, cursor.as_deref()))
            }
            _ => None,
        }
    }

    fn empty_copy(self, handle: &str) -> String {
        match self {
            Tab::Commits => format!("@{handle} has no commits yet."),
            Tab::Replies => format!("@{handle} has no replies yet."),
            Tab::Followers => format!("@{handle} has no followers yet."),
            Tab::Following => format!("@{handle} isn't following anyone yet."),
            Tab::Mentions => format!("Nobody has mentioned @{handle} yet."),
        }
    }
}

#[derive(Clone)]
pub enum Item {
    Commit(CommitId),
    Person(Box<Person>),
    Organization(Box<Organization>),
    Headline(Box<Headline>),
}

impl Item {
    fn same(&self, other: &Item) -> bool {
        match (self, other) {
            (Item::Commit(a), Item::Commit(b)) => a == b,
            (Item::Person(a), Item::Person(b)) => a.user.id == b.user.id,
            (Item::Organization(a), Item::Organization(b)) => a.handle == b.handle,
            (Item::Headline(a), Item::Headline(b)) => a.id == b.id,
            _ => false,
        }
    }

    /// What Enter on the row opens.
    pub fn opens(&self) -> Opens {
        match self {
            Item::Commit(id) => Opens::Thread(id.clone()),
            Item::Person(person) => Opens::Profile(Who::Person(person.user.handle.clone())),
            Item::Organization(organization) => {
                Opens::Profile(Who::Organization(organization.handle.clone()))
            }
            Item::Headline(headline) => Opens::Headline(headline.clone()),
        }
    }
}

/// What Enter on a row opens.
pub enum Opens {
    Thread(CommitId),
    Profile(Who),
    /// The headline's commits, under its recap.
    Headline(Box<Headline>),
}

/// One tab's list for this visit, kept while other tabs are shown.
#[derive(Default)]
pub struct Paged {
    loaded: Option<(Vec<Item>, Option<String>)>,
    failure: Option<Failure>,
    asked_at: Option<Instant>,
    selected: usize,
    offset: usize,
    reading: feed::Reading,
    expanded: HashSet<CommitId>,
    folded: Vec<bool>,
    /// Their pinned commit, put first by the profile rather than by the server.
    pinned: Option<CommitId>,
    /// How many items lead with the organizations someone follows, which page apart from the people.
    organizations: usize,
}

impl Paged {
    pub fn items(&self) -> &[Item] {
        self.loaded
            .as_ref()
            .map_or(&[], |(items, _)| items.as_slice())
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Down | Action::Up | Action::Top | Action::Bottom => !self.items().is_empty(),
            Action::Expand => {
                self.folded.get(self.selected) == Some(&true)
                    || self.commit().is_some_and(|id| self.expanded.contains(id))
            }
            _ => false,
        }
    }

    pub fn selected(&self) -> Option<&Item> {
        self.items().get(self.selected)
    }

    pub fn commit(&self) -> Option<&CommitId> {
        match self.selected()? {
            Item::Commit(id) => Some(id),
            Item::Person(_) | Item::Organization(_) | Item::Headline(_) => None,
        }
    }

    pub fn act(&mut self, action: Action) {
        let last = self.items().len().saturating_sub(1);
        match action {
            Action::Down | Action::Up | Action::Top | Action::Bottom => {
                self.selected = self.reading.step(action, self.selected, last);
            }
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

    pub fn fail(&mut self, failure: Failure) {
        self.failure = Some(failure);
    }

    /// A failed reload over rows still shown.
    pub fn stale(&self) -> Option<&Failure> {
        self.loaded.as_ref().and(self.failure.as_ref())
    }

    /// Clears what failed so a reload shows the loading line again where nothing is shown.
    pub fn reload(&mut self, now: Instant) {
        self.failure = None;
        if self.loaded.is_none() {
            self.asked_at = Some(now);
        }
    }

    /// Notes the ask, and says whether this was the list's first.
    pub fn asked(&mut self, now: Instant) -> bool {
        let first = self.loaded.is_none() && self.asked_at.is_none();
        if first {
            self.asked_at = Some(now);
        }
        first
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        if self.loaded.is_some() || self.failure.is_some() {
            return None;
        }
        self.asked_at.map(|at| at + LOADING_NOTE_DELAY)
    }

    pub fn wants_older(&self) -> Option<String> {
        if self.failure.is_some() {
            return None;
        }
        let (items, cursor) = self.loaded.as_ref()?;
        let left = items.len().saturating_sub(self.selected + 1);
        (left <= AHEAD).then(|| cursor.clone())?
    }

    pub fn land(&mut self, cursor: Option<&str>, items: Vec<Item>, next_cursor: Option<String>) {
        self.failure = None;
        let Some(cursor) = cursor else {
            self.pinned = None;
            let kept = self.selected().cloned();
            let mut all = self.items()[..self.organizations].to_vec();
            all.extend(items);
            self.selected = kept
                .and_then(|kept| all.iter().position(|item| item.same(&kept)))
                .unwrap_or(0);
            self.loaded = Some((all, next_cursor));
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

    /// Puts the organizations someone follows above the people: a first page in place of the
    /// ones listed, a later one after them.
    pub fn land_organizations(&mut self, first: bool, organizations: Vec<Organization>) {
        let kept = self.selected().cloned();
        let (items, _) = self.loaded.get_or_insert_with(Default::default);
        if first {
            items.drain(..self.organizations);
            self.organizations = 0;
        }
        for organization in organizations {
            let item = Item::Organization(Box::new(organization));
            if !items.iter().any(|each| each.same(&item)) {
                items.insert(self.organizations, item);
                self.organizations += 1;
            }
        }
        self.selected = kept
            .and_then(|kept| items.iter().position(|item| item.same(&kept)))
            .unwrap_or(0);
    }

    /// Draws the list, and says where its selected card went.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        store: &Store,
        timers: &Timers,
        now: Instant,
        wall: &DateTime<Tz>,
        loading: &str,
        empty: &str,
        is_ranked: bool,
    ) -> Option<Rect> {
        let Some((items, _)) = &self.loaded else {
            if let Some(failure) = &self.failure {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, None)).wrap(Wrap { trim: false }),
                    area,
                );
            } else if self.placeholder_at().is_some_and(|at| now >= at) {
                frame.render_widget(Line::styled(loading.to_owned(), theme::muted()), area);
            }
            return None;
        };
        if items.is_empty() {
            let mut lines = Vec::new();
            feed::wrap(
                &[(empty.to_owned(), theme::muted())],
                usize::from(area.width),
                &mut lines,
            );
            let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
            frame.render_widget(Paragraph::new(lines), area);
            return None;
        }
        let ids: Vec<CommitId> = items
            .iter()
            .filter_map(|item| match item {
                Item::Commit(id) => Some(id.clone()),
                Item::Person(_) | Item::Organization(_) | Item::Headline(_) => None,
            })
            .collect();
        let (feed, folded) = if ids.is_empty() {
            (nodes(items, store, area.width), Vec::new())
        } else {
            let mut rows = store.rows_of(&ids);
            if self.pinned.is_some() {
                let rest = rows.split_off(1.min(rows.len()));
                pinned_first(rows, rest, area.width, &self.expanded, wall)
            } else {
                let page = page(rows, is_ranked);
                feed::layout(&page, area.width, &self.expanded, true, None, wall)
            }
        };
        self.folded = folded;
        self.selected = self.selected.min(feed.entries.len().saturating_sub(1));
        let selected = Some(self.selected);
        let total = feed.lines.len();
        let height = usize::from(area.height);
        self.offset = feed::scroll(
            &feed,
            total,
            self.selected,
            self.offset,
            height,
            &mut self.reading,
        );
        let shown = self.offset.min(total)..(self.offset + height).min(total);
        let rows = feed.card_rows(selected, &shown);
        let card = feed::card_area(area, &rows);
        let lines = feed.window(selected, shown, area.width);
        frame.render_widget(Paragraph::new(&lines[..]), area);
        self.reading.hint(frame.buffer_mut(), area);
        if let Some(id) = ids.get(self.selected) {
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
        card
    }
}

/// Whose page a profile is, and their header once it has come.
enum Header {
    Person(Option<Box<Profile>>),
    Organization(Option<Box<OrganizationPage>>),
}

impl Header {
    fn loaded(&self) -> bool {
        match self {
            Header::Person(profile) => profile.is_some(),
            Header::Organization(page) => page.is_some(),
        }
    }
}

/// Someone's profile: their header over the tab shown, each tab paged on its own.
pub struct ProfileScreen {
    handle: String,
    header: Header,
    failure: Option<Failure>,
    asked_at: Instant,
    tab: Tab,
    lists: [Paged; TABS],
    /// Where the selected card was last drawn, for a box about it to open beside.
    card: Option<Rect>,
}

impl ProfileScreen {
    pub fn new(who: Who, now: Instant) -> Self {
        let mut lists: [Paged; TABS] = Default::default();
        lists[Tab::Commits.index()].asked(now);
        let (handle, header) = match who {
            Who::Person(handle) => (handle, Header::Person(None)),
            Who::Organization(handle) => (handle, Header::Organization(None)),
        };
        ProfileScreen {
            handle,
            header,
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

    pub fn handle(&self) -> &str {
        &self.handle
    }

    pub fn who(&self) -> Who {
        match self.header {
            Header::Person(_) => Who::Person(self.handle.clone()),
            Header::Organization(_) => Who::Organization(self.handle.clone()),
        }
    }

    fn tabs(&self) -> &'static [Tab] {
        match self.header {
            Header::Person(_) => PERSON_TABS,
            Header::Organization(_) => ORGANIZATION_TABS,
        }
    }

    fn list(&self) -> &Paged {
        &self.lists[self.tab.index()]
    }

    fn list_mut(&mut self) -> &mut Paged {
        &mut self.lists[self.tab.index()]
    }

    /// The header, then the first page of the tab shown.
    pub fn calls(&self) -> Vec<Call> {
        let header = match self.header {
            Header::Person(_) => Call::User(self.handle.clone()),
            Header::Organization(_) => Call::Organization(self.handle.clone()),
        };
        let mut calls = vec![header];
        calls.extend(self.tab_calls());
        calls
    }

    /// The first page of the tab shown.
    pub fn tab_calls(&self) -> Vec<Call> {
        self.tab.calls(&self.who(), None)
    }

    pub fn set_profile(&mut self, profile: Profile) {
        if let Header::Person(header) = &mut self.header {
            self.failure = None;
            *header = Some(Box::new(profile));
        }
    }

    pub fn set_organization(&mut self, page: Box<OrganizationPage>) {
        if let Header::Organization(header) = &mut self.header {
            self.failure = None;
            *header = Some(page);
        }
    }

    /// Puts the organizations they follow above the people they follow.
    pub fn land_organizations(&mut self, first: bool, organizations: Vec<Organization>) {
        self.lists[Tab::Following.index()].land_organizations(first, organizations);
    }

    /// Puts their pinned commit first in their commits, as the store shows it now, so every
    /// commit key works on it and a pin made anywhere moves it at once, even one this profile
    /// hasn't listed.
    fn pin(&mut self, store: &Store) {
        let Header::Person(profile) = &self.header else {
            return;
        };
        let header = (profile.as_ref())
            .and_then(|profile| profile.pinned.as_ref())
            .map(|row| &row.commit.id);
        let list = &mut self.lists[Tab::Commits.index()];
        let Some((items, next)) = &mut list.loaded else {
            return;
        };
        let listed = items.iter().filter_map(|item| match item {
            Item::Commit(id) => Some(id),
            Item::Person(_) | Item::Organization(_) | Item::Headline(_) => None,
        });
        let pinned = (header.into_iter().chain(listed))
            .find(|id| store.pinned(id))
            .or_else(|| store.pinned_by(&self.handle))
            .cloned();
        if list.pinned == pinned {
            return;
        }
        // The selection follows the commit it is on, unless it sat on top of a list a pin just
        // came to, where it stays on top.
        let kept = items
            .get(list.selected)
            .filter(|_| list.selected > 0 || list.pinned.is_some())
            .cloned();
        if let Some(old) = list.pinned.take() {
            items.remove(0);
            // dev-null.sh leaves the pinned commit out of the list, so an unpinned one goes back
            // where its date puts it, or comes with a later page.
            let created = |id: &CommitId| store.row(id).map(|row| row.commit.created_at);
            let at = items.iter().position(|item| match item {
                Item::Commit(id) => created(id) < created(&old),
                Item::Person(_) | Item::Organization(_) | Item::Headline(_) => false,
            });
            match at {
                Some(at) => items.insert(at, Item::Commit(old)),
                None if next.is_none() => items.push(Item::Commit(old)),
                None => {}
            }
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
    }

    pub fn fail_tab(&mut self, tab: Tab, failure: Failure) {
        self.lists[tab.index()].fail(failure);
    }

    /// A failed reload over a header or rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        let header = self.failure.as_ref().filter(|_| self.header.loaded());
        header.or(self.list().stale())
    }

    pub fn people_tab(&self) -> bool {
        matches!(self.tab, Tab::Followers | Tab::Following)
    }

    pub fn selected(&self) -> Option<&Item> {
        self.list().items().get(self.list().selected)
    }

    pub fn opens(&self) -> Option<Opens> {
        self.selected().map(Item::opens)
    }

    /// Where `link` goes for whom this profile is about, when they have one.
    pub fn link(&self, link: Link) -> Option<String> {
        let (website, github, page) = match &self.header {
            Header::Person(profile) => {
                let person = &profile.as_ref()?.person;
                (
                    &person.website,
                    person.github_url.as_ref(),
                    &person.user.html_url,
                )
            }
            Header::Organization(page) => {
                let organization = &page.as_ref()?.organization;
                (
                    &organization.website,
                    Some(&organization.github_url),
                    &organization.html_url,
                )
            }
        };
        let given = |url: Option<&String>| url.filter(|url| !url.is_empty()).cloned();
        match link {
            Link::Website => given(website.as_ref()).map(|site| {
                if site.contains("://") {
                    site
                } else {
                    format!("https://{site}")
                }
            }),
            Link::GitHub => given(github),
            Link::Page => Some(page.clone()),
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
        let tabs = self.tabs();
        let at = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let count = tabs.len();
        match action {
            Action::NextFilter => self.tab = tabs[(at + 1) % count],
            Action::PreviousFilter => self.tab = tabs[(at + count - 1) % count],
            _ => {
                self.list_mut().act(action);
                return false;
            }
        }
        self.list_mut().asked(now)
    }

    /// Clears what failed so a reload shows the loading line again where nothing is shown.
    pub fn reload(&mut self, now: Instant) {
        if !self.header.loaded() {
            self.failure = None;
            self.asked_at = now;
        }
        self.list_mut().reload(now);
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        if !self.header.loaded() && self.failure.is_none() {
            return Some(self.asked_at + LOADING_NOTE_DELAY);
        }
        self.list().placeholder_at()
    }

    /// The next page of the tab shown, once the selection nears its end.
    pub fn wants_older(&self) -> Option<Call> {
        let cursor = self.list().wants_older()?;
        self.tab.calls(&self.who(), Some(cursor)).pop()
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
        self.pin(store);
        if !self.header.loaded() {
            if let Some(failure) = &self.failure {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, None)).wrap(Wrap { trim: false }),
                    area,
                );
            } else if now >= self.asked_at + LOADING_NOTE_DELAY {
                let loading = format!("loading @{}", self.handle);
                frame.render_widget(Line::styled(loading, theme::muted()), area);
            }
            return;
        }
        // The tabs are the rule between the header and the list, and on a wide screen they join
        // the box that holds both.
        let edge = Style::new().fg(log::edge(true));
        let framed = log::boxes(area);
        let inside = if framed {
            let panel = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(edge);
            let inside = panel.inner(area);
            frame.render_widget(panel, area);
            inside
        } else {
            area
        };
        let pad = u16::from(framed);
        let links: Vec<(String, String)> = [Link::Website, Link::GitHub, Link::Page]
            .into_iter()
            .filter_map(|link| {
                let key = keys::key_for(Action::Link(link))?;
                Some((self.link(link)?, key))
            })
            .collect();
        // The graph goes first when the header would leave the list too little room.
        let room = usize::from(inside.height.saturating_sub(8));
        let width = inside.width.saturating_sub(2 * pad);
        let lines = |graph: bool| match &self.header {
            Header::Person(Some(profile)) => header(profile, &links, store, width, graph),
            Header::Organization(Some(page)) => organization_header(page, &links, store, width),
            Header::Person(None) | Header::Organization(None) => Vec::new(),
        };
        let mut header = lines(true);
        if header.len() + 2 > room {
            header = lines(false);
        }
        let height = (header.len() as u16 + 2).min(room as u16);
        let [top, rule, list] = Layout::vertical([
            Constraint::Length(height),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inside);
        frame.render_widget(
            Paragraph::new(header).block(Block::new().padding(Padding::new(pad, pad, 1, 1))),
            top,
        );
        let tabs = self.tabs();
        let names: Vec<&str> = tabs.iter().map(|tab| tab.slug()).collect();
        let active = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let tabs = log::tabs(&names, active, true, usize::from(rule.width));
        let fill = usize::from(rule.width).saturating_sub(tabs.width());
        let (open, close) = if framed { ("├", "┤") } else { ("", "") };
        let mut spans = vec![Span::styled(open, edge)];
        spans.extend(tabs.spans);
        spans.push(Span::styled("─".repeat(fill), edge));
        spans.push(Span::styled(close, edge));
        frame.render_widget(
            Line::from(spans),
            Rect {
                x: area.x,
                width: area.width,
                ..rule
            },
        );
        let area = list.inner(Margin::new(pad, 0));

        let loading = format!("loading {}", self.tab.slug());
        let empty = self.tab.empty_copy(&self.handle);
        let list = &mut self.lists[self.tab.index()];
        self.card = list.draw(
            frame, area, store, timers, now, wall, &loading, &empty, false,
        );
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
        let (feed, cut) = feed::layout(&page(rows, is_ranked), width, expanded, true, None, wall);
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

/// What a profile's header shows: who they are, what they say about themselves, where they are
/// and where to find them on the left; their standing with you, counts and rank on the right.
struct Sides {
    name: Vec<Span<'static>>,
    chips: Vec<Span<'static>>,
    bio: Option<String>,
    facts: Vec<Vec<Span<'static>>>,
    links: Vec<Vec<Span<'static>>>,
    counts: Vec<(String, &'static str)>,
    standing: Option<Vec<Vec<Span<'static>>>>,
}

fn quiet() -> Style {
    Style::new().fg(theme::MUTED_FOREGROUND)
}

fn faint() -> Style {
    Style::new().fg(theme::FAINT)
}

/// A name in its lane, with the handle after it when the two differ.
fn named(name: &str, handle: &str, lane: Lane) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        name.to_owned(),
        Style::new()
            .fg(theme::lane(lane))
            .add_modifier(Modifier::BOLD),
    )];
    if name != handle {
        spans.push(Span::styled(format!("  @{handle}"), quiet()));
    }
    spans
}

fn following_chip() -> Span<'static> {
    Span::styled(
        " following ",
        Style::new()
            .fg(theme::BACKGROUND)
            .bg(theme::PRIMARY)
            .add_modifier(Modifier::BOLD),
    )
}

/// Where to find them, with the key that opens each.
fn link_groups(links: &[(String, String)]) -> Vec<Vec<Span<'static>>> {
    links
        .iter()
        .map(|(url, key)| {
            vec![
                Span::styled(format!("{} ", theme::icons().link), faint()),
                Span::styled(bare(url), Style::new().fg(theme::PRIMARY)),
                Span::styled(format!(" {key}"), quiet()),
            ]
        })
        .collect()
}

/// Lays the sides out in two columns, or one under the other when the screen is narrow.
fn columns(sides: Sides, width: usize) -> Vec<Vec<Span<'static>>> {
    let bold = Style::new()
        .fg(theme::FOREGROUND)
        .add_modifier(Modifier::BOLD);
    let bio = |width: usize, lines: &mut Vec<Vec<Span<'static>>>| {
        if let Some(bio) = sides.bio.as_ref().filter(|bio| !bio.is_empty()) {
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
        let left = width - SIDE - GUTTER;
        let mut body = vec![feed::fit(sides.name, left)];
        bio(left, &mut body);
        body.push(Vec::new());
        body.extend(flow(sides.facts, faint(), left));
        body.extend(flow(sides.links, faint(), left));
        let mut side = vec![right(sides.chips, SIDE), Vec::new()];
        let digits = sides.counts.iter().map(|(count, _)| count.len()).max();
        for (count, noun) in &sides.counts {
            let pad = digits.unwrap_or(0) - count.len();
            side.push(right(
                vec![
                    Span::styled(format!("{}{count}", " ".repeat(pad)), bold),
                    Span::styled(format!("  {noun:<9}"), quiet()),
                ],
                SIDE,
            ));
        }
        if let Some(standing) = sides.standing {
            side.push(Vec::new());
            side.extend(standing.into_iter().map(|line| right(line, SIDE)));
        }
        for row in 0..body.len().max(side.len()) {
            let mut line = body.get(row).cloned().unwrap_or_default();
            let used = Line::from(line.clone()).width();
            line.push(Span::raw(" ".repeat(left + GUTTER - used)));
            line.extend(side.get(row).cloned().unwrap_or_default());
            lines.push(line);
        }
    } else {
        lines.push(feed::fit(sides.name, width));
        lines.push(right(sides.chips, width));
        bio(width, &mut lines);
        lines.push(Vec::new());
        lines.extend(flow(sides.facts, faint(), width));
        lines.extend(flow(sides.links, faint(), width));
        let counts = (sides.counts.into_iter())
            .map(|(count, noun)| {
                vec![
                    Span::styled(count, bold),
                    Span::styled(format!(" {noun}"), quiet()),
                ]
            })
            .collect();
        lines.extend(flow(counts, faint(), width));
        if let Some(standing) = sides.standing {
            lines.push(Vec::new());
            lines.extend(standing);
        }
    }
    lines
}

/// Who they are and their standing with you, what they say about themselves, where they are,
/// where to find them with the key that opens each, and their counts.
fn header(
    profile: &Profile,
    links: &[(String, String)],
    store: &Store,
    width: u16,
    graph: bool,
) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let person = &profile.person;
    let user = &person.user;
    let quiet = quiet();

    let mut chips = Vec::new();
    if user.handle == store.me.handle {
        chips.push(Span::styled(" you ", quiet.bg(theme::MUTED)));
    } else if store.following(&user.who()).unwrap_or(profile.is_following) {
        chips.push(following_chip());
    }
    if store.follows_you(&user.handle, profile.follows_you) {
        if !chips.is_empty() {
            chips.push(Span::raw(" "));
        }
        chips.push(Span::styled(" follows you ", quiet.bg(theme::MUTED)));
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

    let counts = [
        (person.commits_count, "commits"),
        (person.followers_count, "followers"),
        (person.following_count, "following"),
    ]
    .into_iter()
    .filter_map(|(count, noun)| Some((grouped(count?), noun)))
    .collect();
    let lane = theme::lane(user.lane);
    let standing = profile.level.as_ref().map(|level| {
        let mine = user.handle == store.me.handle;
        standing(level, mine, lane)
    });
    let sides = Sides {
        name: named(&user.name, &user.handle, user.lane),
        chips,
        bio: person.bio.clone(),
        facts,
        links: link_groups(links),
        counts,
        standing,
    };
    let mut lines = columns(sides, width);
    if graph && let Some(activity) = &profile.activity {
        lines.push(Vec::new());
        lines.extend(self::activity(activity, profile.streaks, lane, width));
    }

    lines.into_iter().map(Line::from).collect()
}

/// An organization's name and whether you follow it, what it says about itself, where it is,
/// who runs it on dev-null.sh, where to find it, and how many follow it.
fn organization_header(
    page: &OrganizationPage,
    links: &[(String, String)],
    store: &Store,
    width: u16,
) -> Vec<Line<'static>> {
    let organization = &page.organization;
    let quiet = quiet();
    let who = Who::Organization(organization.handle.clone());
    let mut chips = vec![Span::styled(" org ", quiet.bg(theme::MUTED))];
    if store.following(&who).unwrap_or(page.is_following) {
        chips.push(Span::raw(" "));
        chips.push(following_chip());
    }
    let mut facts = Vec::new();
    if let Some(location) = organization.location.clone().filter(|at| !at.is_empty()) {
        facts.push(vec![Span::styled(location, quiet)]);
    }
    if !organization.admins.is_empty() {
        let mut admins = vec![Span::styled("admins", quiet)];
        for admin in &organization.admins {
            admins.push(Span::styled(
                format!(" @{}", admin.handle),
                Style::new().fg(theme::lane(admin.lane)),
            ));
        }
        facts.push(admins);
    }
    let counts = (organization.followers_count)
        .map(|count| (grouped(count), "followers"))
        .into_iter()
        .collect();
    let sides = Sides {
        name: named(&organization.name, &organization.handle, organization.lane),
        chips,
        bio: organization.bio.clone(),
        facts,
        links: link_groups(links),
        counts,
        standing: None,
    };
    (columns(sides, usize::from(width)).into_iter())
        .map(Line::from)
        .collect()
}

/// Their level and rank, and on your own profile how far you are from the next rank.
fn standing(level: &Level, mine: bool, lane: Color) -> Vec<Vec<Span<'static>>> {
    let quiet = Style::new().fg(theme::MUTED_FOREGROUND);
    let mut lines = vec![vec![
        Span::styled(
            format!("Lv. {}", level.level),
            Style::new().fg(lane).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", level.rank), quiet),
    ]];
    if mine {
        let xp = match level.next_level_xp {
            Some(next) => format!("  {}/{} xp", grouped(level.xp), grouped(next)),
            None => format!("  {} xp", grouped(level.xp)),
        };
        let filled = usize::from(level.level_progress.min(100)).div_ceil(10);
        lines.push(vec![
            Span::styled("▰".repeat(filled), Style::new().fg(lane)),
            Span::styled("▱".repeat(10 - filled), Style::new().fg(theme::RAIL)),
            Span::styled(xp, quiet),
        ]);
    }
    lines
}

/// Their commits over the graph's window as bars two rows tall, a year a week to a bar and
/// shorter windows a bucket to a bar, then the total and their streaks.
fn activity(
    activity: &Activity,
    streaks: Option<Streaks>,
    lane: Color,
    width: usize,
) -> Vec<Vec<Span<'static>>> {
    const BARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let quiet = Style::new().fg(theme::MUTED_FOREGROUND);
    let bold = Style::new()
        .fg(theme::FOREGROUND)
        .add_modifier(Modifier::BOLD);
    let total: u32 = activity.counts.iter().sum();

    let mut lines = Vec::new();
    if total > 0 || activity.scale == Scale::Year {
        let (bars, cell) = if activity.scale == Scale::Year {
            let skip = activity.first.weekday().num_days_from_sunday() as usize;
            let mut weeks = vec![0; (skip + activity.counts.len()).div_ceil(7)];
            for (day, count) in activity.counts.iter().enumerate() {
                weeks[(skip + day) / 7] += count;
            }
            (weeks, 1)
        } else {
            (activity.counts.clone(), 3)
        };
        let bars = &bars[bars.len().saturating_sub(width / cell)..];
        let peak = bars.iter().copied().max().unwrap_or(0).max(1);
        for row in [1, 0] {
            lines.push(
                bars.iter()
                    .map(|&count| {
                        let height = (count * 16).div_ceil(peak);
                        let part = height.saturating_sub(row * 8).min(8) as usize;
                        let (bar, color) = if count == 0 && row == 0 {
                            ("▁", theme::MUTED)
                        } else {
                            (BARS[part], lane)
                        };
                        let bar = format!("{bar:<cell$}", bar = bar.repeat(cell.min(2)));
                        Span::styled(bar, Style::new().fg(color))
                    })
                    .collect(),
            );
        }
        lines.push(Vec::new());
    }

    let window = match activity.scale {
        Scale::Day => "in the last 24 hours",
        Scale::Week => "in the last 7 days",
        Scale::Month => "in the last 30 days",
        Scale::Year => "in the last year",
    };
    let mut summary = vec![if total == 0 && activity.scale != Scale::Year {
        vec![Span::styled("No commits yet", quiet)]
    } else {
        let noun = if total == 1 { "commit" } else { "commits" };
        vec![
            Span::styled(grouped(total), bold),
            Span::styled(format!(" {noun} {window}"), quiet),
        ]
    }];
    if let Some(streaks) = streaks.filter(|streaks| streaks.longest > 0) {
        let days = if streaks.longest == 1 { "day" } else { "days" };
        summary.push(vec![
            Span::styled("longest streak ", quiet),
            Span::styled(streaks.longest.to_string(), bold),
            Span::styled(format!(" {days}"), quiet),
        ]);
        if streaks.current > 0 {
            summary.push(vec![
                Span::styled("● ", Style::new().fg(lane)),
                Span::styled("current ", quiet),
                Span::styled(streaks.current.to_string(), bold),
            ]);
        }
    }
    lines.extend(flow(summary, Style::new().fg(theme::FAINT), width));
    lines
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

/// One node per person or organization, the way the inbox lays out a follow group, or per
/// headline: its name over its recap.
fn nodes(items: &[Item], store: &Store, width: u16) -> Feed {
    let mut out = Writer::new(width);
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.gap();
        }
        let header = out.lines.len();
        let lane = match item {
            Item::Commit(_) => continue,
            Item::Person(person) => person_node(&mut out, person, store),
            Item::Organization(organization) => organization_node(&mut out, organization, store),
            Item::Headline(headline) => {
                let lane = Lane::Palette(index as u8 % PALETTE_LEN);
                headline_node(&mut out, headline, lane);
                lane
            }
        };
        entries.push(Entry {
            lines: header..out.lines.len(),
            header,
            lane,
            indent: 0,
        });
    }
    Feed {
        lines: out.lines,
        entries,
    }
}

/// An organization as a following list shows it, marked apart from the people under it.
fn organization_node(out: &mut Writer, organization: &Organization, store: &Store) -> Lane {
    let lane = Style::new().fg(theme::lane(organization.lane));
    let mut spans = vec![
        Span::styled(organization.name.clone(), lane.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" @{} · org", organization.handle), theme::muted()),
    ];
    let who = Who::Organization(organization.handle.clone());
    if (store.following(&who).or(organization.is_followed)).unwrap_or(false) {
        spans.push(Span::styled(" · following", theme::muted()));
    }
    out.line(Span::styled("◇ ", lane), feed::fit(spans, out.content()));
    if let Some(bio) = organization.bio.as_ref().filter(|bio| !bio.is_empty()) {
        let bio = vec![Span::styled(bio.replace('\n', " "), theme::muted())];
        out.line(feed::rail(), feed::fit(bio, out.content()));
    }
    organization.lane
}

fn person_node(out: &mut Writer, person: &Person, store: &Store) -> Lane {
    let user = &person.user;
    if store.blocking(&user.handle) {
        out.node(
            Span::styled("○ ", Style::new().fg(theme::RAIL)),
            feed::rail(),
            vec![Span::styled(
                feed::folded(Fold::Blocked, &user.handle),
                theme::muted(),
            )],
        );
        return user.lane;
    }
    let lane = Style::new().fg(theme::lane(user.lane));
    let mut spans = vec![
        Span::styled(user.name.clone(), lane.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" @{}", user.handle), theme::muted()),
    ];
    if store.follows_you(&user.handle, person.follows_you == Some(true)) {
        spans.push(Span::styled(" · follows you", theme::muted()));
    }
    if store
        .following(&user.who())
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
    user.lane
}

fn headline_node(out: &mut Writer, headline: &Headline, lane: Lane) {
    let color = Style::new().fg(theme::lane(lane));
    let name = Span::styled(headline.name.clone(), color.add_modifier(Modifier::BOLD));
    out.node(Span::styled("● ", color), feed::rail(), vec![name]);
    if let Some(summary) = &headline.summary {
        out.push(
            feed::rail(),
            vec![Span::styled(summary.clone(), theme::muted())],
        );
    }
}
