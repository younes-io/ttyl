use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::api::{Call, Failure};
use crate::clock::Timers;
use crate::keys::{self, Act, Action};
use crate::model::CommitId;
use crate::profile::{Item, Paged};
use crate::store::Store;

pub const TITLE: &str = "starred";

/// Your starred commits, newest star first. An unstarred row stays until the next reload.
pub struct StarredScreen {
    list: Paged,
    /// Where the selected card was last drawn, for a box about it to open beside.
    card: Option<Rect>,
}

impl StarredScreen {
    pub fn new(now: Instant) -> Self {
        let mut list = Paged::default();
        list.asked(now);
        StarredScreen { list, card: None }
    }

    pub fn call(cursor: Option<String>) -> Call {
        Call::Starred { cursor }
    }

    pub fn card(&self) -> Option<Rect> {
        self.card
    }

    pub fn commit(&self) -> Option<&CommitId> {
        self.list.commit()
    }

    pub fn land(&mut self, cursor: Option<&str>, ids: Vec<CommitId>, next: Option<String>) {
        let items = ids.into_iter().map(Item::Commit).collect();
        self.list.land(cursor, items, next);
    }

    pub fn fail(&mut self, failure: Failure) {
        self.list.fail(failure);
    }

    /// A failed reload over rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        self.list.stale()
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.commit().is_some(),
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
            .map(|cursor| Self::call(Some(cursor)))
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
        let empty = match keys::key_for(Action::Commit(Act::Star)) {
            Some(key) => format!("Nothing starred yet. Press {key} on a commit to star it."),
            None => "Nothing starred yet.".to_owned(),
        };
        self.card = self.list.draw(
            frame,
            area,
            store,
            timers,
            now,
            wall,
            "loading starred",
            &empty,
            true,
        );
    }
}
