use std::time::Instant;

use chrono::{DateTime, TimeZone, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::api::Failure;
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::Action;
use crate::log::Retry;
use crate::model::{
    CommitId, Event, Filter, Group, GroupPage, Kind, NotificationId, NotificationPage, Shown,
};
use crate::store::Store;
use crate::theme;
use crate::thread::LOADING_NOTE_DELAY;

pub const TITLE: &str = "notifications";

#[derive(Default)]
pub struct Inbox {
    filter: Filter,
    lists: [List; Filter::ALL.len()],
}

/// One filter's list for this run, kept while other filters are shown.
#[derive(Default)]
struct List {
    loaded: Option<Loaded>,
    failure: Option<Failure>,
    asked_at: Option<Instant>,
    selected: usize,
    offset: usize,
}

struct Loaded {
    groups: Vec<Group>,
    read_at: DateTime<Utc>,
    next_cursor: Option<String>,
}

fn index(filter: Filter) -> usize {
    Filter::ALL
        .iter()
        .position(|each| *each == filter)
        .expect("every filter is listed")
}

/// What Enter on a row opens.
pub enum Opens {
    Thread(CommitId),
    Group(Box<Group>),
}

impl Inbox {
    pub fn filter(&self) -> Filter {
        self.filter
    }

    fn list(&self) -> &List {
        &self.lists[index(self.filter)]
    }

    fn list_mut(&mut self) -> &mut List {
        &mut self.lists[index(self.filter)]
    }

    /// A failed reload over rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        self.list().failure()
    }

    /// A single event opens its commit's thread, a group or a follow opens the group list.
    pub fn opens(&self) -> Option<Opens> {
        let group = self.list().selected()?;
        let grouped = group.actors_count > 1 || group.event_count > 1;
        match group.event.kind {
            Kind::LeveledUp => None,
            Kind::Followed => Some(Opens::Group(Box::new(group.clone()))),
            _ if grouped => Some(Opens::Group(Box::new(group.clone()))),
            _ => group.event.commit_id().cloned().map(Opens::Thread),
        }
    }

    pub fn can(&self, action: Action, _store: &Store) -> bool {
        match action {
            Action::Open => self.opens().is_some(),
            Action::Back | Action::NextFilter | Action::PreviousFilter | Action::Refresh => true,
            _ => self.list().can(action),
        }
    }

    pub fn act(&mut self, action: Action, _store: &Store) {
        let at = index(self.filter);
        let count = Filter::ALL.len();
        match action {
            Action::NextFilter => self.filter = Filter::ALL[(at + 1) % count],
            Action::PreviousFilter => self.filter = Filter::ALL[(at + count - 1) % count],
            _ => self.list_mut().act(action),
        }
    }

    /// Notes when the active list was asked for, so its loading line can wait.
    pub fn asked(&mut self, now: Instant) {
        self.list_mut().asked(now);
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        self.list().placeholder_at()
    }

    /// The cursor of the active list's next page, once the selection nears its end.
    pub fn wants_older(&self) -> Option<(Filter, String)> {
        let cursor = self.list().wants_older()?;
        Some((self.filter, cursor))
    }

    pub fn land(&mut self, page: NotificationPage) {
        self.lists[index(page.filter)].land(page.groups, page.read_at, page.next_cursor);
    }

    pub fn append(&mut self, cursor: &str, page: NotificationPage) {
        self.lists[index(page.filter)].append(cursor, page.groups, page.next_cursor);
    }

    pub fn fail(&mut self, filter: Filter, failure: Failure) {
        self.lists[index(filter)].failure = Some(failure);
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        retry: Retry,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let filter = self.filter;
        self.list_mut().draw(frame, area, filter, retry, now, wall);
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

impl List {
    fn groups(&self) -> &[Group] {
        self.loaded
            .as_ref()
            .map_or(&[], |loaded| loaded.groups.as_slice())
    }

    fn selected(&self) -> Option<&Group> {
        self.groups().get(self.selected)
    }

    fn failure(&self) -> Option<&Failure> {
        self.loaded.as_ref()?;
        self.failure.as_ref()
    }

    fn can(&self, action: Action) -> bool {
        matches!(
            action,
            Action::Down | Action::Up | Action::Top | Action::Bottom
        ) && !self.groups().is_empty()
    }

    fn act(&mut self, action: Action) {
        let last = self.groups().len().saturating_sub(1);
        match action {
            Action::Down => self.selected = (self.selected + 1).min(last),
            Action::Up => self.selected = self.selected.saturating_sub(1),
            Action::Top => self.selected = 0,
            Action::Bottom => self.selected = last,
            _ => {}
        }
    }

    fn asked(&mut self, now: Instant) {
        if self.loaded.is_none() && self.asked_at.is_none() {
            self.asked_at = Some(now);
        }
    }

    fn placeholder_at(&self) -> Option<Instant> {
        if self.loaded.is_some() || self.failure.is_some() {
            return None;
        }
        self.asked_at.map(|at| at + LOADING_NOTE_DELAY)
    }

    fn wants_older(&self) -> Option<String> {
        let loaded = self.loaded.as_ref()?;
        let cursor = loaded.next_cursor.as_ref()?;
        let left = loaded.groups.len().saturating_sub(self.selected + 1);
        (left <= AHEAD).then(|| cursor.clone())
    }

    /// A first page replaces the list and keeps the selection on its group.
    fn land(&mut self, groups: Vec<Group>, read_at: DateTime<Utc>, next_cursor: Option<String>) {
        let kept = self.selected().map(|group| group.event.id.clone());
        self.selected = kept
            .and_then(|id| groups.iter().position(|group| group.event.id == id))
            .unwrap_or(0);
        self.failure = None;
        self.loaded = Some(Loaded {
            groups,
            read_at,
            next_cursor,
        });
    }

    /// An older page appends what is new, and ends the list when it adds nothing.
    fn append(&mut self, cursor: &str, groups: Vec<Group>, next_cursor: Option<String>) {
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        if loaded.next_cursor.as_deref() != Some(cursor) {
            return;
        }
        let before = loaded.groups.len();
        for group in groups {
            if !loaded
                .groups
                .iter()
                .any(|each| each.event.id == group.event.id)
            {
                loaded.groups.push(group);
            }
        }
        loaded.next_cursor = next_cursor.filter(|_| loaded.groups.len() > before);
    }

    fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        filter: Filter,
        retry: Retry,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let placeholder = self.placeholder_at();
        let Some(loaded) = &self.loaded else {
            if let Some(failure) = &self.failure {
                frame.render_widget(
                    Paragraph::new(fatal::screen(failure, retry.page)).wrap(Wrap { trim: false }),
                    area,
                );
            } else if placeholder.is_some_and(|at| now >= at) {
                frame.render_widget(Line::styled("loading notifications", theme::muted()), area);
            }
            return;
        };
        let more = loaded.next_cursor.is_some();
        let feed = layout(
            filter,
            &loaded.groups,
            loaded.read_at,
            more,
            area.width,
            wall,
        );
        let tail = retry
            .older
            .map(|retry| feed::older_failed("notifications", retry));
        let total = feed.lines.len() + tail.as_ref().map_or(0, |tail| tail.len());
        self.selected = self.selected.min(loaded.groups.len().saturating_sub(1));
        let selected = (!loaded.groups.is_empty()).then_some(self.selected);
        let height = usize::from(area.height);
        self.offset = feed::scroll(&feed, total, self.selected, self.offset, height);
        let end = (self.offset + height).min(total);
        let shown = self.offset.min(feed.lines.len())..end.min(feed.lines.len());
        let mut lines = feed.window(selected, shown);
        if let Some(tail) = tail
            && end > feed.lines.len()
        {
            let from = self.offset.saturating_sub(feed.lines.len());
            lines.extend_from_slice(&tail[from..end - feed.lines.len()]);
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// The events behind one group, opened from the list: one row per event.
pub struct GroupList {
    head: Group,
    filter: Filter,
    list: List,
}

impl GroupList {
    pub fn new(head: Group, filter: Filter, now: Instant) -> Self {
        let mut list = List::default();
        list.asked(now);
        GroupList { head, filter, list }
    }

    pub fn id(&self) -> &NotificationId {
        &self.head.event.id
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn failure(&self) -> Option<&Failure> {
        self.list.failure()
    }

    /// The selected event's commit; people in a follow group open nothing until profiles exist.
    pub fn opens(&self) -> Option<&CommitId> {
        let event = &self.list.selected()?.event;
        (event.kind != Kind::Followed)
            .then(|| event.commit_id())
            .flatten()
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.opens().is_some(),
            Action::Back | Action::Refresh => true,
            _ => self.list.can(action),
        }
    }

    pub fn act(&mut self, action: Action) {
        self.list.act(action);
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        self.list.placeholder_at()
    }

    /// The next page's cursor near the end, until an older page fails and R starts over.
    pub fn wants_older(&self) -> Option<String> {
        self.list
            .failure
            .is_none()
            .then(|| self.list.wants_older())?
    }

    pub fn land(&mut self, cursor: Option<&str>, page: GroupPage) {
        let groups = page.events.into_iter().map(Group::single).collect();
        match cursor {
            None => self
                .list
                .land(groups, DateTime::<Utc>::MAX_UTC, page.next_cursor),
            Some(cursor) => self.list.append(cursor, groups, page.next_cursor),
        }
    }

    pub fn fail(&mut self, failure: Failure) {
        self.list.failure = Some(failure);
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let retry = Retry {
            page: None,
            older: None,
        };
        self.list.draw(frame, area, self.filter, retry, now, wall);
    }

    /// The group's own sentence under the top line, where the inbox shows its filters.
    pub fn banner(&self) -> Line<'static> {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(sentence(&self.head, false));
        Line::from(spans)
    }
}

/// The web's empty copy for each filter.
fn empty_copy(filter: Filter) -> &'static str {
    match filter {
        Filter::All => {
            "Nothing here yet. Follows, stars, replies, quotes, mentions, level-ups, and moderation updates for your commits will show up here."
        }
        Filter::Replies => {
            "No replies yet. When someone replies to one of your commits, it shows up here."
        }
        Filter::Quotes => {
            "No quotes yet. When someone quotes one of your commits, it shows up here."
        }
        Filter::Stars => "No stars yet. When someone stars one of your commits, it shows up here.",
        Filter::Mentions => {
            "No mentions yet. When someone mentions you in a commit, it shows up here."
        }
        Filter::Follows => "No followers yet. When someone follows you, it shows up here.",
    }
}

/// Lays groups out newest first, one node each, the way the log lays out commits.
pub fn layout<Tz: TimeZone>(
    filter: Filter,
    groups: &[Group],
    read_at: DateTime<Utc>,
    more: bool,
    width: u16,
    now: &DateTime<Tz>,
) -> Feed {
    let mut out = Writer::new(width);
    if groups.is_empty() {
        let mut lines = vec![vec![Span::styled(
            "$ cat /dev/notifications",
            Style::new().fg(theme::FOREGROUND),
        )]];
        feed::wrap(
            &[(empty_copy(filter).to_owned(), theme::muted())],
            usize::from(width),
            &mut lines,
        );
        return Feed {
            lines: lines.into_iter().map(Line::from).collect(),
            entries: Vec::new(),
        };
    }

    let mut entries = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            out.gap();
        }
        let header = out.lines.len();
        let event = &group.event;
        let unread = event.read_at.is_none_or(|at| at >= read_at);
        let time = feed::relative_time(event.created_at, now);
        let room = out.content().saturating_sub(time.len() + 1);
        let mut spans = feed::fit(sentence(group, unread), room);
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(Span::raw(" ".repeat(out.content() - used - time.len())));
        spans.push(Span::styled(time, theme::muted()));
        let lane = Style::new().fg(theme::lane(event.actor.lane));
        let node = if unread {
            Span::styled("● ", lane.add_modifier(Modifier::BOLD))
        } else {
            Span::styled("○ ", lane)
        };
        out.line(node, spans);
        if let Some(detail) = detail(event) {
            out.line(feed::rail(), feed::fit(detail, out.content()));
        }
        entries.push(Entry {
            lines: header..out.lines.len(),
            header,
            lane: event.actor.lane,
            indent: 0,
        });
    }
    if !more {
        let lane = groups.last().expect("groups is not empty").event.actor.lane;
        out.gap();
        out.node(
            Span::styled("○ ", Style::new().fg(theme::lane(lane))),
            Span::raw("  "),
            vec![Span::styled(
                "That's the oldest notification. There's nothing before it.",
                theme::muted(),
            )],
        );
    }
    Feed {
        lines: out.lines,
        entries,
    }
}

/// The web's sentence for a group: who, and what they did, or what happened to your commit.
fn sentence(group: &Group, unread: bool) -> Vec<Span<'static>> {
    let event = &group.event;
    let text = if unread {
        Style::new()
            .fg(theme::FOREGROUND)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme::FOREGROUND)
    };
    let action = match event.kind {
        Kind::Approved => return vec![Span::styled("Your commit was approved.", text)],
        Kind::Rejected => return vec![Span::styled("Your commit was rejected.", text)],
        Kind::LeveledUp => return vec![Span::styled("You leveled up.", text)],
        Kind::Followed => "followed you",
        Kind::Replied => "replied to your commit",
        Kind::Quoted => "quoted your commit",
        Kind::Starred => "starred your commit",
        Kind::Mentioned => "mentioned you",
    };
    let actor = &event.actor;
    let mut spans = vec![Span::styled(
        actor.name.clone(),
        Style::new()
            .fg(theme::lane(actor.lane))
            .add_modifier(Modifier::BOLD),
    )];
    let grouped = group.actors_count > 1 || group.event_count > 1;
    let others = match event.kind {
        Kind::Followed | Kind::Starred => group.actors_count.saturating_sub(1),
        _ => 0,
    };
    if !grouped {
        spans.push(Span::styled(
            format!(" @{}", actor.username),
            theme::muted(),
        ));
    } else if others > 0 {
        let noun = if others == 1 { "other" } else { "others" };
        spans.push(Span::styled(format!(" and {others} {noun}"), text));
    }
    spans.push(Span::styled(format!(" {action}"), text));
    if others == 0 && group.event_count > 1 {
        spans.push(Span::styled(
            format!(" · {} updates", group.event_count),
            theme::muted(),
        ));
    }
    spans
}

/// The dim line under the sentence: the rejection reason, or the commit's first line.
fn detail(event: &Event) -> Option<Vec<Span<'static>>> {
    if let (Kind::Rejected, Some(reason)) = (event.kind, &event.reason) {
        return Some(vec![Span::styled(
            format!("reason: {reason}"),
            theme::muted(),
        )]);
    }
    match &event.commit {
        Some(Shown::Live(commit)) if event.kind != Kind::Followed => {
            Some(feed::one_line(&commit.body_segments, theme::muted()))
        }
        _ => None,
    }
}
