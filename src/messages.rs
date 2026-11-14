use std::ops::Range;
use std::time::Instant;

use chrono::{DateTime, TimeZone};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::api::{Call, Failure, Paging};
use crate::compose;
use crate::device::Nonce;
use crate::emoji::{self, Emoji};
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed;
use crate::keys::{
    self, Act, Action, CONVERSATION_SCOPES, Keep, MESSAGE_SCOPES, REACT_SCOPES, Scope,
};
use crate::model::{
    Access, Candidate, Chat, ClientId, Conversation, ConversationPage, Filed, Folder, Message,
    MessageId, MessagePage, Party, Reaction, ReactionsVersion, ReplyTo, User, UserId,
};
use crate::popup;
use crate::text::Buffer;
use crate::theme;
use crate::thread::LOADING_NOTE_DELAY;

pub const TITLE: &str = "messages";
/// dev-null.sh keeps a message to this many characters.
pub const MAX_CHARS: usize = 2000;
pub const NO_ANSWER: &str = "fatal: no answer from dev-null.sh. it may have gone through; the conversation shows it if it did.";

/// Pinned first, then unread, then the rest, each group in dev-null.sh's order of newest first.
pub fn order(conversations: &mut [Conversation]) {
    conversations
        .sort_by_key(|conversation| (!conversation.filed.pinned, conversation.unread == 0));
}

/// Why a message can't go as written, before anything is sent.
pub fn refusal(body: &str) -> Option<String> {
    let count = body.chars().count();
    if body.trim().is_empty() {
        Some("nothing to send. the message is empty.".to_owned())
    } else if count > MAX_CHARS {
        Some(format!(
            "messages stop at {MAX_CHARS} characters. this one has {count}."
        ))
    } else {
        None
    }
}

/// One folder of conversations: pinned first, then unread, then the rest, each in dev-null.sh's
/// order of newest first. One line each, like a mail index. A find narrows the folder to a name
/// and lists the people it matches under it, to start a conversation with.
pub struct Conversations {
    folder: Folder,
    /// What `/` narrowed the folder to, empty for all of it.
    query: String,
    loaded: Option<Loaded>,
    /// The people the find matches, once dev-null.sh says.
    people: Vec<Candidate>,
    /// Requests waiting on the user, as the last page said.
    requests: u32,
    failure: Option<Failure>,
    asked_at: Instant,
    selected: usize,
    offset: usize,
}

struct Loaded {
    rows: Vec<Conversation>,
    next: Option<String>,
}

/// What the selected row of the list is.
pub enum Picked<'a> {
    Conversation(&'a Conversation),
    /// Someone the find matched, to start a conversation with.
    Person(&'a Candidate),
}

impl Conversations {
    pub fn new(now: Instant) -> Self {
        Conversations {
            folder: Folder::Open,
            query: String::new(),
            loaded: None,
            people: Vec::new(),
            requests: 0,
            failure: None,
            asked_at: now,
            selected: 0,
            offset: 0,
        }
    }

    /// The folder's first page.
    pub fn call(&self) -> Call {
        Call::Conversations {
            folder: self.folder,
            query: self.query.clone(),
            before: None,
        }
    }

    /// The people the find matches, when there is one.
    pub fn people_call(&self) -> Option<Call> {
        (!self.query.is_empty()).then(|| Call::Candidates(self.query.clone()))
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether a page or a failure is for the folder and find on screen.
    pub fn shows(&self, folder: Folder, query: &str) -> bool {
        self.folder == folder && self.query == query
    }

    fn rows(&self) -> &[Conversation] {
        self.loaded
            .as_ref()
            .map_or(&[], |loaded| loaded.rows.as_slice())
    }

    /// The people the find matched, but for anyone the user already talks to in the list.
    fn people(&self) -> Vec<&Candidate> {
        let rows = self.rows();
        (self.people.iter())
            .filter(|candidate| {
                let chat = Chat::Direct(candidate.user.handle.clone());
                !rows.iter().any(|row| row.party.is(&chat))
            })
            .collect()
    }

    fn count(&self) -> usize {
        self.rows().len() + self.people().len()
    }

    pub fn selected(&self) -> Option<Picked<'_>> {
        let rows = self.rows();
        match rows.get(self.selected) {
            Some(conversation) => Some(Picked::Conversation(conversation)),
            None => (self.people().get(self.selected - rows.len()))
                .map(|candidate| Picked::Person(candidate)),
        }
    }

    fn selected_conversation(&self) -> Option<&Conversation> {
        match self.selected()? {
            Picked::Conversation(conversation) => Some(conversation),
            Picked::Person(_) => None,
        }
    }

    /// A failed reload over rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        self.loaded.as_ref()?;
        self.failure.as_ref()
    }

    pub fn can(&self, action: Action) -> bool {
        let requests = self.folder == Folder::Requests;
        let conversation = self.selected_conversation();
        match action {
            Action::Open => self.selected().is_some(),
            Action::Back
            | Action::Refresh
            | Action::Find
            | Action::NextFilter
            | Action::PreviousFilter => true,
            Action::Down | Action::Up | Action::Top | Action::Bottom => self.count() > 0,
            Action::Keep(Keep::Pin | Keep::Stash | Keep::Mute) => {
                !requests && conversation.is_some()
            }
            Action::Keep(Keep::Merge | Keep::Drop) => {
                requests && conversation.is_some_and(|row| matches!(row.party, Party::Person(_)))
            }
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action) {
        let last = self.count().saturating_sub(1);
        self.selected = match action {
            Action::Down => (self.selected + 1).min(last),
            Action::Up => self.selected.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => self.selected,
        };
    }

    /// The folders to cycle through: requests only while some wait, or while it is open.
    fn folders(&self) -> Vec<Folder> {
        (Folder::ALL.into_iter())
            .filter(|folder| {
                *folder != Folder::Requests || self.requests > 0 || self.folder == *folder
            })
            .collect()
    }

    /// The folders as tabs, requests with how many wait, and which one is open.
    pub fn tabs(&self) -> (Vec<String>, usize) {
        let folders = self.folders();
        let names = (folders.iter())
            .map(|folder| match folder {
                Folder::Requests if self.requests > 0 => format!("requests {}", self.requests),
                folder => folder.slug().to_owned(),
            })
            .collect();
        let at = (folders.iter()).position(|folder| *folder == self.folder);
        (names, at.unwrap_or(0))
    }

    /// Opens the next folder, or the one before with `back`, keeping the find.
    pub fn turn(&mut self, back: bool, now: Instant) {
        let folders = self.folders();
        let at = (folders.iter()).position(|folder| *folder == self.folder);
        let count = folders.len();
        let next = match (at, back) {
            (Some(at), false) => (at + 1) % count,
            (Some(at), true) => (at + count - 1) % count,
            (None, _) => 0,
        };
        self.folder = folders[next];
        self.restart(now);
    }

    /// Narrows the folder to `query`, or with an empty one shows all of it again.
    pub fn find(&mut self, query: &str, now: Instant) {
        self.query = query.trim().to_owned();
        self.people.clear();
        self.restart(now);
    }

    fn restart(&mut self, now: Instant) {
        self.loaded = None;
        self.failure = None;
        self.selected = 0;
        self.offset = 0;
        self.asked_at = now;
    }

    pub fn reload(&mut self, now: Instant) {
        self.asked_at = now;
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        (self.loaded.is_none() && self.failure.is_none())
            .then_some(self.asked_at + LOADING_NOTE_DELAY)
    }

    /// The next page, once the selection nears the end.
    pub fn wants_older(&self) -> Option<Call> {
        let loaded = self.loaded.as_ref()?;
        let before = loaded.next.clone()?;
        let left = loaded.rows.len().saturating_sub(self.selected + 1);
        (left <= AHEAD && self.failure.is_none()).then(|| Call::Conversations {
            folder: self.folder,
            query: self.query.clone(),
            before: Some(before),
        })
    }

    /// A first page replaces the list and keeps the selection on its conversation; a later one adds
    /// the conversations it brings. Either leaves out what the user moved to another folder.
    pub fn land(&mut self, before: Option<&str>, page: ConversationPage) {
        self.requests = page.requests;
        let kept = self.kept();
        self.failure = None;
        let folder = self.folder;
        let conversations = (page.conversations.into_iter()).filter(|row| folder.holds(row.filed));
        match (before, &mut self.loaded) {
            (None, _) => {
                self.loaded = Some(Loaded {
                    rows: conversations.collect(),
                    next: page.next,
                });
            }
            (Some(before), Some(loaded)) if loaded.next.as_deref() == Some(before) => {
                for conversation in conversations {
                    let chat = conversation.party.chat();
                    if !loaded.rows.iter().any(|row| row.party.is(&chat)) {
                        loaded.rows.push(conversation);
                    }
                }
                loaded.next = page.next.filter(|next| next != before);
            }
            (Some(_), _) => return,
        }
        if let Some(loaded) = &mut self.loaded {
            order(&mut loaded.rows);
        }
        self.reselect(kept);
    }

    /// The people `query` matches, if the find on screen is still for it.
    pub fn land_people(&mut self, query: &str, people: Vec<Candidate>) {
        if self.query == query {
            let kept = self.kept();
            self.people = people;
            self.reselect(kept);
        }
    }

    pub fn fail(&mut self, failure: Failure) {
        self.failure = Some(failure);
    }

    /// The conversation `chat` was just read, so its count goes.
    pub fn read(&mut self, chat: &Chat) {
        if let Some(loaded) = &mut self.loaded {
            for row in &mut loaded.rows {
                if row.party.is(chat) {
                    row.unread = 0;
                }
            }
        }
    }

    /// Shows `chat` kept as `filed`. A conversation moved to another folder stays where it is,
    /// marked, until the folder is listed again, so the same key takes it back.
    pub fn refile(&mut self, chat: &Chat, filed: Filed) {
        let kept = self.kept();
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        for row in loaded.rows.iter_mut().filter(|row| row.party.is(chat)) {
            row.filed = filed;
        }
        order(&mut loaded.rows);
        self.reselect(kept);
    }

    /// The request from `handle` was merged or dropped, so it leaves the requests.
    pub fn answered(&mut self, handle: &str) {
        let chat = Chat::Direct(handle.to_owned());
        if self.folder == Folder::Requests
            && let Some(loaded) = &mut self.loaded
        {
            loaded.rows.retain(|row| !row.party.is(&chat));
        }
        self.requests = self.requests.saturating_sub(1);
        self.selected = self.selected.min(self.count().saturating_sub(1));
    }

    /// Who the selection is on, a conversation or a person the find matched, to keep it there as
    /// rows and people come and go.
    fn kept(&self) -> Option<Chat> {
        Some(match self.selected()? {
            Picked::Conversation(conversation) => conversation.party.chat(),
            Picked::Person(candidate) => Chat::Direct(candidate.user.handle.clone()),
        })
    }

    /// Puts the selection back on `kept`, as a conversation or else as a person, or where it was
    /// when that left the list.
    fn reselect(&mut self, kept: Option<Chat>) {
        let found = kept.and_then(|chat| {
            let rows = self.rows();
            (rows.iter().position(|row| row.party.is(&chat))).or_else(|| {
                (self.people().iter())
                    .position(|candidate| matches!(&chat, Chat::Direct(handle) if *handle == candidate.user.handle))
                    .map(|at| rows.len() + at)
            })
        });
        self.selected = found.unwrap_or(self.selected.min(self.count().saturating_sub(1)));
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        me: &str,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        let width = usize::from(area.width);
        let Some(loaded) = &self.loaded else {
            let lines = match &self.failure {
                Some(failure) => fatal::screen(failure, None),
                None if self.placeholder_at().is_some_and(|at| now >= at) => {
                    vec![Line::styled("loading messages", theme::muted())]
                }
                None => Vec::new(),
            };
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
            return;
        };
        let people = self.people();
        if loaded.rows.is_empty() && people.is_empty() && self.query.is_empty() {
            let lines = self.empty(width);
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        let mut lines: Vec<Line> = Vec::new();
        let mut at_line = Vec::new();
        if !self.query.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("/{}", self.query),
                    Style::new().fg(theme::FOREGROUND),
                ),
                Span::styled("  esc shows all", theme::muted()),
            ]));
        }
        if loaded.rows.is_empty() && !self.query.is_empty() {
            let none = format!("no {} conversations match", self.folder.slug());
            lines.push(Line::styled(none, theme::muted()));
        }
        let selected = self.selected;
        for (at, conversation) in loaded.rows.iter().enumerate() {
            at_line.push(lines.len());
            lines.push(row(
                conversation,
                at == selected,
                self.folder,
                width,
                me,
                wall,
            ));
        }
        if !people.is_empty() {
            lines.push(Line::styled("write to", theme::muted()));
            for (at, candidate) in people.iter().enumerate() {
                at_line.push(lines.len());
                let picked = loaded.rows.len() + at == selected;
                lines.push(person(candidate, picked, width));
            }
        }
        let height = usize::from(area.height).max(1);
        let line = at_line.get(selected).copied().unwrap_or(0);
        if line < self.offset {
            // Keeps the find's line in view while the selection is near the top.
            self.offset = if selected == 0 { 0 } else { line };
        } else if line >= self.offset + height {
            self.offset = line + 1 - height;
        }
        let shown: Vec<Line> = lines.into_iter().skip(self.offset).take(height).collect();
        frame.render_widget(Paragraph::new(shown), area);
    }

    /// What an empty folder says.
    fn empty(&self, width: usize) -> Vec<Line<'static>> {
        let note = match self.folder {
            Folder::Open => {
                let key = keys::key_for(Action::Commit(Act::Message)).unwrap_or_default();
                format!("No conversations yet. Press {key} on a commit or a person to start one.")
            }
            Folder::Requests => "No requests wait on you.".to_owned(),
            Folder::Stashed => {
                let key = keys::key_for(Action::Keep(Keep::Stash)).unwrap_or_default();
                format!("Nothing stashed. {key} on a conversation stashes it until someone writes.")
            }
        };
        let path = match self.folder {
            Folder::Open => "$ ls /dev/remotes".to_owned(),
            folder => format!("$ ls /dev/remotes/{}", folder.slug()),
        };
        let mut lines = vec![vec![Span::styled(path, Style::new().fg(theme::FOREGROUND))]];
        feed::wrap(&[(note, theme::muted())], width, &mut lines);
        lines.into_iter().map(Line::from).collect()
    }
}

/// `+ Grace Hopper @grace`, or dimmed with why they can't be messaged.
fn person(candidate: &Candidate, selected: bool, width: usize) -> Line<'static> {
    let user = &candidate.user;
    let name = match candidate.reason {
        Some(_) => theme::muted(),
        None => Style::new().fg(theme::lane(user.lane)),
    };
    let mut spans = vec![
        Span::styled("+ ", name),
        Span::styled(user.name.clone(), name),
        Span::styled(format!(" @{}", user.handle), theme::muted()),
    ];
    if let Some(reason) = &candidate.reason {
        spans.push(Span::styled(format!("  {reason}"), theme::muted()));
    }
    let mut spans = feed::fit(spans, width);
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let line = Line::from(spans);
    match selected {
        true => line.style(Style::new().bg(theme::MUTED)),
        false => line,
    }
}

/// `● Grace Hopper @grace (2)  did you see the patch?          3m`, or for an org
/// `● #unix-room 5 members (4)  grace: pushed v2          3m`, filling `width`. A conversation
/// just moved out of `folder` says where it went.
fn row<Tz: TimeZone>(
    conversation: &Conversation,
    selected: bool,
    folder: Folder,
    width: usize,
    me: &str,
    wall: &DateTime<Tz>,
) -> Line<'static> {
    let unread = conversation.unread > 0;
    let bold = |style: Style| match unread {
        true => style.add_modifier(Modifier::BOLD),
        false => style,
    };
    let dot = if unread { "● " } else { "○ " };
    let mut spans = match &conversation.party {
        Party::Person(partner) => {
            let name = bold(Style::new().fg(theme::lane(partner.lane)));
            vec![
                Span::styled(dot, name),
                Span::styled(partner.name.clone(), name),
                Span::styled(format!(" @{}", partner.handle), theme::muted()),
            ]
        }
        Party::Org(org) => {
            let name = bold(Style::new().fg(theme::FOREGROUND));
            let count = org.members.len();
            let noun = if count == 1 { "member" } else { "members" };
            vec![
                Span::styled(dot, name),
                Span::styled(conversation.party.title(), name),
                Span::styled(format!(" {count} {noun}"), theme::muted()),
            ]
        }
    };
    if unread {
        spans.push(Span::styled(
            format!(" ({})", conversation.unread),
            Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(last) = &conversation.last {
        spans.push(Span::raw("  "));
        let by = match other(&conversation.party, me, last) {
            _ if last.notice.is_some() => None,
            None => Some("you"),
            Some(_) if matches!(conversation.party, Party::Person(_)) => None,
            Some(author) => Some(author.handle.as_str()),
        };
        if let Some(by) = by {
            spans.push(Span::styled(format!("{by}: "), theme::muted()));
        }
        let text = match last.body.is_empty() || last.notice.is_some() {
            true => theme::muted(),
            false => Style::new().fg(theme::FOREGROUND),
        };
        spans.push(Span::styled(last.preview().into_owned(), text));
    }
    let filed = conversation.filed;
    let moved = match filed.stashed {
        true => "stashed",
        false => "unstashed",
    };
    let marks = [
        (filed.pinned, "pinned"),
        (filed.muted, "muted"),
        (!folder.holds(filed), moved),
    ];
    let age = (marks.iter())
        .filter_map(|(on, mark)| on.then_some(*mark))
        .map(str::to_owned)
        .chain(conversation.last_at.map(|at| feed::relative_time(at, wall)))
        .collect::<Vec<_>>()
        .join(" ");
    let room = width.saturating_sub(age.width() + 1);
    let mut spans = feed::fit(spans, room);
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + age.width())),
    ));
    spans.push(Span::styled(age, theme::muted()));
    let line = Line::from(spans);
    match selected {
        true => line.style(Style::new().bg(theme::MUTED)),
        false => line,
    }
}

/// Who wrote `message` when it wasn't the user. An org's thread names each author; a message the
/// user just sent there doesn't, so the members name it.
fn other<'a>(party: &'a Party, me: &str, message: &'a Message) -> Option<&'a User> {
    match party {
        Party::Person(partner) => (message.user_id == partner.id).then_some(partner),
        Party::Org(org) => (message.author.as_ref())
            .or_else(|| (org.members.iter()).find(|member| member.id == message.user_id))
            .filter(|author| author.handle != me),
    }
}

/// Whether `message` is one the others said, which dev-null.sh counts as unread until marked. An
/// org's notices don't count.
fn heard(party: &Party, me: &str, message: &Message) -> bool {
    message.notice.is_none() && other(party, me, message).is_some()
}

/// A conversation as an IRC client shows a query window, or for an org a channel: oldest at the
/// top, newest at the bottom where the view stays, and the line being typed under it.
pub struct ConversationScreen {
    party: Party,
    /// The user's handle, the nick their own messages show under.
    me: String,
    title: String,
    thread: Option<Thread>,
    failure: Option<Failure>,
    asked_at: Instant,
    mode: Mode,
    input: Buffer,
    /// The message the next send answers.
    reply: Option<Replying>,
    /// The failed send whose id goes again while its text is unchanged.
    retry: Option<Outgoing>,
    /// Sent and not yet answered, shown dimmed under the conversation.
    outbox: Vec<Outgoing>,
    /// Emoji the user left or took back that dev-null.sh hasn't answered for yet, which show as
    /// done meanwhile.
    wants: Vec<Want>,
    view: View,
    /// How many rows the last frame had for messages: a page.
    page: usize,
    /// Whether the last frame reached the oldest loaded message.
    at_top: bool,
    older_failed: bool,
}

struct Thread {
    /// Oldest first, each id once.
    messages: Vec<Message>,
    has_older: bool,
    read_through: MessageId,
    /// How far the partner has read, when dev-null.sh says.
    partner_read: Option<MessageId>,
    access: Access,
    /// The newest message a poll brought. A message this screen sent can be newer than one the
    /// partner sent before it, so polling after the newest message shown would skip theirs.
    polled: Option<MessageId>,
    /// The reactions version the last page brought, so the next poll hears only of changes since.
    reactions: ReactionsVersion,
}

impl Thread {
    /// Puts `message` in its place by id, or over the copy already there, whose reactions may be
    /// older. Says whether it was new.
    fn place(&mut self, message: Message) -> bool {
        let at = (self.messages).partition_point(|each| each.id < message.id);
        match self.messages.get_mut(at) {
            Some(known) if known.id == message.id => {
                *known = message;
                false
            }
            _ => {
                self.messages.insert(at, message);
                true
            }
        }
    }

    fn message(&self, id: MessageId) -> Option<&Message> {
        let at = self
            .messages
            .binary_search_by_key(&id, |each| each.id)
            .ok()?;
        Some(&self.messages[at])
    }

    fn message_mut(&mut self, id: MessageId) -> Option<&mut Message> {
        let at = self
            .messages
            .binary_search_by_key(&id, |each| each.id)
            .ok()?;
        Some(&mut self.messages[at])
    }

    /// The messages someone said, oldest first, which can be selected. Events can't be answered.
    fn said(&self) -> impl DoubleEndedIterator<Item = &Message> {
        self.messages.iter().filter(|message| !message.is_event())
    }
}

/// What keys do in a conversation, split the way vim splits its modes: typing goes into the
/// input, a selected message takes single keys, and reacting takes an emoji's name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Typing,
    Selecting(MessageId),
    Reacting {
        message: MessageId,
        query: String,
        /// The emoji chosen among those offered.
        at: usize,
    },
}

/// What the reaction prompt offers before a name is typed, after the emoji already on the message.
const REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "🎉", "👀", "✅"];

/// The message the next send answers, with its start as the input shows it, which stays even once
/// the message is no longer loaded.
#[derive(Debug, Clone)]
struct Replying {
    id: MessageId,
    quote: String,
}

#[derive(Debug)]
struct Want {
    message: MessageId,
    emoji: String,
    on: bool,
}

/// One emoji on a message: how many left it, and whether the user did.
struct Tally<'a> {
    emoji: &'a str,
    count: usize,
    mine: bool,
}

#[derive(Debug, Clone)]
struct Outgoing {
    client_id: ClientId,
    body: String,
    reply: Option<Replying>,
}

/// What was typed in a conversation and not sent, kept while it is closed.
#[derive(Debug, Default)]
pub struct Unsent {
    text: String,
    retry: Option<Outgoing>,
    reply: Option<Replying>,
}

impl Unsent {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reply.is_none()
    }

    /// Puts a failed send's text back in front of what was typed since. Sent again unchanged, it
    /// goes with the same id, so dev-null.sh keeps one copy if the first one did land.
    pub fn restore(&mut self, client_id: ClientId, body: String) {
        self.text = match self.text.as_str() {
            "" => body.clone(),
            typed => format!("{body}\n{typed}"),
        };
        self.retry = Some(Outgoing {
            client_id,
            body,
            reply: None,
        });
    }
}

/// Where the view sits: on the newest message, following new ones in, or scrolled back.
enum View {
    Stuck,
    /// `lines` up from the bottom as the last frame drew it, when `newest` was the newest
    /// message, with `unseen` messages from the partner come in since.
    Back {
        lines: usize,
        newest: Option<MessageId>,
        unseen: usize,
    },
}

impl ConversationScreen {
    /// `unsent` is what was typed here before the conversation was last left.
    pub fn new(party: Party, me: String, unsent: Unsent, now: Instant) -> Self {
        ConversationScreen {
            title: party.title(),
            party,
            me,
            thread: None,
            failure: None,
            asked_at: now,
            mode: Mode::Typing,
            input: Buffer::new(&unsent.text),
            reply: unsent.reply,
            retry: unsent.retry,
            outbox: Vec::new(),
            wants: Vec::new(),
            view: View::Stuck,
            page: 0,
            at_top: false,
            older_failed: false,
        }
    }

    /// Which conversation this is, and what was typed and not sent, to keep for when it opens
    /// again.
    pub fn leave(self) -> (Chat, Unsent) {
        let unsent = Unsent {
            text: self.input.text().to_owned(),
            retry: self.retry,
            reply: self.reply,
        };
        (self.party.chat(), unsent)
    }

    pub fn chat(&self) -> Chat {
        self.party.chat()
    }

    pub fn is(&self, chat: &Chat) -> bool {
        self.party.is(chat)
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn scopes(&self) -> &'static [Scope] {
        match self.mode {
            Mode::Typing => CONVERSATION_SCOPES,
            Mode::Selecting(_) => MESSAGE_SCOPES,
            Mode::Reacting { .. } => REACT_SCOPES,
        }
    }

    /// Whether a key that does nothing here is text, for the input or an emoji's name.
    pub fn takes_text(&self) -> bool {
        !matches!(self.mode, Mode::Selecting(_))
    }

    /// The latest messages, or once they are in, the ones since and the reactions changed since.
    pub fn poll(&self) -> Call {
        let paging = match &self.thread {
            Some(Thread {
                polled: Some(after),
                reactions,
                ..
            }) => Paging::Since {
                after: *after,
                reactions: *reactions,
            },
            _ => Paging::Latest,
        };
        Call::Messages {
            chat: self.chat(),
            paging,
        }
    }

    /// How far the user has read here, as far as ttyl knows.
    pub fn read_through(&self) -> Option<MessageId> {
        self.thread.as_ref().map(|thread| thread.read_through)
    }

    /// The page before the oldest message, once the view reaches it.
    pub fn wants_older(&self) -> Option<Call> {
        let thread = self.thread.as_ref()?;
        let oldest = thread.messages.first()?;
        (thread.has_older && self.at_top && !self.older_failed).then(|| Call::Messages {
            chat: self.chat(),
            paging: Paging::Before(oldest.id),
        })
    }

    pub fn placeholder_at(&self) -> Option<Instant> {
        (self.thread.is_none() && self.failure.is_none())
            .then_some(self.asked_at + LOADING_NOTE_DELAY)
    }

    fn writable(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| thread.access == Access::Open)
    }

    pub fn can(&self, action: Action) -> bool {
        let thread = self.thread.as_ref();
        let said = thread.is_some_and(|thread| thread.said().next().is_some());
        match (&self.mode, action) {
            (Mode::Typing, Action::Send | Action::Editor) => self.writable(),
            (Mode::Typing, Action::PageUp) => thread.is_some(),
            (Mode::Typing, Action::PageDown) => matches!(self.view, View::Back { .. }),
            // esc does the first of these that can be done.
            (Mode::Typing, Action::Close) => self.reply.is_some(),
            (Mode::Typing, Action::Select) => self.reply.is_none() && said,
            (Mode::Typing, Action::Back) => self.reply.is_none() && !said,
            (Mode::Typing, Action::Up | Action::Down) => said,
            (Mode::Typing | Mode::Selecting(_), Action::Ping) => {
                self.writable() && thread.is_some_and(|thread| !thread.messages.is_empty())
            }
            (Mode::Selecting(_), Action::Up | Action::Down | Action::Type | Action::Back) => true,
            (Mode::Selecting(_), Action::Reply | Action::React) => self.writable(),
            (Mode::Reacting { .. }, Action::Close) => true,
            (Mode::Reacting { .. }, Action::Next | Action::Previous | Action::Confirm) => {
                !self.choices().is_empty()
            }
            _ => false,
        }
    }

    /// Does what a key asks of the conversation, and says what to send dev-null.sh for it.
    pub fn act(&mut self, action: Action) -> Option<Call> {
        match (&self.mode, action) {
            (_, Action::Ping) => return Some(Call::Ping(self.chat())),
            (Mode::Typing, Action::PageUp | Action::PageDown) => self.scroll(action),
            (Mode::Typing, Action::Close) => self.reply = None,
            (Mode::Typing, Action::Select | Action::Up | Action::Down) => {
                let newest = (self.thread.as_ref()).and_then(|thread| thread.said().next_back());
                if let Some(newest) = newest {
                    self.mode = Mode::Selecting(newest.id);
                }
            }
            (Mode::Selecting(id), Action::Up | Action::Down) => {
                let id = *id;
                let next = self.thread.as_ref().and_then(|thread| match action {
                    Action::Up => thread.said().rev().find(|message| message.id < id),
                    _ => thread.said().find(|message| message.id > id),
                });
                if let Some(next) = next {
                    self.mode = Mode::Selecting(next.id);
                }
            }
            (Mode::Selecting(_), Action::Type) => self.mode = Mode::Typing,
            (Mode::Selecting(id), Action::Reply) => {
                let id = *id;
                self.reply = self.replying(id);
                self.mode = Mode::Typing;
            }
            (Mode::Selecting(id), Action::React) => {
                self.mode = Mode::Reacting {
                    message: *id,
                    query: String::new(),
                    at: 0,
                };
            }
            (Mode::Reacting { message, .. }, Action::Close) => {
                self.mode = Mode::Selecting(*message);
            }
            (Mode::Reacting { .. }, Action::Next | Action::Previous) => {
                let count = self.choices().len().max(1);
                if let Mode::Reacting { at, .. } = &mut self.mode {
                    *at = match action {
                        Action::Next => (*at + 1) % count,
                        _ => (*at + count - 1) % count,
                    };
                }
            }
            (Mode::Reacting { message, at, .. }, Action::Confirm) => {
                let (message, at) = (*message, *at);
                let emoji = self.choices().get(at)?.glyph;
                self.mode = Mode::Selecting(message);
                return Some(self.toggle(message, emoji));
            }
            _ => {}
        }
        None
    }

    fn scroll(&mut self, action: Action) {
        let back = match self.view {
            View::Stuck => 0,
            View::Back { lines, .. } => lines,
        };
        let back = match action {
            Action::PageUp => back + self.page.saturating_sub(1).max(1),
            _ => back.saturating_sub(self.page.saturating_sub(1).max(1)),
        };
        self.view = match (back, &self.view) {
            (0, _) => View::Stuck,
            (lines, View::Back { newest, unseen, .. }) => View::Back {
                lines,
                newest: *newest,
                unseen: *unseen,
            },
            (lines, View::Stuck) => View::Back {
                lines,
                newest: self.newest(),
                unseen: 0,
            },
        };
    }

    fn newest(&self) -> Option<MessageId> {
        let thread = self.thread.as_ref()?;
        thread.messages.last().map(|message| message.id)
    }

    /// `@grace: are you around tomorrow?`, the message `id` as a reply to it quotes it.
    fn replying(&self, id: MessageId) -> Option<Replying> {
        let thread = self.thread.as_ref()?;
        let message = thread.message(id)?;
        let who = handle(&self.party, &self.me, &thread.messages, &message.user_id);
        Some(Replying {
            id,
            quote: format!("@{who}: {}", message.preview()),
        })
    }

    /// What a reply in the history quotes: what dev-null.sh sent along, or the message itself
    /// when it is loaded.
    fn quote(&self, thread: &Thread, reply: &ReplyTo) -> String {
        let handle = |user_id| handle(&self.party, &self.me, &thread.messages, user_id);
        match (&reply.quote, thread.message(reply.id)) {
            (Some(quote), _) => format!("@{}: {}", handle(&quote.user_id), quote.text),
            (None, Some(message)) => {
                format!("@{}: {}", handle(&message.user_id), message.preview())
            }
            (None, None) => "an earlier message".to_owned(),
        }
    }

    /// Each emoji on message `id` in the order it was first left, counting what the user did that
    /// dev-null.sh hasn't answered for yet.
    fn tally(&self, id: MessageId) -> Vec<Tally<'_>> {
        let reactions = (self.thread.as_ref())
            .and_then(|thread| thread.message(id))
            .map_or(&[][..], |message| message.reactions.as_slice());
        let mut tally: Vec<Tally> = Vec::new();
        for reaction in reactions {
            let mine = mine(&self.party, &self.me, &reaction.user_id);
            match tally.iter_mut().find(|each| each.emoji == reaction.emoji) {
                Some(each) => {
                    each.count += 1;
                    each.mine |= mine;
                }
                None => tally.push(Tally {
                    emoji: &reaction.emoji,
                    count: 1,
                    mine,
                }),
            }
        }
        for want in self.wants.iter().filter(|want| want.message == id) {
            match tally.iter().position(|each| each.emoji == want.emoji) {
                Some(at) if tally[at].mine == want.on => {}
                Some(at) if want.on => {
                    tally[at].count += 1;
                    tally[at].mine = true;
                }
                Some(at) if tally[at].count == 1 => {
                    tally.remove(at);
                }
                Some(at) => {
                    tally[at].count -= 1;
                    tally[at].mine = false;
                }
                None if want.on => tally.push(Tally {
                    emoji: &want.emoji,
                    count: 1,
                    mine: true,
                }),
                None => {}
            }
        }
        tally
    }

    /// The emoji the reaction prompt offers: those named like what was typed, or before anything
    /// is, the ones already on the message and then the usual few.
    fn choices(&self) -> Vec<Emoji> {
        let Mode::Reacting { message, query, .. } = &self.mode else {
            return Vec::new();
        };
        if !query.is_empty() {
            return emoji::search(query);
        }
        let mut choices: Vec<Emoji> = Vec::new();
        let there: Vec<&str> = self.tally(*message).iter().map(|each| each.emoji).collect();
        for glyph in there.into_iter().chain(REACTIONS) {
            if let Some(emoji) = emoji::named(glyph)
                && !choices.contains(&emoji)
            {
                choices.push(emoji);
            }
        }
        choices
    }

    /// Leaves `emoji` on `message`, or takes it back if the user had left it, showing it at once.
    fn toggle(&mut self, message: MessageId, emoji: &str) -> Call {
        let on = !(self.tally(message).iter()).any(|each| each.emoji == emoji && each.mine);
        match (self.wants.iter_mut()).find(|want| want.message == message && want.emoji == emoji) {
            Some(want) => want.on = on,
            None => self.wants.push(Want {
                message,
                emoji: emoji.to_owned(),
                on,
            }),
        }
        Call::React {
            chat: self.chat(),
            message,
            emoji: emoji.to_owned(),
            on,
        }
    }

    /// Takes in a message's reactions as dev-null.sh answered a change to `emoji` on it.
    pub fn reacted(&mut self, message: MessageId, emoji: &str, reactions: Vec<Reaction>) {
        self.unreacted(message, emoji);
        if let Some(reacted) = (self.thread.as_mut()).and_then(|thread| thread.message_mut(message))
        {
            reacted.reactions = reactions;
        }
    }

    /// A change to `emoji` on `message` dev-null.sh refused shows as it was.
    pub fn unreacted(&mut self, message: MessageId, emoji: &str) {
        (self.wants).retain(|want| want.message != message || want.emoji != emoji);
    }

    /// Takes a page in, and says how far to mark the conversation read when the partner wrote
    /// something the user hasn't read. A page asked for by a screen closed since, or before
    /// an oldest message that is no longer the oldest, is dropped.
    pub fn land(&mut self, paging: Paging, page: MessagePage) -> Option<MessageId> {
        let oldest = self
            .thread
            .as_ref()
            .map(|thread| thread.messages.first().map(|each| each.id));
        match (paging, oldest) {
            (Paging::Since { .. }, None) | (Paging::Before(_), None) => return None,
            (Paging::Before(before), Some(oldest)) if oldest != Some(before) => return None,
            _ => {}
        }
        self.failure = None;
        let landed: Vec<&ClientId> = (page.messages.iter())
            .filter_map(|message| message.client_id.as_ref())
            .collect();
        self.outbox
            .retain(|outgoing| !landed.contains(&&outgoing.client_id));
        self.party = page.party;
        self.title = self.party.title();
        let (party, me) = (&self.party, self.me.as_str());
        let newest = page.messages.last().map(|message| message.id);
        let thread = match (&mut self.thread, paging) {
            (Some(thread), Paging::Before(_)) => {
                for message in page.messages {
                    thread.place(message);
                }
                thread.has_older = page.has_older;
                self.at_top = false;
                return None;
            }
            (Some(thread), Paging::Latest | Paging::Since { .. }) => {
                let mut arrived = 0;
                for message in page.messages {
                    let theirs = heard(party, me, &message);
                    if thread.place(message) && theirs {
                        arrived += 1;
                    }
                }
                for (id, reactions) in page.reaction_changes {
                    if let Some(message) = thread.message_mut(id) {
                        message.reactions = reactions;
                    }
                }
                if let View::Back { unseen, .. } = &mut self.view {
                    *unseen += arrived;
                }
                thread.access = page.access;
                thread.read_through = thread.read_through.max(page.read_through);
                thread.partner_read = page.partner_read_through;
                thread.reactions = thread.reactions.max(page.reactions_version);
                thread.polled = thread.polled.max(newest);
                thread
            }
            (thread, _) => thread.insert(Thread {
                messages: page.messages,
                has_older: page.has_older,
                read_through: page.read_through,
                partner_read: page.partner_read_through,
                access: page.access,
                polled: newest,
                reactions: page.reactions_version,
            }),
        };
        let unread = (thread.messages.iter())
            .any(|message| heard(party, me, message) && message.id > thread.read_through);
        let newest = thread.messages.last()?.id;
        unread.then(|| {
            thread.read_through = newest;
            newest
        })
    }

    pub fn fail(&mut self, paging: Paging, failure: Failure) {
        match paging {
            Paging::Latest if self.thread.is_none() => self.failure = Some(failure),
            Paging::Before(_) => self.older_failed = true,
            Paging::Latest | Paging::Since { .. } => {}
        }
    }

    pub fn typed(&mut self, event: KeyEvent) {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        match &mut self.mode {
            Mode::Typing => match event.code {
                KeyCode::Char(ch) if !ctrl && !alt => {
                    self.input.insert(ch.encode_utf8(&mut [0; 4]));
                }
                code => compose::edit(&mut self.input, code, ctrl, alt),
            },
            Mode::Reacting { message, query, at } => match event.code {
                KeyCode::Backspace if query.is_empty() => self.mode = Mode::Selecting(*message),
                KeyCode::Backspace => {
                    query.pop();
                    *at = 0;
                }
                KeyCode::Char(ch) if !ctrl && !alt => {
                    query.extend(named(ch));
                    *at = 0;
                }
                _ => {}
            },
            Mode::Selecting(_) => {}
        }
    }

    pub fn paste(&mut self, text: &str) {
        match &mut self.mode {
            Mode::Typing => self.input.insert(text),
            Mode::Reacting { query, at, .. } => {
                query.extend(text.chars().filter_map(named));
                *at = 0;
            }
            Mode::Selecting(_) => {}
        }
    }

    pub fn text(&self) -> &str {
        self.input.text()
    }

    /// Takes the text back from an editor, which ends a file with a newline a message never wants.
    pub fn replace(&mut self, edited: &str) {
        self.input = Buffer::new(edited.trim_end_matches(['\n', '\r']));
    }

    /// The call that sends what was typed, as a reply if one is pending, which waits dimmed at the
    /// bottom until it lands, or why it can't go. Nothing typed sends nothing.
    pub fn send(&mut self) -> Option<Result<Call, String>> {
        let body = self.input.text().trim().to_owned();
        if body.is_empty() {
            return None;
        }
        if let Some(refusal) = refusal(&body) {
            return Some(Err(refusal));
        }
        let client_id = match self.retry.take() {
            Some(retry) if retry.body == body => retry.client_id,
            _ => ClientId(Nonce::random().uuid()),
        };
        self.input = Buffer::default();
        self.view = View::Stuck;
        let reply = self.reply.take();
        let reply_to = reply.as_ref().map(|reply| reply.id);
        self.outbox.push(Outgoing {
            client_id: client_id.clone(),
            body: body.clone(),
            reply,
        });
        Some(Ok(Call::SendMessage {
            chat: self.chat(),
            body,
            client_id,
            reply_to,
        }))
    }

    /// The message a send created takes its dimmed place.
    pub fn sent(&mut self, client_id: &ClientId, message: Message) {
        self.outbox
            .retain(|outgoing| outgoing.client_id != *client_id);
        if let Some(thread) = &mut self.thread {
            thread.place(message);
        }
    }

    /// The ping dev-null.sh recorded shows at the bottom.
    pub fn pinged(&mut self, message: Message) {
        if let Some(thread) = &mut self.thread {
            thread.place(message);
            self.view = View::Stuck;
        }
    }

    /// A send that failed puts its text back in front of what was typed since, to send again,
    /// even when the send began on an earlier visit and so is not in this screen's outbox. The
    /// reply it was goes back too, unless another is pending.
    pub fn unsent(&mut self, client_id: ClientId, body: String) {
        let failed = (self.outbox.iter())
            .position(|outgoing| outgoing.client_id == client_id)
            .map(|at| self.outbox.remove(at));
        let mut unsent = Unsent {
            text: self.input.text().to_owned(),
            retry: None,
            reply: None,
        };
        unsent.restore(client_id, body);
        self.input = Buffer::new(&unsent.text);
        self.retry = unsent.retry;
        self.reply = self.reply.take().or(failed.and_then(|failed| failed.reply));
    }

    pub fn draw<Tz: TimeZone>(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        now: Instant,
        wall: &DateTime<Tz>,
    ) {
        if area.height == 0 {
            return;
        }
        let bottom = self.bottom(usize::from(area.width));
        let rows = (bottom.len() as u16).min(area.height);
        let history = Rect {
            height: area.height - rows,
            ..area
        };
        let prompt = Rect {
            y: area.y + area.height - rows,
            height: rows,
            ..area
        };
        let skip = bottom.len() - usize::from(rows);
        frame.render_widget(Paragraph::new(bottom[skip..].to_vec()), prompt);
        let Some(thread) = &self.thread else {
            let lines = match &self.failure {
                Some(failure) => fatal::screen(failure, None),
                None if self.placeholder_at().is_some_and(|at| now >= at) => vec![Line::styled(
                    format!("loading {}", self.title),
                    theme::muted(),
                )],
                None => Vec::new(),
            };
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), history);
            return;
        };
        let (lines, spans) = self.lines(thread, usize::from(area.width), wall);
        let total = lines.len();
        let page = usize::from(history.height);
        self.page = page;
        let newest = thread.messages.last().map(|message| message.id);
        let (mut back, unseen) = match &self.view {
            View::Stuck => (0, 0),
            View::Back {
                lines: up,
                newest: then,
                unseen,
            } => {
                let below = then
                    .and_then(|then| spans.iter().find(|(id, _)| *id == then))
                    .map_or(0, |(_, span)| total - span.end);
                (up + below, *unseen)
            }
        };
        if let Some(selected) = self.selected()
            && let Some((_, span)) = spans.iter().find(|(id, _)| *id == selected)
        {
            // The oldest message brings the top into view, which loads the ones before it.
            let oldest = thread
                .said()
                .next()
                .is_some_and(|first| first.id == selected);
            let start = if oldest { 0 } else { span.start };
            if span.end > total - back {
                back = total - span.end;
            }
            if start + page < total - back {
                back = total - (start + page);
            }
        }
        let back = back.min(total.saturating_sub(page));
        self.view = match back {
            0 => View::Stuck,
            up => View::Back {
                lines: up,
                newest,
                unseen,
            },
        };
        let end = total - back;
        let start = end.saturating_sub(page);
        self.at_top = start == 0;
        let mut shown: Vec<Line> = lines[start..end].to_vec();
        if unseen > 0
            && back > 0
            && let Some(last) = shown.last_mut()
        {
            let noun = if unseen == 1 { "message" } else { "messages" };
            let key = keys::key_for(Action::PageDown).unwrap_or_default();
            *last = Line::styled(
                format!("── {unseen} new {noun} below · {key} ──"),
                Style::new().fg(theme::PRIMARY),
            );
        }
        let gap = page.saturating_sub(shown.len()) as u16;
        let history = Rect {
            y: history.y + gap,
            height: history.height - gap,
            ..history
        };
        frame.render_widget(Paragraph::new(shown), history);
    }

    /// The message keys act on, while one is selected.
    fn selected(&self) -> Option<MessageId> {
        match self.mode {
            Mode::Typing => None,
            Mode::Selecting(id) | Mode::Reacting { message: id, .. } => Some(id),
        }
    }

    /// Every line of the conversation, and the lines each message takes.
    fn lines<Tz: TimeZone>(
        &self,
        thread: &Thread,
        width: usize,
        wall: &DateTime<Tz>,
    ) -> (Vec<Line<'static>>, Vec<(MessageId, Range<usize>)>) {
        let me = self.me.as_str();
        let title = &self.title;
        let mut out: Vec<Line<'static>> = Vec::new();
        let faint = Style::new().fg(theme::FAINT);
        out.push(match (thread.has_older, &self.party) {
            (true, _) => Line::styled("── older messages above ──", faint),
            (false, Party::Person(_)) if thread.messages.is_empty() => Line::styled(
                format!("No messages with {title} yet. Say hello."),
                theme::muted(),
            ),
            (false, Party::Org(_)) if thread.messages.is_empty() => Line::styled(
                format!("No messages in {title} yet. Say hello."),
                theme::muted(),
            ),
            (false, Party::Person(_)) => Line::styled(
                format!("── the start of your conversation with {title} ──"),
                faint,
            ),
            (false, Party::Org(_)) => Line::styled(format!("── the start of {title} ──"), faint),
        });
        let nick = (thread.messages.iter())
            .filter(|message| message.notice.is_none())
            .filter_map(|message| other(&self.party, me, message))
            .map(|author| author.handle.width())
            .fold(me.width(), usize::max);
        let indent = "00:00 ".len() + nick + "<> ".len();
        let room = width.saturating_sub(indent).max(1);
        let notice_indent = "00:00 -!- ".len();
        let notice_room = width.saturating_sub(notice_indent).max(1);
        let mine = Style::new()
            .fg(theme::FOREGROUND)
            .add_modifier(Modifier::BOLD);
        let text = Style::new().fg(theme::FOREGROUND);
        // A quote starts under the nick, left of where a message's own lines wrap, so it reads as
        // the start of the reply under it and not the end of the message above.
        let quote_indent = "00:00 ".len();
        let quoted = |quote: &str| {
            Line::from(vec![
                Span::raw(" ".repeat(quote_indent)),
                Span::styled(
                    fatal::cut(&format!("↳ {quote}"), width.saturating_sub(quote_indent)),
                    faint,
                ),
            ])
        };
        let direct = matches!(self.party, Party::Person(_));
        let selected = self.selected();
        let mut day = None;
        let mut spans = Vec::with_capacity(thread.messages.len());
        for message in &thread.messages {
            let local = message.created_at.with_timezone(&wall.timezone());
            if day != Some(local.date_naive()) {
                day = Some(local.date_naive());
                out.push(Line::styled(
                    format!("── {} ──", feed::day_label(local.date_naive(), wall)),
                    faint,
                ));
            }
            let start = out.len();
            let time = Span::styled(local.time().format("%H:%M ").to_string(), theme::muted());
            if let Some(told) = message.told() {
                let prefix = vec![time, Span::styled("-!- ", faint)];
                let muted = theme::muted();
                said(
                    &mut out,
                    prefix,
                    &told,
                    None,
                    None,
                    muted,
                    notice_room,
                    notice_indent,
                );
                spans.push((message.id, start..out.len()));
                continue;
            }
            let author = other(&self.party, me, message);
            let (who, style) = match author {
                Some(author) => (
                    author.handle.as_str(),
                    Style::new()
                        .fg(theme::lane(author.lane))
                        .add_modifier(Modifier::BOLD),
                ),
                None => (me, mine),
            };
            if let Some(reply) = &message.reply_to {
                out.push(quoted(&self.quote(thread, reply)));
            }
            let prefix = vec![
                time,
                Span::raw(" ".repeat(nick - who.width())),
                Span::styled(format!("<{who}> "), style),
            ];
            let mark = message.extra.map(|extra| extra.mark().to_owned());
            let receipt = (direct && author.is_none() && !message.is_event()).then(|| match thread
                .partner_read
            {
                Some(read) if message.id <= read => "✓✓",
                _ => "✓",
            });
            said(
                &mut out,
                prefix,
                &message.body,
                mark,
                receipt,
                text,
                room,
                indent,
            );
            let tally = self.tally(message.id);
            if !tally.is_empty() {
                let mut chips = vec![Span::raw(" ".repeat(indent)), Span::styled("[", faint)];
                for (at, each) in tally.iter().enumerate() {
                    if at > 0 {
                        chips.push(Span::raw("  "));
                    }
                    let style = match each.mine {
                        true => Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
                        false => theme::muted(),
                    };
                    chips.push(Span::styled(
                        format!("{} {}", each.emoji, each.count),
                        style,
                    ));
                }
                chips.push(Span::styled("]", faint));
                out.push(Line::from(feed::fit(chips, width)));
            }
            if selected == Some(message.id) {
                for line in &mut out[start..] {
                    *line = std::mem::take(line).style(Style::new().bg(theme::MUTED));
                }
            }
            spans.push((message.id, start..out.len()));
        }
        for outgoing in &self.outbox {
            if let Some(reply) = &outgoing.reply {
                out.push(quoted(&reply.quote));
            }
            let prefix = vec![
                Span::styled("--:-- ", faint),
                Span::raw(" ".repeat(nick - me.width())),
                Span::styled(format!("<{me}> "), faint),
            ];
            said(
                &mut out,
                prefix,
                &outgoing.body,
                None,
                None,
                faint,
                room,
                indent,
            );
        }
        (out, spans)
    }

    /// The rows under the conversation: the message a reply answers, the emoji a reaction can
    /// be, and the line being typed.
    fn bottom(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(reply) = &self.reply {
            lines.push(Line::styled(
                fatal::cut(&format!("↳ replying to {}", reply.quote), width),
                theme::muted(),
            ));
        }
        match &self.mode {
            Mode::Reacting { message, query, at } => {
                lines.push(self.offered(*message, query, *at, width));
                let block = Style::new().fg(theme::BACKGROUND).bg(theme::FOREGROUND);
                let mut prompt = vec![
                    Span::styled("+ ", Style::new().fg(theme::PRIMARY)),
                    Span::styled(":", theme::muted()),
                    Span::styled(query.clone(), Style::new().fg(theme::FOREGROUND)),
                    Span::styled(" ", block),
                ];
                if query.is_empty() {
                    prompt.push(Span::styled(
                        "type an emoji's name, or tab to one",
                        Style::new().fg(theme::FAINT),
                    ));
                }
                lines.push(Line::from(prompt));
            }
            mode => lines.push(self.input_line(width, *mode == Mode::Typing)),
        }
        lines
    }

    /// The emoji the prompt offers on one row, the chosen one filled in and those the user left
    /// already lit, scrolled so the chosen one shows.
    fn offered(&self, message: MessageId, query: &str, at: usize, width: usize) -> Line<'static> {
        let choices = self.choices();
        if choices.is_empty() {
            return Line::styled(format!("no emoji is named like :{query}"), theme::muted());
        }
        let tally = self.tally(message);
        let left = |glyph: &str| tally.iter().any(|each| each.mine && each.emoji == glyph);
        let cells: Vec<Span<'static>> = (choices.iter().enumerate())
            .map(|(each, emoji)| {
                let style = match (each == at, left(emoji.glyph)) {
                    (true, _) => popup::chosen(),
                    (false, true) => Style::new().fg(theme::PRIMARY).add_modifier(Modifier::BOLD),
                    (false, false) => Style::new().fg(theme::FOREGROUND),
                };
                Span::styled(format!(" {} {} ", emoji.glyph, emoji.name), style)
            })
            .collect();
        let at = at.min(cells.len() - 1);
        let mut first = at;
        let mut used = cells[at].width();
        while first > 0 && used + cells[first - 1].width() <= width {
            first -= 1;
            used += cells[first].width();
        }
        let mut used = 0;
        let shown: Vec<Span<'static>> = (cells.into_iter().skip(first))
            .take_while(|cell| {
                used += cell.width();
                used <= width
            })
            .collect();
        Line::from(shown)
    }

    /// `› typed text`, with the cursor as a block while the input has the keys, or why nothing can
    /// be sent.
    fn input_line(&self, width: usize, focused: bool) -> Line<'static> {
        let mark = Span::styled("› ", Style::new().fg(theme::PRIMARY));
        let refusal = match (&self.party, &self.thread) {
            (Party::Person(partner), Some(thread)) => thread.access.refusal(&partner.handle),
            _ => None,
        };
        if let Some(why) = refusal {
            return Line::from(vec![mark, Span::styled(why, theme::muted())]);
        }
        let room = width.saturating_sub(3).max(1);
        let text = self.input.text();
        let shown = |part: &str| part.replace('\n', "↵");
        if !focused {
            let text = match text {
                "" => format!("message {}", self.title),
                text => shown(text),
            };
            return Line::from(vec![
                Span::styled("› ", theme::muted()),
                Span::styled(fatal::cut(&text, room), Style::new().fg(theme::FAINT)),
            ]);
        }
        let cursor = self.input.cursor();
        let mut before = shown(&text[..cursor]);
        while before.width() >= room {
            let first = before.graphemes(true).next().map_or(1, str::len);
            before.drain(..first);
        }
        let after = shown(&text[cursor..]);
        let under = after.graphemes(true).next().unwrap_or(" ").to_owned();
        let rest = after.get(under.len()..).unwrap_or("");
        let left = room.saturating_sub(before.width() + 1);
        let block = Style::new().fg(theme::BACKGROUND).bg(theme::FOREGROUND);
        let mut spans = vec![
            mark,
            Span::styled(before, Style::new().fg(theme::FOREGROUND)),
            Span::styled(under, block),
        ];
        if text.is_empty() {
            spans.push(Span::styled(
                format!("message {}", self.title),
                Style::new().fg(theme::FAINT),
            ));
        } else {
            spans.push(Span::styled(
                fatal::cut(rest, left),
                Style::new().fg(theme::FOREGROUND),
            ));
        }
        Line::from(spans)
    }
}

/// What an emoji's name takes of a typed character: the name without the colons around it.
fn named(ch: char) -> Option<char> {
    (!ch.is_whitespace() && !ch.is_control() && ch != ':').then_some(ch)
}

/// Whether `user_id` is the user's. An org lists the user among its members by handle.
fn mine(party: &Party, me: &str, user_id: &UserId) -> bool {
    match party {
        Party::Person(partner) => *user_id != partner.id,
        Party::Org(org) => {
            (org.members.iter()).any(|member| member.handle == me && member.id == *user_id)
        }
    }
}

/// The handle of whoever `user_id` is in this conversation.
fn handle<'a>(party: &'a Party, me: &'a str, messages: &'a [Message], user_id: &UserId) -> &'a str {
    match party {
        Party::Person(partner) if partner.id == *user_id => &partner.handle,
        Party::Person(_) => me,
        Party::Org(org) => (org.members.iter())
            .chain(
                messages
                    .iter()
                    .filter_map(|message| message.author.as_ref()),
            )
            .find(|user| user.id == *user_id)
            .map_or("someone", |user| user.handle.as_str()),
    }
}

/// A message's lines: the prefix, then the text wrapped to `room` and indented under itself, then
/// the mark of what it carries, and after the last of them whether the partner has it.
#[allow(clippy::too_many_arguments)]
fn said(
    out: &mut Vec<Line<'static>>,
    prefix: Vec<Span<'static>>,
    body: &str,
    mark: Option<String>,
    receipt: Option<&'static str>,
    style: Style,
    room: usize,
    indent: usize,
) {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    if !body.is_empty() {
        for paragraph in body.split('\n') {
            feed::wrap(&[(paragraph.to_owned(), style)], room, &mut rows);
        }
    }
    if let Some(mark) = mark {
        rows.push(vec![Span::styled(mark, theme::muted())]);
    }
    if let Some(receipt) = receipt {
        let receipt = Span::styled(receipt, theme::muted());
        match rows.last_mut() {
            Some(last)
                if last.iter().map(Span::width).sum::<usize>() + 1 + receipt.width() <= room =>
            {
                last.extend([Span::raw(" "), receipt]);
            }
            _ => rows.push(vec![receipt]),
        }
    }
    let mut prefix = Some(prefix);
    for row in rows {
        let mut line = prefix
            .take()
            .unwrap_or_else(|| vec![Span::raw(" ".repeat(indent))]);
        line.extend(row);
        out.push(Line::from(line));
    }
}
