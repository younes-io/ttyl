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
    refills: HashMap<Log, Refill<Listing>>,
    reply_refills: HashMap<CommitId, Refill<Replies>>,
    star_wants: HashMap<CommitId, bool>,
    follows: HashMap<String, Follow>,
    deleting: HashSet<CommitId>,
    unsent_pastels: HashMap<CommitId, u32>,
    picking: Option<(CommitId, usize)>,
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

    /// Adds an older page's new ids. A page that adds nothing ends the list, so following its
    /// cursor doesn't ask again on every pump.
    fn extend(&mut self, ids: Vec<CommitId>, next_cursor: Option<String>) {
        self.next_cursor = extend(&mut self.ids, ids, next_cursor);
    }
}

impl Replies {
    fn extend(&mut self, ids: Vec<CommitId>, next_cursor: Option<String>) {
        self.next_cursor = extend(&mut self.ids, ids, next_cursor);
        self.more = None;
    }
}

/// Appends the ids not listed yet, and returns the cursor to follow next, if the page added any.
fn extend(
    listed: &mut Vec<CommitId>,
    ids: Vec<CommitId>,
    next_cursor: Option<String>,
) -> Option<String> {
    let before = listed.len();
    for id in ids {
        if !listed.contains(&id) {
            listed.push(id);
        }
    }
    next_cursor.filter(|_| listed.len() > before)
}

/// A list fetched again from its first page. It fills back to the depth the reader had loaded
/// before it takes the old list's place, so a reload drops what is gone everywhere at once and
/// never cuts the list short under the reader.
#[derive(Debug, Clone)]
struct Refill<T> {
    fresh: T,
    depth: usize,
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
            refills: HashMap::new(),
            reply_refills: HashMap::new(),
            star_wants: HashMap::new(),
            follows: HashMap::new(),
            deleting: HashSet::new(),
            unsent_pastels: HashMap::new(),
            picking: None,
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

    /// Puts the user on option `at` of the poll in `id` while they pick, or takes them off it.
    pub fn pick(&mut self, picking: Option<(CommitId, usize)>) {
        self.picking = picking;
        self.generation += 1;
    }

    pub fn picking(&self) -> Option<usize> {
        self.picking.as_ref().map(|(_, at)| *at)
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
                if let Some(Follow::Asking { .. }) = self.follows.get(username) {
                    self.follows.remove(username);
                }
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
            | Call::Vote { .. }
            | Call::Report { .. }
            | Call::Post { .. }
            | Call::Preview(_)
            | Call::Notifications { .. }
            | Call::Group { .. }
            | Call::Commits { .. }
            | Call::People { .. }
            | Call::Starred { .. } => return,
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
            (Call::Star { .. } | Call::Vote { .. }, Reply::Commit(focus)) => {
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
            (Call::User(username), Reply::User(profile)) => {
                let is_following = profile.is_following;
                if let Some(pinned) = profile.pinned {
                    self.land_row(pinned);
                    self.generation += 1;
                }
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
            (_, Reply::Rows(page)) => {
                self.absorb_rows(page.rows);
            }
            (_, Reply::People(page)) => {
                for person in page.people {
                    if let Some(server) = person.is_followed {
                        self.follows
                            .entry(person.user.username)
                            .or_insert(Follow::Known {
                                server,
                                want: server,
                            });
                    }
                }
                self.generation += 1;
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
                | Reply::User(_)
                | Reply::Notifications(_)
                | Reply::Group(_)
                | Reply::Preview(_)
                | Reply::Done,
            ) => {}
        }
    }

    fn place(&mut self, target: &Target, id: CommitId) {
        match target {
            Target::New | Target::Quote(_) => {
                for log in [Log::Latest, Log::Following] {
                    let refill = self.refills.get_mut(&log).map(|refill| &mut refill.fresh);
                    let shown = match self.logs.get_mut(&log) {
                        Some(Slot::Loaded(listing)) => Some(listing),
                        _ => None,
                    };
                    for listing in shown.into_iter().chain(refill) {
                        if !listing.ids.contains(&id) {
                            listing.ids.insert(0, id.clone());
                        }
                    }
                }
            }
            Target::Reply(parent) => {
                let refill = (self.reply_refills.get_mut(parent)).map(|refill| &mut refill.fresh);
                let shown = match self.replies.get_mut(parent) {
                    Some(Branch::Listed(replies)) => Some(replies),
                    _ => None,
                };
                for replies in shown.into_iter().chain(refill) {
                    if !replies.ids.contains(&id) {
                        replies.ids.push(id.clone());
                    }
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
        let picking = (self.picking.as_ref())
            .filter(|(on, _)| on == id)
            .map(|(_, at)| *at);
        if want.is_none() && !deleting && picking.is_none() {
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
        if let Some(poll) = row.media.poll.as_mut() {
            poll.picking = picking;
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
        let ids: Vec<CommitId> = page
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
        let next_cursor = page.next_cursor;
        let Some(cursor) = cursor else {
            let depth = match self.replies.get(id) {
                Some(Branch::Listed(old)) => old.ids.len(),
                Some(Branch::Failed(_)) | None => 0,
            };
            let fresh = Replies {
                ids,
                next_cursor,
                reload: None,
                more: None,
            };
            self.settle_replies(id, Refill { fresh, depth });
            return;
        };
        if let Some(mut refill) = self.reply_refills.remove(id) {
            if refill.fresh.next_cursor.as_deref() == Some(cursor) {
                refill.fresh.extend(ids, next_cursor);
                self.settle_replies(id, refill);
                return;
            }
            self.reply_refills.insert(id.clone(), refill);
        }
        if let Some(Branch::Listed(old)) = self.replies.get_mut(id)
            && old.next_cursor.as_deref() == Some(cursor)
        {
            old.extend(ids, next_cursor);
            self.generation += 1;
        }
    }

    /// Shows a refilled reply list once it is as deep as the one it replaces, or has run out.
    fn settle_replies(&mut self, id: &CommitId, refill: Refill<Replies>) {
        self.generation += 1;
        if refill.fresh.next_cursor.is_some() && refill.fresh.ids.len() < refill.depth {
            self.reply_refills.insert(id.clone(), refill);
        } else {
            self.reply_refills.remove(id);
            self.replies
                .insert(id.clone(), Branch::Listed(refill.fresh));
        }
    }

    /// The cursor of the next page a reply list refills from, while it does.
    pub fn refilling_replies(&self, id: &CommitId) -> Option<&str> {
        self.reply_refills.get(id)?.fresh.next_cursor.as_deref()
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
        // A card has no reply preview, so the one a log row brought stays.
        let (old_count, reply_preview) = old
            .map(|old| (Some(old.replies_count), old.reply_preview))
            .unwrap_or_else(|| (None, no_preview()));
        self.rows.insert(
            id.clone(),
            Row {
                commit: card.commit,
                media: card.media,
                stars_count: card.stars_count,
                starred: card.starred,
                replies_count: replies_count.or(old_count).unwrap_or(0),
                quoted_commit: card.quoted_commit,
                reply_preview,
                deletion: None,
            },
        );
        self.gone.remove(&id);
        id
    }

    fn land_first_page(&mut self, page: LogPage) {
        let ids = self.absorb(page.rows);
        let depth = match self.logs.get(&page.log) {
            Some(Slot::Loaded(old)) => old.ids.len(),
            Some(Slot::Loading | Slot::Failed(_)) | None => 0,
        };
        let fresh = Listing {
            ids,
            is_ranked: page.is_ranked,
            next_cursor: page.next_cursor,
            new_commits: 0,
            failure: None,
            suggestions: page.suggestions,
        };
        self.settle(page.log, Refill { fresh, depth });
    }

    fn append(&mut self, log: Log, cursor: &str, page: LogPage) {
        let ids = self.absorb(page.rows);
        self.generation += 1;
        if let Some(mut refill) = self.refills.remove(&log) {
            if refill.fresh.ends_at(cursor) {
                refill.fresh.extend(ids, page.next_cursor);
                self.settle(log, refill);
                return;
            }
            self.refills.insert(log, refill);
        }
        if let Some(Slot::Loaded(listing)) = self.logs.get_mut(&log)
            && listing.ends_at(cursor)
        {
            listing.extend(ids, page.next_cursor);
        }
    }

    /// Shows a refilled log once it is as deep as the one it replaces, or has run out.
    fn settle(&mut self, log: Log, refill: Refill<Listing>) {
        self.generation += 1;
        if refill.fresh.next_cursor.is_some() && refill.fresh.ids.len() < refill.depth {
            self.refills.insert(log, refill);
        } else {
            self.refills.remove(&log);
            self.logs.insert(log, Slot::Loaded(refill.fresh));
        }
    }

    /// The cursor of the next page a log refills from, while it does.
    pub fn refilling(&self, log: Log) -> Option<&str> {
        self.refills.get(&log)?.fresh.next_cursor.as_deref()
    }

    /// Keeps rows a screen lists on its own, and returns their ids in order.
    pub fn absorb_rows(&mut self, rows: Vec<Row>) -> Vec<CommitId> {
        self.generation += 1;
        self.absorb(rows)
    }

    fn absorb(&mut self, rows: Vec<Row>) -> Vec<CommitId> {
        rows.into_iter().map(|row| self.land_row(row)).collect()
    }

    /// A row whose reply count moved since its replies were listed drops that list, so an open
    /// thread fetches the replies that came in, or left, since.
    fn land_row(&mut self, row: Row) -> CommitId {
        let id = row.commit.id.clone();
        if self
            .rows
            .get(&id)
            .is_some_and(|old| old.replies_count != row.replies_count)
        {
            self.replies.remove(&id);
            self.reply_refills.remove(&id);
        }
        self.gone.remove(&id);
        self.rows.insert(id.clone(), row);
        id
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

    /// The rows behind these ids as they are shown now, the deleted ones marked so.
    pub fn rows_of(&self, ids: &[CommitId]) -> Vec<Row> {
        ids.iter()
            .filter_map(|id| {
                let mut row = self.shown(self.rows.get(id)?).into_owned();
                if self.gone.contains_key(id) {
                    row.deletion = Some(Deletion::Done);
                }
                Some(row)
            })
            .collect()
    }

    pub fn page(&self, log: Log) -> Option<LogPage> {
        let Slot::Loaded(listing) = self.slot(log) else {
            return None;
        };
        Some(LogPage {
            log,
            rows: self.rows_of(&listing.ids),
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
