use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use chrono::Utc;

use crate::api::{Account, Call, Failure, Me, Reply};
use crate::draft::Target;
use crate::model::{
    Card, CommitId, CommitPage, Deletion, Log, LogPage, RepliesPage, ReplyPreview, Row, Shown,
    Suggestion, Tombstone,
};
use crate::sched::CallKey;

pub struct Store {
    pub me: Me,
    pub unread: u32,
    rows: HashMap<CommitId, Row>,
    gone: HashMap<CommitId, Tombstone>,
    logs: HashMap<Log, Slot>,
    threads: HashMap<CommitId, Thread>,
    replies: HashMap<CommitId, Branch>,
    star_wants: HashMap<CommitId, bool>,
    follows: HashMap<String, Follow>,
    deleting: HashSet<CommitId>,
    unsent_pastels: HashMap<CommitId, u32>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    Asking { odd_presses: bool },
    Known { server: bool, want: bool },
}

pub enum Node<'a> {
    Live(Box<Cow<'a, Row>>),
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
            star_wants: HashMap::new(),
            follows: HashMap::new(),
            deleting: HashSet::new(),
            unsent_pastels: HashMap::new(),
            generation: 0,
        }
    }

    /// Flips the star the user sees and returns whether it is now starred.
    pub fn star(&mut self, id: &CommitId) -> Option<bool> {
        let row = self.rows.get(id)?;
        let want = !self.star_wants.get(id).copied().unwrap_or(row.starred);
        self.star_wants.insert(id.clone(), want);
        self.generation += 1;
        Some(want)
    }

    /// Flips the follow the user sees and returns it, or `None` while the client still asks the server.
    pub fn follow(&mut self, username: &str) -> Option<bool> {
        let next = match self.follows.get(username) {
            None => Follow::Asking { odd_presses: true },
            Some(Follow::Asking { odd_presses }) => Follow::Asking {
                odd_presses: !odd_presses,
            },
            Some(Follow::Known { server, want }) => Follow::Known {
                server: *server,
                want: !want,
            },
        };
        self.follows.insert(username.to_owned(), next);
        match next {
            Follow::Asking { .. } => None,
            Follow::Known { want, .. } => Some(want),
        }
    }

    pub fn following(&self, username: &str) -> Option<bool> {
        match self.follows.get(username)? {
            Follow::Asking { .. } => None,
            Follow::Known { want, .. } => Some(*want),
        }
    }

    pub fn give_pastel(&mut self, id: &CommitId) {
        *self.unsent_pastels.entry(id.clone()).or_default() += 1;
    }

    pub fn start_delete(&mut self, id: &CommitId) {
        self.deleting.insert(id.clone());
        self.generation += 1;
    }

    pub fn writes(&mut self, pending: impl Fn(&CallKey) -> bool) -> Vec<Call> {
        let mut calls = Vec::new();
        let rows = &self.rows;
        self.star_wants.retain(|id, want| {
            if pending(&CallKey::Star(id.clone())) {
                return true;
            }
            match rows.get(id) {
                Some(row) if row.starred != *want => {
                    calls.push(Call::Star {
                        id: id.clone(),
                        on: *want,
                    });
                    true
                }
                Some(_) | None => false,
            }
        });
        for (username, follow) in &self.follows {
            match *follow {
                Follow::Asking { .. } if !pending(&CallKey::User(username.clone())) => {
                    calls.push(Call::User(username.clone()));
                }
                Follow::Known { server, want }
                    if server != want && !pending(&CallKey::Follow(username.clone())) =>
                {
                    calls.push(Call::Follow {
                        username: username.clone(),
                        on: want,
                    });
                }
                Follow::Asking { .. } | Follow::Known { .. } => {}
            }
        }
        self.unsent_pastels.retain(|id, left| {
            if !pending(&CallKey::Pastel(id.clone())) {
                calls.push(Call::Pastel(id.clone()));
                *left -= 1;
            }
            *left > 0
        });
        calls
    }

    pub fn refused(&mut self, call: &Call) {
        match call {
            Call::Star { id, .. } => {
                self.star_wants.remove(id);
            }
            Call::Follow { username, .. } => {
                if let Some(Follow::Known { server, want }) = self.follows.get_mut(username) {
                    *want = *server;
                }
            }
            Call::User(username) => {
                self.follows.remove(username);
            }
            Call::Delete(id) => {
                self.deleting.remove(id);
            }
            Call::Pastel(id) => {
                self.unsent_pastels.remove(id);
                return;
            }
            Call::Me
            | Call::Redeem { .. }
            | Call::Page(_)
            | Call::Older { .. }
            | Call::Since { .. }
            | Call::Commit(_)
            | Call::Replies { .. }
            | Call::Report { .. }
            | Call::Post { .. } => return,
        }
        self.generation += 1;
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
            (Call::Star { .. }, Reply::Commit(focus)) => {
                self.keep_counted(focus.card, Some(focus.replies_count));
                self.generation += 1;
            }
            (Call::Post { target, .. }, Reply::Commit(focus)) => {
                let id = self.keep_counted(focus.card, Some(focus.replies_count));
                self.place(target, id);
                self.generation += 1;
            }
            (Call::Pastel(_), Reply::Pastel { commit, remaining }) => {
                self.keep_counted(commit.card, Some(commit.replies_count));
                self.me.pasteis_remaining = Some(remaining);
                self.generation += 1;
            }
            (Call::User(username), Reply::User { is_following }) => {
                let want = match self.follows.get(username) {
                    Some(Follow::Asking { odd_presses }) => is_following ^ odd_presses,
                    Some(Follow::Known { want, .. }) => *want,
                    None => is_following,
                };
                self.follows.insert(
                    username.clone(),
                    Follow::Known {
                        server: is_following,
                        want,
                    },
                );
            }
            (Call::Follow { username, on }, Reply::Done) => {
                if let Some(Follow::Known { server, .. }) = self.follows.get_mut(username) {
                    *server = *on;
                }
            }
            (Call::Delete(id), Reply::Done) => {
                self.deleting.remove(id);
                let row = self.rows.get(id);
                let tombstone = Tombstone {
                    id: id.clone(),
                    parent_id: row.and_then(|row| row.commit.parent_id.clone()),
                    deleted_at: Utc::now(),
                    replies_count: row.map(|row| row.replies_count),
                };
                self.gone.insert(id.clone(), tombstone);
                self.generation += 1;
            }
            (
                _,
                Reply::Thread(_)
                | Reply::Replies(_)
                | Reply::Commit(_)
                | Reply::Pastel { .. }
                | Reply::User { .. }
                | Reply::Done,
            ) => {}
        }
    }

    fn place(&mut self, target: &Target, id: CommitId) {
        match target {
            Target::New | Target::Quote(_) => {
                for log in [Log::Latest, Log::Following] {
                    if let Some(Slot::Loaded(listing)) = self.logs.get_mut(&log)
                        && !listing.ids.contains(&id)
                    {
                        listing.ids.insert(0, id.clone());
                    }
                }
            }
            Target::Reply(parent) => {
                if let Some(Branch::Listed(replies)) = self.replies.get_mut(parent)
                    && !replies.ids.contains(&id)
                {
                    replies.ids.push(id);
                }
                if let Some(row) = self.rows.get_mut(parent) {
                    row.replies_count += 1;
                }
            }
        }
    }

    fn shown<'a>(&self, row: &'a Row) -> Cow<'a, Row> {
        let id = &row.commit.id;
        let want = self
            .star_wants
            .get(id)
            .copied()
            .filter(|want| *want != row.starred);
        let deleting = self.deleting.contains(id);
        if want.is_none() && !deleting {
            return Cow::Borrowed(row);
        }
        let mut row = row.clone();
        if let Some(want) = want {
            row.starred = want;
            row.stars_count = if want {
                row.stars_count + 1
            } else {
                row.stars_count.saturating_sub(1)
            };
        }
        if deleting {
            row.deletion = Some(Deletion::Running);
        }
        Cow::Owned(row)
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
                            deletion: None,
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
                deletion: None,
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
            None => self
                .rows
                .get(id)
                .map(|row| Node::Live(Box::new(self.shown(row)))),
        }
    }

    /// Changes whenever what the rows show changes, so a layout built from them can tell it is stale.
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
                .filter_map(|id| {
                    let mut row = self.shown(self.rows.get(id)?).into_owned();
                    if self.gone.contains_key(id) {
                        row.deletion = Some(Deletion::Done);
                    }
                    Some(row)
                })
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
