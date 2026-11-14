use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::api::{Account, Call, Failure, Me, Reply};
use crate::config::{Change, Config, Field, Wants};
use crate::draft::Target;
use crate::locale;
use crate::model::{
    Card, Chat, CommitId, CommitPage, Deletion, Filed, Fold, Hush, Log, LogPage, Refile,
    RepliesPage, Row, Settings, Showing, Shown, Suggestion, Tombstone, Translated, Translation,
    Untranslated, Who,
};
use crate::sched::{CallKey, Ticket};

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
    pin_want: Option<Pin>,
    follows: HashMap<Who, Follow>,
    mutes: HashMap<String, Switch>,
    blocks: HashMap<String, Switch>,
    filings: HashMap<Chat, Filing>,
    language: Language,
    /// The language the device reads in, which commits translate into unless the user picked one.
    device_language: Option<String>,
    /// What dev-null.sh's settings page last said, once asked.
    settings: Option<Settings>,
    /// Settings the user changed that dev-null.sh hasn't confirmed.
    wants: Wants,
    /// The first call to go out after dev-null.sh confirmed each setting's last change. A read that
    /// went out before it may still hold the value from before.
    confirmed: HashMap<Field, Ticket>,
    translations: HashMap<CommitId, Translating>,
    /// Everyone whose follow of the user a block ended. Unblocking leaves it ended.
    unfollowed_by: HashSet<String>,
    deleting: HashSet<CommitId>,
    unsent_pastels: HashMap<CommitId, u32>,
    picking: Option<(CommitId, usize)>,
    left_off: HashMap<Log, DateTime<Utc>>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    Asking { odd_presses: bool },
    Known { server: bool, want: bool },
}

/// Whether the user mutes or blocks someone, as dev-null.sh last said and as they want it now.
#[derive(Debug, Clone, Copy, Default)]
struct Switch {
    server: bool,
    want: bool,
}

/// How the user keeps a conversation, as dev-null.sh last said and as they want it now, with how
/// long a mute they picked since lasts and the one dev-null.sh last took.
#[derive(Debug, Clone, Copy)]
struct Filing {
    server: Filed,
    want: Filed,
    /// The mute length the user picked since, while they want it muted. A muted `want` without
    /// one is the mute the conversation came with, which dev-null.sh already has.
    hush: Option<Hush>,
    took: Option<Hush>,
}

impl Filing {
    /// The change still to send, if any, one at a time.
    fn next(&self) -> Option<Refile> {
        let (server, want) = (self.server, self.want);
        // A mute set elsewhere leaves `took` empty, so a length picked here goes out even when
        // the conversation already shows muted.
        let rehush = self.hush.is_some_and(|hush| self.took != Some(hush));
        if server.pinned != want.pinned {
            Some(Refile::Pin(want.pinned))
        } else if server.stashed != want.stashed {
            Some(Refile::Stash(want.stashed))
        } else if server.muted != want.muted || rehush {
            Some(Refile::Mute(self.hush))
        } else {
            None
        }
    }
}

/// The language commits translate into, settled once dev-null.sh says whether the user picked one.
enum Language {
    Unsettled,
    Settled(String),
}

/// A commit's translation, and whether the reader wants it in place of the original.
enum Translating {
    Asking {
        show: bool,
    },
    Got {
        translated: Box<Translated>,
        show: bool,
    },
    Skipped(Untranslated),
}

/// The user's commit they last asked to pin or unpin, until dev-null.sh says it is so.
#[derive(Debug, Clone)]
struct Pin {
    id: CommitId,
    on: bool,
}

/// dev-null.sh keeps one pin per user, so `id` pinned unpins the rest of `handle`'s commits.
fn unpin_others(rows: &mut HashMap<CommitId, Row>, handle: &str, id: &CommitId) {
    for row in rows.values_mut() {
        if row.commit.author.handle == handle && row.commit.id != *id {
            row.commit.is_pinned = false;
        }
    }
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
            pin_want: None,
            follows: HashMap::new(),
            mutes: HashMap::new(),
            language: Language::Unsettled,
            device_language: None,
            settings: None,
            wants: Wants::default(),
            confirmed: HashMap::new(),
            translations: HashMap::new(),
            blocks: HashMap::new(),
            filings: HashMap::new(),
            unfollowed_by: HashSet::new(),
            deleting: HashSet::new(),
            unsent_pastels: HashMap::new(),
            picking: None,
            left_off: HashMap::new(),
            generation: 0,
        }
    }

    /// Translates into `device`, the language the device reads in, unless the user picked one on dev-null.sh.
    pub fn with_device_language(self, device: Option<String>) -> Self {
        Store {
            device_language: device,
            ..self
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

    /// Flips whether the user's commit is pinned to their profile and returns whether it now is.
    pub fn pin(&mut self, id: &CommitId) -> Option<bool> {
        let on = !self.is_pinned(self.rows.get(id)?);
        self.pin_want = Some(Pin { id: id.clone(), on });
        self.generation += 1;
        Some(on)
    }

    pub fn pinned(&self, id: &CommitId) -> bool {
        self.rows.get(id).is_some_and(|row| self.is_pinned(row))
    }

    /// The commit `handle` has pinned, among the ones loaded.
    pub fn pinned_by(&self, handle: &str) -> Option<&CommitId> {
        self.rows
            .values()
            .find(|row| row.commit.author.handle == handle && self.is_pinned(row))
            .map(|row| &row.commit.id)
    }

    /// Whether `row` is pinned as the user sees it. Pinning one commit unpins the user's others.
    fn is_pinned(&self, row: &Row) -> bool {
        match &self.pin_want {
            Some(pin) if pin.id == row.commit.id => pin.on,
            Some(Pin { on: true, .. }) if row.commit.author.handle == self.me.handle => false,
            Some(_) | None => row.commit.is_pinned,
        }
    }

    /// Flips the follow the user sees and returns it, or `None` while the client still asks the server.
    pub fn follow(&mut self, who: &Who) -> Option<bool> {
        let next = match self.follows.get(who) {
            None => Follow::Asking { odd_presses: true },
            Some(Follow::Asking { odd_presses }) => Follow::Asking {
                odd_presses: !odd_presses,
            },
            Some(Follow::Known { server, want }) => Follow::Known {
                server: *server,
                want: !want,
            },
        };
        self.follows.insert(who.clone(), next);
        match next {
            Follow::Asking { .. } => None,
            Follow::Known { want, .. } => Some(want),
        }
    }

    pub fn following(&self, who: &Who) -> Option<bool> {
        match self.follows.get(who)? {
            Follow::Asking { .. } => None,
            Follow::Known { want, .. } => Some(*want),
        }
    }

    /// Learns whether the user follows `who` from their page, keeping a press made while it was asked for.
    fn followed(&mut self, who: &Who, is_following: bool) {
        let want = match self.follows.get(who) {
            Some(Follow::Asking { odd_presses }) => is_following ^ odd_presses,
            Some(Follow::Known { want, .. }) => *want,
            None => is_following,
        };
        self.follows.insert(
            who.clone(),
            Follow::Known {
                server: is_following,
                want,
            },
        );
    }

    /// Flips whether the user mutes `handle` and returns whether they now do. Anyone the logs show
    /// starts unmuted, since dev-null.sh leaves the people the user mutes out of them.
    pub fn mute(&mut self, handle: &str) -> bool {
        let mute = self.mutes.entry(handle.to_owned()).or_default();
        mute.want = !mute.want;
        self.generation += 1;
        mute.want
    }

    /// Changes how the user keeps `chat`, which shows as `shown`, and returns how they now want it.
    pub fn refile(&mut self, chat: &Chat, shown: Filed, change: Refile) -> Filed {
        let filing = self.filings.entry(chat.clone()).or_insert(Filing {
            server: shown,
            want: shown,
            hush: None,
            took: None,
        });
        match change {
            Refile::Pin(on) => filing.want.pinned = on,
            Refile::Stash(on) => filing.want.stashed = on,
            Refile::Mute(hush) => {
                filing.want.muted = hush.is_some();
                filing.hush = hush;
            }
        }
        filing.want
    }

    /// How the user wants `chat` kept, while dev-null.sh hasn't confirmed it yet.
    pub fn filed(&self, chat: &Chat) -> Option<Filed> {
        (self.filings.get(chat)).map(|filing| filing.want)
    }

    /// Whether the user mutes `handle`, as far as ttyl knows.
    pub fn muting(&self, handle: &str) -> bool {
        self.mutes.get(handle).is_some_and(|mute| mute.want)
    }

    /// Takes in people dev-null.sh says the user mutes, or with `blocks` blocks, short of a change
    /// the user made since.
    pub fn listed(&mut self, handles: impl IntoIterator<Item = String>, blocks: bool) {
        let switches = if blocks {
            &mut self.blocks
        } else {
            &mut self.mutes
        };
        for handle in handles {
            switches
                .entry(handle)
                .and_modify(|switch| switch.server = true)
                .or_insert(Switch {
                    server: true,
                    want: true,
                });
        }
        self.generation += 1;
    }

    /// The account's settings as the user wants them.
    pub fn config(&self) -> Config<'_> {
        Config::new(&self.me, self.settings.as_ref(), &self.wants)
    }

    /// Wants a setting changed, which goes to dev-null.sh once nothing else is changing it.
    pub fn configure(&mut self, change: Change) {
        self.wants.want(change);
        self.generation += 1;
    }

    /// Flips the commit between its translation and the original, asking dev-null.sh for the
    /// translation the first time. Says why instead when dev-null.sh found nothing to translate.
    pub fn translate(&mut self, id: &CommitId) -> Option<Untranslated> {
        match self.translations.get_mut(id) {
            None => {
                self.translations
                    .insert(id.clone(), Translating::Asking { show: true });
            }
            Some(Translating::Asking { show } | Translating::Got { show, .. }) => *show = !*show,
            Some(Translating::Skipped(why)) => return Some(why.clone()),
        }
        self.generation += 1;
        None
    }

    /// Settles the language from what dev-null.sh's settings say: the one the user picked, else the
    /// device's if dev-null.sh translates into it.
    fn settle_language(&mut self) {
        let Some(settings) = &self.settings else {
            return;
        };
        let device = (self.device_language.as_deref())
            .filter(|device| (settings.languages.iter()).any(|language| language.code == *device));
        let language = locale::target(settings.translation_language.as_deref(), device);
        self.language = Language::Settled(language);
    }

    /// A settings request that failed. A refusal is final, so the device's language stands. Anything
    /// else may pass, so the translations waiting on it drop and the next `t` asks again.
    pub fn settings_failed(&mut self, refused: bool) {
        match &self.language {
            Language::Unsettled if refused => {
                let device = self.device_language.as_deref();
                self.language = Language::Settled(locale::target(None, device));
            }
            Language::Unsettled => {
                (self.translations)
                    .retain(|_, translating| !matches!(translating, Translating::Asking { .. }));
                self.generation += 1;
            }
            Language::Settled(_) => {}
        }
    }

    /// Sets whether the user blocks `handle`. Anyone the logs show starts unblocked, since
    /// nothing crosses a block on dev-null.sh.
    pub fn block(&mut self, handle: &str, on: bool) {
        self.blocks.entry(handle.to_owned()).or_default().want = on;
        self.generation += 1;
    }

    pub fn blocking(&self, handle: &str) -> bool {
        self.blocks.get(handle).is_some_and(|block| block.want)
    }

    /// Whether `handle` follows the user, given what dev-null.sh `said`, short of a block since.
    pub fn follows_you(&self, handle: &str, said: bool) -> bool {
        said && !self.unfollowed_by.contains(handle)
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
        if let Some(pin) = &self.pin_want
            && !pending(&CallKey::Pin)
        {
            match rows.get(&pin.id) {
                Some(row) if row.commit.is_pinned != pin.on => calls.push(Call::Pin {
                    id: pin.id.clone(),
                    on: pin.on,
                }),
                Some(_) | None => self.pin_want = None,
            }
        }
        for (who, follow) in &self.follows {
            let handle = who.handle().to_owned();
            match *follow {
                Follow::Asking { .. } => match who {
                    Who::Person(_) if !pending(&CallKey::User(handle.clone())) => {
                        calls.push(Call::User(handle));
                    }
                    Who::Organization(_) if !pending(&CallKey::Organization(handle.clone())) => {
                        calls.push(Call::Organization(handle));
                    }
                    Who::Person(_) | Who::Organization(_) => {}
                },
                Follow::Known { server, want }
                    if server != want && !pending(&CallKey::Follow(handle)) =>
                {
                    calls.push(Call::Follow {
                        who: who.clone(),
                        on: want,
                    });
                }
                Follow::Known { .. } => {}
            }
        }
        for (handle, mute) in &self.mutes {
            if mute.server != mute.want && !pending(&CallKey::Mute(handle.clone())) {
                calls.push(Call::Mute {
                    handle: handle.clone(),
                    on: mute.want,
                });
            }
        }
        let mut asking = (self.translations.iter())
            .filter(|(_, translating)| matches!(translating, Translating::Asking { .. }))
            .map(|(id, _)| id);
        match &self.language {
            Language::Unsettled => {
                if !pending(&CallKey::Settings) && asking.next().is_some() {
                    calls.push(Call::Settings);
                }
            }
            Language::Settled(language) => {
                for id in asking.filter(|id| !pending(&CallKey::Translate((*id).clone()))) {
                    calls.push(Call::Translate {
                        id: id.clone(),
                        language: language.clone(),
                    });
                }
            }
        }
        for (handle, block) in &self.blocks {
            if block.server != block.want && !pending(&CallKey::Block(handle.clone())) {
                calls.push(Call::Block {
                    handle: handle.clone(),
                    on: block.want,
                });
            }
        }
        for (chat, filing) in &self.filings {
            if let Some(change) = filing.next()
                && !pending(&CallKey::File(chat.clone()))
            {
                calls.push(Call::File {
                    chat: chat.clone(),
                    change,
                });
            }
        }
        // A want equal to what dev-null.sh holds goes, unless a change before it may still land over it.
        let server = Config::new(&self.me, self.settings.as_ref(), Wants::NONE);
        self.wants.retain(|want| {
            pending(&CallKey::Configure(want.field()))
                || server.change(want.field()).as_ref() != Some(want)
        });
        for want in self.wants.iter() {
            if !pending(&CallKey::Configure(want.field())) {
                calls.push(Call::Configure(want.clone()));
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
            // A later `^` on another commit still stands.
            Call::Pin { id, .. } => {
                if self.pin_want.as_ref().is_some_and(|pin| pin.id == *id) {
                    self.pin_want = None;
                }
            }
            Call::Follow { who, .. } => {
                if let Some(Follow::Known { server, want }) = self.follows.get_mut(who) {
                    *want = *server;
                }
            }
            Call::Mute { handle, .. } => {
                if let Some(mute) = self.mutes.get_mut(handle) {
                    mute.want = mute.server;
                }
            }
            Call::Block { handle, .. } => {
                if let Some(block) = self.blocks.get_mut(handle) {
                    block.want = block.server;
                }
            }
            // The list reloads to show what dev-null.sh kept.
            Call::File { chat, .. } => {
                self.filings.remove(chat);
            }
            Call::User(handle) => {
                let who = Who::Person(handle.clone());
                if let Some(Follow::Asking { .. }) = self.follows.get(&who) {
                    self.follows.remove(&who);
                }
            }
            Call::Organization(handle) => {
                let who = Who::Organization(handle.clone());
                if let Some(Follow::Asking { .. }) = self.follows.get(&who) {
                    self.follows.remove(&who);
                }
            }
            Call::Delete(id) => {
                self.deleting.remove(id);
            }
            Call::Pastel(id) => {
                self.unsent_pastels.remove(id);
                return;
            }
            Call::Translate { id, .. } => {
                self.translations.remove(id);
            }
            Call::Configure(change) => {
                self.wants.retain(|want| want != change);
            }
            Call::Me
            | Call::Settings
            | Call::Muted { .. }
            | Call::Blocked { .. }
            | Call::Devices
            | Call::Revoke { .. }
            | Call::Redeem { .. }
            | Call::Page(_)
            | Call::Older { .. }
            | Call::Since { .. }
            | Call::Commit(_)
            | Call::Replies { .. }
            | Call::Vote { .. }
            | Call::Restore(_)
            | Call::Report { .. }
            | Call::Post { .. }
            | Call::Schedule { .. }
            | Call::Queue
            | Call::Unschedule(_)
            | Call::SendNow(_)
            | Call::Preview(_)
            | Call::Notifications { .. }
            | Call::Group { .. }
            | Call::Commits { .. }
            | Call::OrganizationCommits { .. }
            | Call::FollowedOrganizations { .. }
            | Call::People { .. }
            | Call::Starred { .. }
            | Call::SearchCommits(_)
            | Call::SearchPeople(_)
            | Call::Related(_)
            | Call::Headlines
            | Call::Headline(_)
            | Call::Conversations { .. }
            | Call::Answer { .. }
            | Call::Candidates(_)
            | Call::Messages { .. }
            | Call::MarkRead { .. }
            | Call::SendMessage { .. } => return,
        }
        self.generation += 1;
    }

    /// Takes in `reply` to `call`, which went out as `sent` while `next` is the next call to go
    /// out. A setting dev-null.sh confirmed changing since `sent` went out keeps its new value, as
    /// the reply may be from before the change.
    pub fn answer(&mut self, call: &Call, reply: Reply, sent: Ticket, next: Ticket) {
        let stale: Vec<Field> = match reply {
            Reply::Account(_) | Reply::Settings(_) => (self.confirmed.iter())
                .filter(|(_, confirmed)| sent < **confirmed)
                .map(|(field, _)| *field)
                .collect(),
            _ => Vec::new(),
        };
        let before = (!stale.is_empty()).then(|| (self.me.clone(), self.settings.clone()));
        self.apply(call, reply);
        if let Some((me, settings)) = before {
            for field in stale {
                field.carry(&me, &mut self.me);
                if let (Some(from), Some(to)) = (&settings, &mut self.settings) {
                    field.carry_regional(from, to);
                }
            }
            self.settle_language();
        }
        if let Call::Configure(change) = call {
            self.confirmed.insert(change.field(), next);
        }
    }

    pub fn apply(&mut self, call: &Call, reply: Reply) {
        match (call, reply) {
            (_, Reply::Account(account)) => {
                self.me = account.me;
                self.unread = account.unread;
                if let Call::Configure(change) = call
                    && let Some(settings) = &mut self.settings
                {
                    change.confirm(settings);
                    self.settle_language();
                }
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
            (Call::Star { .. } | Call::Vote { .. } | Call::Restore(_), Reply::Commit(focus)) => {
                self.keep_counted(focus.card, Some(focus.replies_count));
                self.generation += 1;
            }
            (Call::Pin { id, on }, Reply::Commit(focus)) => {
                self.keep_counted(focus.card, Some(focus.replies_count));
                if *on {
                    unpin_others(&mut self.rows, &self.me.handle, id);
                }
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
            (Call::User(handle), Reply::User(profile)) => {
                let is_following = profile.is_following;
                if let Some(mut pinned) = profile.pinned {
                    pinned.commit.is_pinned = true;
                    let commit = &pinned.commit;
                    unpin_others(&mut self.rows, &commit.author.handle, &commit.id);
                    self.land_row(pinned);
                    self.generation += 1;
                }
                self.followed(&Who::Person(handle.clone()), is_following);
            }
            (Call::Organization(handle), Reply::Organization(page)) => {
                self.followed(&Who::Organization(handle.clone()), page.is_following);
            }
            (_, Reply::Rows(page)) => {
                self.absorb_rows(page.rows);
            }
            (_, Reply::Headline(page)) => {
                self.absorb_rows(page.rows);
            }
            (_, Reply::Headlines(_)) => {}
            (Call::Muted { .. } | Call::Blocked { .. }, Reply::People(page)) => {
                let blocks = matches!(call, Call::Blocked { .. });
                self.listed(
                    page.people.into_iter().map(|person| person.user.handle),
                    blocks,
                );
            }
            (_, Reply::People(page)) => {
                for person in page.people {
                    if let Some(server) = person.is_followed {
                        self.follows
                            .entry(Who::Person(person.user.handle))
                            .or_insert(Follow::Known {
                                server,
                                want: server,
                            });
                    }
                }
                self.generation += 1;
            }
            (_, Reply::Organizations(list)) => {
                for organization in list.organizations {
                    if let Some(server) = organization.is_followed {
                        self.follows
                            .entry(Who::Organization(organization.handle))
                            .or_insert(Follow::Known {
                                server,
                                want: server,
                            });
                    }
                }
                self.generation += 1;
            }
            (Call::Follow { who, on }, Reply::Done) => {
                if let Some(Follow::Known { server, .. }) = self.follows.get_mut(who) {
                    *server = *on;
                }
            }
            (Call::Mute { handle, on }, Reply::Done) => {
                if let Some(mute) = self.mutes.get_mut(handle) {
                    mute.server = *on;
                }
            }
            (Call::File { chat, change }, Reply::Done) => {
                if let Some(filing) = self.filings.get_mut(chat) {
                    match *change {
                        Refile::Pin(on) => filing.server.pinned = on,
                        Refile::Stash(on) => filing.server.stashed = on,
                        Refile::Mute(hush) => {
                            filing.server.muted = hush.is_some();
                            filing.took = hush;
                        }
                    }
                    if filing.next().is_none() {
                        self.filings.remove(chat);
                    }
                }
            }
            (Call::Settings, Reply::Settings(settings)) => {
                self.settings = Some(settings);
                self.settle_language();
                self.generation += 1;
            }
            (Call::Translate { id, .. }, Reply::Translation(translation)) => {
                if let Some(Translating::Asking { show }) = self.translations.get(id) {
                    let landed = match *translation {
                        Translation::Done(translated) => Translating::Got {
                            translated: Box::new(translated),
                            show: *show,
                        },
                        Translation::Skipped(why) => Translating::Skipped(why),
                    };
                    self.translations.insert(id.clone(), landed);
                    self.generation += 1;
                }
            }
            (Call::Block { handle, on }, Reply::Done) => {
                if let Some(block) = self.blocks.get_mut(handle) {
                    block.server = *on;
                }
                // Blocking ends the follows both ways, and unblocking leaves them ended.
                if *on {
                    self.follows.insert(
                        Who::Person(handle.clone()),
                        Follow::Known {
                            server: false,
                            want: false,
                        },
                    );
                    self.unfollowed_by.insert(handle.clone());
                    self.generation += 1;
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
                | Reply::Organization(_)
                | Reply::Notifications(_)
                | Reply::Group(_)
                | Reply::Preview(_)
                | Reply::Settings(_)
                | Reply::Devices(_)
                | Reply::Translation(_)
                | Reply::Done
                | Reply::Scheduled(_)
                | Reply::Queue(_)
                | Reply::Submitted { .. }
                | Reply::Conversations(_)
                | Reply::Candidates(_)
                | Reply::Messages(_)
                | Reply::Message(_),
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
        let author = &row.commit.author.handle;
        let fold = if self.blocking(author) {
            Some(Fold::Blocked)
        } else {
            (self.mutes.get(author))
                .is_some_and(|mute| mute.want)
                .then_some(Fold::Muted)
        };
        let translating = self.translations.get(id);
        let asking = matches!(translating, Some(Translating::Asking { show: true }));
        let translated = match translating {
            Some(Translating::Got {
                translated,
                show: true,
            }) => Some(translated),
            _ => None,
        };
        let pinned = self.is_pinned(row);
        if want.is_none()
            && !deleting
            && picking.is_none()
            && fold.is_none()
            && !asking
            && translated.is_none()
            && pinned == row.commit.is_pinned
        {
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
        row.fold = fold;
        if asking {
            row.showing = Showing::Translating;
        }
        if let Some(translated) = translated {
            row.commit.body = translated.body.clone();
            row.commit.body_segments = translated.body_segments.clone();
            row.commit.preview_segments = translated.preview_segments.clone();
            // They point into the original's text.
            row.commit.mentions.clear();
            row.showing = Showing::Translation {
                from: translated.from.clone(),
            };
        }
        row.commit.is_pinned = pinned;
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
                        .unwrap_or_default();
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
                            fold: None,
                            showing: Showing::Original,
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
            .unwrap_or_default();
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
                fold: None,
                showing: Showing::Original,
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
    /// thread fetches the replies that came in, or left, since. A row that came without a reply
    /// preview keeps the one stored, unless the count moved and it may name a reply since gone.
    fn land_row(&mut self, mut row: Row) -> CommitId {
        let id = row.commit.id.clone();
        if let Some(old) = self.rows.get_mut(&id) {
            if old.replies_count != row.replies_count {
                self.replies.remove(&id);
                self.reply_refills.remove(&id);
            } else if row.reply_preview.is_none() {
                row.reply_preview = old.reply_preview.take();
            }
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

    /// Remembers the newest commit a log showed the last run, so its feed marks where the reader left off.
    pub fn leave_off(&mut self, page: &LogPage) {
        let newest = page.rows.iter().map(|row| row.commit.created_at).max();
        if let (false, Some(at)) = (page.is_ranked, newest) {
            self.left_off.insert(page.log, at);
        }
    }

    pub fn left_off(&self, log: Log) -> Option<DateTime<Utc>> {
        self.left_off.get(&log).copied()
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
