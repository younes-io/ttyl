use chrono::{DateTime, TimeZone, Utc};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::feed::{self, Entry, Feed, Writer};
use crate::keys::Action;
use crate::model::{Event, Filter, Group, Kind, Shown};
use crate::store::Store;
use crate::theme;

pub const TITLE: &str = "notifications";

#[derive(Default)]
pub struct Inbox {
    filter: Filter,
}

fn index(filter: Filter) -> usize {
    Filter::ALL
        .iter()
        .position(|each| *each == filter)
        .expect("every filter is listed")
}

impl Inbox {
    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn can(&self, action: Action, _store: &Store) -> bool {
        matches!(
            action,
            Action::Back | Action::NextFilter | Action::PreviousFilter
        )
    }

    pub fn act(&mut self, action: Action, _store: &Store) {
        let at = index(self.filter);
        let count = Filter::ALL.len();
        match action {
            Action::NextFilter => self.filter = Filter::ALL[(at + 1) % count],
            Action::PreviousFilter => self.filter = Filter::ALL[(at + count - 1) % count],
            _ => {}
        }
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
