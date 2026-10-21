use std::collections::HashMap;

use crate::api::{Account, Me, Reply};
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
    Failed(String),
    Loaded(Listing),
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub ids: Vec<CommitId>,
    pub is_ranked: bool,
    pub next_cursor: Option<String>,
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

    pub fn apply(&mut self, reply: Reply) {
        match reply {
            Reply::Account(account) => {
                self.me = account.me;
                self.unread = account.unread;
            }
            Reply::Page(page) => {
                let listing = Listing {
                    ids: self.absorb(page.rows),
                    is_ranked: page.is_ranked,
                    next_cursor: page.next_cursor,
                    suggestions: page.suggestions,
                };
                self.logs.insert(page.log, Slot::Loaded(listing));
            }
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

    pub fn fail(&mut self, log: Log, message: String) {
        if !matches!(self.slot(log), Slot::Loaded(_)) {
            self.logs.insert(log, Slot::Failed(message));
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
