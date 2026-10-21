use std::collections::HashMap;

use crate::api::{Account, Call, Failure, Me, Reply};
use crate::model::{CommitId, Log, LogPage, Row, Suggestion};

pub struct Store {
    pub me: Me,
    pub unread: u32,
    rows: HashMap<CommitId, Row>,
    logs: HashMap<Log, Slot>,
}

#[derive(Debug, Clone)]
pub enum Slot {
    Loading,
    Failed(Failure),
    Loaded(Listing),
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub ids: Vec<CommitId>,
    pub is_ranked: bool,
    pub next_cursor: Option<String>,
    pub new_commits: u32,
    suggestions: Vec<Suggestion>,
}

impl Store {
    pub fn new(account: Account) -> Self {
        Store {
            me: account.me,
            unread: account.unread,
            rows: HashMap::new(),
            logs: HashMap::new(),
        }
    }

    pub fn apply(&mut self, call: &Call, reply: Reply) {
        match (call, reply) {
            (_, Reply::Account(account)) => {
                self.me = account.me;
                self.unread = account.unread;
            }
            (Call::Older { log, cursor }, Reply::Page(page)) => self.append(*log, cursor, page),
            (Call::Since { log, since }, Reply::Page(page)) => {
                if let Some(Slot::Loaded(listing)) = self.logs.get_mut(log)
                    && listing.ids.first() == Some(since)
                {
                    listing.new_commits = page.new_commits_count.unwrap_or(0);
                }
            }
            (_, Reply::Page(page)) => self.replace(page),
        }
    }

    /// A first page that overlaps a chronological log lands on top of it, so the older pages below stay loaded.
    fn replace(&mut self, page: LogPage) {
        let mut ids = self.absorb(page.rows);
        let mut next_cursor = page.next_cursor;
        if let Some(Slot::Loaded(old)) = self.logs.get(&page.log)
            && !page.is_ranked
            && let Some(last) = old.ids.iter().rposition(|id| ids.contains(id))
        {
            ids.extend(old.ids[last + 1..].iter().cloned());
            next_cursor.clone_from(&old.next_cursor);
        }
        let listing = Listing {
            ids,
            is_ranked: page.is_ranked,
            next_cursor,
            new_commits: 0,
            suggestions: page.suggestions,
        };
        self.logs.insert(page.log, Slot::Loaded(listing));
    }

    /// A page asked for after a refresh moved the cursor on belongs to a listing that is gone, so it is dropped.
    fn append(&mut self, log: Log, cursor: &str, page: LogPage) {
        let Some(Slot::Loaded(listing)) = self.logs.get_mut(&log) else {
            return;
        };
        if listing.next_cursor.as_deref() != Some(cursor) {
            return;
        }
        listing.next_cursor = page.next_cursor;
        for row in page.rows {
            let id = row.commit.id.clone();
            if !listing.ids.contains(&id) {
                listing.ids.push(id.clone());
            }
            self.rows.insert(id, row);
        }
    }

    fn absorb(&mut self, rows: Vec<Row>) -> Vec<CommitId> {
        rows.into_iter()
            .map(|row| {
                let id = row.commit.id.clone();
                self.rows.insert(id.clone(), row);
                id
            })
            .collect()
    }

    pub fn fail(&mut self, log: Log, failure: Failure) {
        if !matches!(self.slot(log), Slot::Loaded(_)) {
            self.logs.insert(log, Slot::Failed(failure));
        }
    }

    pub fn slot(&self, log: Log) -> &Slot {
        self.logs.get(&log).unwrap_or(&Slot::Loading)
    }

    pub fn row(&self, id: &CommitId) -> Option<&Row> {
        self.rows.get(id)
    }

    pub fn page(&self, log: Log) -> Option<LogPage> {
        let Slot::Loaded(listing) = self.slot(log) else {
            return None;
        };
        Some(LogPage {
            log,
            rows: listing
                .ids
                .iter()
                .filter_map(|id| self.rows.get(id).cloned())
                .collect(),
            next_cursor: listing.next_cursor.clone(),
            is_ranked: listing.is_ranked,
            new_commits_count: None,
            suggestions: listing.suggestions.clone(),
        })
    }
}
