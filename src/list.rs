use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::api::{Call, Failure};
use crate::clock::Timers;
use crate::feed;
use crate::keys::{self, Act, Action};
use crate::model::{CommitId, Headline, HeadlineId};
use crate::profile::{Item, Opens, Paged};
use crate::store::Store;
use crate::theme;

/// Where a list's items come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Your starred commits, newest star first.
    Starred,
    /// Commits nearest in meaning to the words, closest first.
    Commits(String),
    /// People whose name or handle matches.
    People(String),
    /// The few commits nearest in meaning to this one.
    Related(CommitId),
    /// The topics dev-null.sh is talking about right now, hottest first.
    Headlines,
    /// One headline's commits from the past week, newest first.
    Headline(HeadlineId),
}

impl Source {
    /// What `/` finds for `query`: people when it starts with `@`, commits otherwise.
    pub fn search(query: &str) -> Source {
        match query.strip_prefix('@') {
            Some(name) => Source::People(name.to_owned()),
            None => Source::Commits(query.to_owned()),
        }
    }

    /// The list a call fills, with the cursor it continues from.
    pub fn of(call: &Call) -> Option<(Source, Option<&str>)> {
        match call {
            Call::Starred { cursor } => Some((Source::Starred, cursor.as_deref())),
            Call::SearchCommits(query) => Some((Source::Commits(query.clone()), None)),
            Call::SearchPeople(name) => Some((Source::People(name.clone()), None)),
            Call::Related(id) => Some((Source::Related(id.clone()), None)),
            Call::Headlines => Some((Source::Headlines, None)),
            Call::Headline(id) => Some((Source::Headline(id.clone()), None)),
            _ => None,
        }
    }

    fn call(&self, cursor: Option<String>) -> Call {
        match self {
            Source::Starred => Call::Starred { cursor },
            Source::Commits(query) => Call::SearchCommits(query.clone()),
            Source::People(name) => Call::SearchPeople(name.clone()),
            Source::Related(id) => Call::Related(id.clone()),
            Source::Headlines => Call::Headlines,
            Source::Headline(id) => Call::Headline(id.clone()),
        }
    }
}

/// Starred commits, search results, related commits or headlines: a list of cards, people or
/// headlines, in the order dev-null.sh gives them. A row that stops matching, such as an unstarred
/// commit, stays until the next reload.
pub struct ListScreen {
    source: Source,
    title: String,
    /// What the list is about, drawn above it: a headline's recap.
    about: Option<String>,
    list: Paged,
    /// Where the selected card was last drawn, for a box about it to open beside.
    card: Option<Rect>,
}

impl ListScreen {
    pub fn new(source: Source, now: Instant) -> Self {
        let mut list = Paged::default();
        list.asked(now);
        let title = match &source {
            Source::Starred => "starred".to_owned(),
            Source::Commits(query) => format!("/{query}"),
            Source::People(name) => format!("/@{name}"),
            Source::Related(id) => format!("related {}", id.short_hash()),
            Source::Headlines => "headlines".to_owned(),
            Source::Headline(_) => "headline".to_owned(),
        };
        ListScreen {
            source,
            title,
            about: None,
            list,
            card: None,
        }
    }

    /// A headline's commits, titled with its name and its recap shown while they load.
    pub fn headline(headline: Headline, now: Instant) -> Self {
        let mut list = ListScreen::new(Source::Headline(headline.id), now);
        list.recap(headline.name, headline.summary);
        list
    }

    /// Names the headline the list is about, with its latest recap.
    pub fn recap(&mut self, name: String, summary: Option<String>) {
        self.title = name;
        self.about = summary;
    }

    /// The first page.
    pub fn call(&self) -> Call {
        self.source.call(None)
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn card(&self) -> Option<Rect> {
        self.card
    }

    pub fn selected(&self) -> Option<&Item> {
        self.list.selected()
    }

    pub fn commit(&self) -> Option<&CommitId> {
        self.list.commit()
    }

    pub fn opens(&self) -> Option<Opens> {
        self.selected().map(Item::opens)
    }

    pub fn land(&mut self, cursor: Option<&str>, items: Vec<Item>, next: Option<String>) {
        self.list.land(cursor, items, next);
    }

    pub fn fail(&mut self, failure: Failure) {
        self.list.fail(failure);
    }

    /// A failed reload over rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        self.list.stale()
    }

    pub fn people(&self) -> bool {
        matches!(self.source, Source::People(_))
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.selected().is_some(),
            Action::Back | Action::Refresh => true,
            _ => self.list.can(action),
        }
    }

    pub fn act(&mut self, action: Action) {
        self.list.act(action);
    }

    pub fn reload(&mut self, now: Instant) {
        self.list.reload(now);
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        self.list.placeholder_at()
    }

    /// The next page, once the selection nears the end.
    pub fn wants_older(&self) -> Option<Call> {
        self.list
            .wants_older()
            .map(|cursor| self.source.call(Some(cursor)))
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
        let (loading, empty) = match &self.source {
            Source::Starred => (
                "loading starred",
                match keys::key_for(Action::Commit(Act::Star)) {
                    Some(key) => {
                        format!("Nothing starred yet. Press {key} on a commit to star it.")
                    }
                    None => "Nothing starred yet.".to_owned(),
                },
            ),
            Source::Commits(query) => ("searching", format!("No commits about \"{query}\".")),
            Source::People(name) => ("searching", format!("Nobody goes by \"{name}\".")),
            Source::Related(_) => (
                "finding commits like it",
                "Nothing like this commit yet.".to_owned(),
            ),
            Source::Headlines => ("loading headlines", "Nothing is hot right now.".to_owned()),
            Source::Headline(_) => (
                "loading",
                "No commits under this headline from the past week.".to_owned(),
            ),
        };
        let area = match &self.about {
            Some(about) => {
                let mut lines = Vec::new();
                feed::wrap(
                    &[(about.clone(), theme::muted())],
                    usize::from(area.width),
                    &mut lines,
                );
                let height = u16::try_from(lines.len() + 1).unwrap_or(u16::MAX);
                let [top, rest] =
                    Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area);
                let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
                frame.render_widget(Paragraph::new(lines), top);
                rest
            }
            None => area,
        };
        let is_ranked = !matches!(self.source, Source::Headline(_));
        self.card = self.list.draw(
            frame, area, store, timers, now, wall, loading, &empty, is_ranked,
        );
    }
}
