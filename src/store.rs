//! Everything the server has told this session, kept once. A commit has one slot however many logs list it, so
//! a change to it shows everywhere at once.

use std::collections::HashMap;

use crate::api::{Account, Me, Reply};
use crate::model::{CommitId, Entry, Log, LogPage};

pub struct Store {
    pub me: Me,
    pub unread: u32,
    entries: HashMap<CommitId, Entry>,
    logs: HashMap<Log, Slot>,
}

/// Where a log's first page stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Loading,
    Failed(String),
    Loaded(Listing),
}

/// A log page as the list of commits it showed, with the ids resolved through the store on read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub ids: Vec<CommitId>,
    pub is_ranked: bool,
    pub next_cursor: Option<String>,
    suggestions: Vec<CommitId>,
}

impl Store {
    pub fn new(account: Account) -> Self {
        Store {
            me: account.me,
            unread: account.unread,
            entries: HashMap::new(),
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
                    ids: self.absorb(page.entries),
                    is_ranked: page.is_ranked,
                    next_cursor: page.next_cursor,
                    suggestions: self.absorb(page.suggestions),
                };
                self.logs.insert(page.log, Slot::Loaded(listing));
            }
        }
    }

    fn absorb(&mut self, entries: Vec<Entry>) -> Vec<CommitId> {
        entries
            .into_iter()
            .map(|entry| {
                let id = entry.id().clone();
                self.entries.insert(id.clone(), entry);
                id
            })
            .collect()
    }

    /// A page load failed. Loaded rows stay on screen; only a log with nothing yet shows the failure.
    pub fn fail(&mut self, log: Log, message: String) {
        if !matches!(self.slot(log), Slot::Loaded(_)) {
            self.logs.insert(log, Slot::Failed(message));
        }
    }

    pub fn slot(&self, log: Log) -> &Slot {
        self.logs.get(&log).unwrap_or(&Slot::Loading)
    }

    pub fn entry(&self, id: &CommitId) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// The page the renderer draws for a log, rebuilt from the ids so it reads the newest state of each commit.
    pub fn page(&self, log: Log) -> Option<LogPage> {
        let Slot::Loaded(listing) = self.slot(log) else {
            return None;
        };
        let resolve = |ids: &[CommitId]| {
            ids.iter()
                .filter_map(|id| self.entries.get(id).cloned())
                .collect()
        };
        Some(LogPage {
            log,
            entries: resolve(&listing.ids),
            next_cursor: listing.next_cursor.clone(),
            is_ranked: listing.is_ranked,
            new_commits_count: None,
            suggestions: resolve(&listing.suggestions),
        })
    }
}
