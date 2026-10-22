use std::collections::HashMap;

use crate::api::{Account, Call, Failure, Me, Reply};
use crate::model::{
    Card, CommitId, CommitPage, Log, LogPage, RepliesPage, ReplyPreview, Row, Shown, Suggestion,
    Tombstone,
};

pub struct Store {
    pub me: Me,
    pub unread: u32,
    rows: HashMap<CommitId, Row>,
    gone: HashMap<CommitId, Tombstone>,
    logs: HashMap<Log, Slot>,
    threads: HashMap<CommitId, Thread>,
    replies: HashMap<CommitId, Branch>,
    generation: u64,
}

pub enum Node<'a> {
    Live(&'a Row),
    Gone(&'a Tombstone),
}

impl Node<'_> {
    pub fn replies_count(&self) -> u32 {
        match self {
            Node::Live(row) => row.replies_count,
            Node::Gone(tombstone) => tombstone.replies_count.unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Thread {
    pub chain: Chain,
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, Default)]
pub enum Chain {
    #[default]
    Unknown,
    Known {
        ancestors: Vec<CommitId>,
        earlier: Option<CommitId>,
    },
    Missing,
}

#[derive(Debug, Clone)]
pub enum Branch {
    Failed(Failure),
    Listed(Replies),
}

#[derive(Debug, Clone)]
pub struct Replies {
    pub ids: Vec<CommitId>,
    pub next_cursor: Option<String>,
    pub reload: Option<Failure>,
    pub more: Option<Failure>,
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
    pub failure: Option<Failure>,
    suggestions: Vec<Suggestion>,
}

impl Listing {
    fn ends_at(&self, cursor: &str) -> bool {
        self.next_cursor.as_deref() == Some(cursor)
    }
}

impl Store {
    pub fn new(account: Account) -> Self {
        Store {
            me: account.me,
            unread: account.unread,
            rows: HashMap::new(),
            gone: HashMap::new(),
            logs: HashMap::new(),
            threads: HashMap::new(),
            replies: HashMap::new(),
            generation: 0,
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
            (_, Reply::Page(page)) => self.land_first_page(page),
            (Call::Commit(id), Reply::Thread(page)) => self.land_thread(id, *page),
            (Call::Replies { id, cursor }, Reply::Replies(page)) => {
                self.land_replies(id, cursor.as_deref(), page);
            }
            (_, Reply::Thread(_) | Reply::Replies(_)) => {}
        }
    }

    fn land_thread(&mut self, id: &CommitId, page: CommitPage) {
        self.keep_counted(page.commit, Some(page.replies_count));
        let earlier = page.earlier.map(|card| self.keep(card, Self::keep_card));
        let ancestors = page
            .ancestors
            .into_iter()
            .map(|card| self.keep(card, Self::keep_card))
            .collect();
        self.threads.insert(
            id.clone(),
            Thread {
                chain: Chain::Known { ancestors, earlier },
                failure: None,
            },
        );
        self.generation += 1;
    }

    fn land_replies(&mut self, id: &CommitId, cursor: Option<&str>, page: RepliesPage) {
        let mut ids: Vec<CommitId> = page
            .replies
            .into_iter()
            .map(|reply| {
                self.keep(reply, |store, reply| {
                    let id = reply.commit.id.clone();
                    let old = store.rows.remove(&id);
                    let (quoted_commit, reply_preview) = old
                        .map(|old| (old.quoted_commit, old.reply_preview))
                        .unwrap_or_else(|| (None, no_preview()));
                    store.rows.insert(
                        id.clone(),
                        Row {
                            commit: reply.commit,
                            media: reply.media,
                            stars_count: reply.stars_count,
                            starred: reply.starred,
                            replies_count: reply.replies_count,
                            quoted_commit,
                            reply_preview,
                        },
                    );
                    id
                })
            })
            .collect();
        let mut next_cursor = page.next_cursor;
        let old = match self.replies.remove(id) {
            Some(Branch::Listed(old)) => Some(old),
            Some(Branch::Failed(_)) | None => None,
        };
        let replies = match (cursor, old) {
            (None, Some(old)) => {
                if let Some(last) = old.ids.iter().rposition(|each| ids.contains(each)) {
                    ids.extend(old.ids[last + 1..].iter().cloned());
                    next_cursor = old.next_cursor;
                }
                Replies {
                    ids,
                    next_cursor,
                    reload: None,
                    more: old.more,
                }
            }
            (None, None) => Replies {
                ids,
                next_cursor,
                reload: None,
                more: None,
            },
            (Some(cursor), Some(mut old)) => {
                if old.next_cursor.as_deref() == Some(cursor) {
                    for id in ids {
                        if !old.ids.contains(&id) {
                            old.ids.push(id);
                        }
                    }
                    old.next_cursor = next_cursor;
                    old.more = None;
                }
                old
            }
            (Some(_), None) => return,
        };
        self.replies.insert(id.clone(), Branch::Listed(replies));
        self.generation += 1;
    }

    fn keep<T>(
        &mut self,
        shown: Shown<T>,
        live: impl FnOnce(&mut Self, T) -> CommitId,
    ) -> CommitId {
        match shown {
            Shown::Live(value) => {
                let id = live(self, *value);
                self.gone.remove(&id);
                id
            }
            Shown::Deleted(tombstone) => {
                let id = tombstone.id.clone();
                self.gone.insert(id.clone(), tombstone);
                id
            }
        }
    }

    fn keep_card(&mut self, card: Card) -> CommitId {
        self.keep_counted(card, None)
    }

    fn keep_counted(&mut self, card: Card, replies_count: Option<u32>) -> CommitId {
        let id = card.commit.id.clone();
        let old = self.rows.remove(&id);
        let (old_count, old_quote, reply_preview) = old
            .map(|old| {
                (
                    Some(old.replies_count),
                    old.quoted_commit,
                    old.reply_preview,
                )
            })
            .unwrap_or_else(|| (None, None, no_preview()));
        self.rows.insert(
            id.clone(),
            Row {
                commit: card.commit,
                media: card.media,
                stars_count: card.stars_count,
                starred: card.starred,
                replies_count: replies_count.or(old_count).unwrap_or(0),
                quoted_commit: card.quoted_commit.or(old_quote),
                reply_preview,
            },
        );
        self.gone.remove(&id);
        id
    }

    fn land_first_page(&mut self, page: LogPage) {
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
            failure: None,
            suggestions: page.suggestions,
        };
        self.logs.insert(page.log, Slot::Loaded(listing));
        self.generation += 1;
    }

    fn append(&mut self, log: Log, cursor: &str, page: LogPage) {
        let Some(Slot::Loaded(listing)) = self.logs.get_mut(&log) else {
            return;
        };
        if !listing.ends_at(cursor) {
            return;
        }
        listing.next_cursor = page.next_cursor;
        self.generation += 1;
        for row in page.rows {
            let id = row.commit.id.clone();
            if !listing.ids.contains(&id) {
                listing.ids.push(id.clone());
            }
            self.gone.remove(&id);
            self.rows.insert(id, row);
        }
    }

    fn absorb(&mut self, rows: Vec<Row>) -> Vec<CommitId> {
        rows.into_iter()
            .map(|row| {
                let id = row.commit.id.clone();
                self.gone.remove(&id);
                self.rows.insert(id.clone(), row);
                id
            })
            .collect()
    }

    pub fn fail(&mut self, log: Log, failure: Failure) {
        match self.logs.get_mut(&log) {
            Some(Slot::Loaded(listing)) => listing.failure = Some(failure),
            Some(Slot::Loading | Slot::Failed(_)) | None => {
                self.logs.insert(log, Slot::Failed(failure));
            }
        }
    }

    pub fn fail_commit(&mut self, id: &CommitId, failure: Failure) {
        self.threads.entry(id.clone()).or_default().failure = Some(failure);
        self.generation += 1;
    }

    pub fn missing(&mut self, id: &CommitId) {
        let thread = self.threads.entry(id.clone()).or_default();
        thread.chain = Chain::Missing;
        thread.failure = None;
        self.generation += 1;
    }

    pub fn fail_replies(&mut self, id: &CommitId, more: bool, failure: Failure) {
        match self.replies.get_mut(id) {
            Some(Branch::Listed(replies)) if more => replies.more = Some(failure),
            Some(Branch::Listed(replies)) => replies.reload = Some(failure),
            Some(Branch::Failed(_)) | None if more => return,
            Some(Branch::Failed(_)) | None => {
                self.replies.insert(id.clone(), Branch::Failed(failure));
            }
        }
        self.generation += 1;
    }

    pub fn no_replies(&mut self, id: &CommitId, more: bool) {
        match self.replies.get_mut(id) {
            Some(Branch::Listed(replies)) if more => {
                replies.next_cursor = None;
                replies.more = None;
            }
            Some(Branch::Failed(_)) | None if more => return,
            _ => {
                self.replies.insert(
                    id.clone(),
                    Branch::Listed(Replies {
                        ids: Vec::new(),
                        next_cursor: None,
                        reload: None,
                        more: None,
                    }),
                );
            }
        }
        self.generation += 1;
    }

    pub fn thread(&self, id: &CommitId) -> Option<&Thread> {
        self.threads.get(id)
    }

    pub fn replies(&self, id: &CommitId) -> Option<&Branch> {
        self.replies.get(id)
    }

    pub fn node(&self, id: &CommitId) -> Option<Node<'_>> {
        match self.gone.get(id) {
            Some(tombstone) => Some(Node::Gone(tombstone)),
            None => self.rows.get(id).map(Node::Live),
        }
    }

    /// Changes whenever a page lands, so a layout built from the rows can tell it is stale.
    pub fn generation(&self) -> u64 {
        self.generation
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

fn no_preview() -> ReplyPreview {
    ReplyPreview {
        data: Vec::new(),
        next_cursor: None,
    }
}
