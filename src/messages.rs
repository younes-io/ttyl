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
use crate::engine::AHEAD;
use crate::fatal;
use crate::feed;
use crate::keys::{self, Act, Action};
use crate::model::{
    Access, Chat, ClientId, Conversation, ConversationPage, Message, MessageId, MessagePage, Party,
    User,
};
use crate::text::Buffer;
use crate::theme;
use crate::thread::LOADING_NOTE_DELAY;

pub const TITLE: &str = "messages";
/// dev-null.sh keeps a message to this many characters.
pub const MAX_CHARS: usize = 2000;
pub const NO_ANSWER: &str = "fatal: no answer from dev-null.sh. it may have gone through; the conversation shows it if it did.";

/// Pinned first, then unread, then the rest, each group in dev-null.sh's order of newest first.
pub fn order(conversations: &mut [Conversation]) {
    conversations.sort_by_key(|conversation| (!conversation.pinned, conversation.unread == 0));
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

/// The conversations with people and orgs: pinned first, then unread, then the rest, each in
/// dev-null.sh's order of newest first. One line each, like a mail index.
pub struct Conversations {
    loaded: Option<Loaded>,
    failure: Option<Failure>,
    asked_at: Instant,
    selected: usize,
    offset: usize,
}

struct Loaded {
    rows: Vec<Conversation>,
    next: Option<String>,
}

impl Conversations {
    pub fn new(now: Instant) -> Self {
        Conversations {
            loaded: None,
            failure: None,
            asked_at: now,
            selected: 0,
            offset: 0,
        }
    }

    /// The first page.
    pub fn call(&self) -> Call {
        Call::Conversations { before: None }
    }

    fn rows(&self) -> &[Conversation] {
        self.loaded
            .as_ref()
            .map_or(&[], |loaded| loaded.rows.as_slice())
    }

    pub fn selected(&self) -> Option<&Conversation> {
        self.rows().get(self.selected)
    }

    /// A failed reload over rows still shown, for the hint line.
    pub fn failure(&self) -> Option<&Failure> {
        self.loaded.as_ref()?;
        self.failure.as_ref()
    }

    pub fn can(&self, action: Action) -> bool {
        match action {
            Action::Open => self.selected().is_some(),
            Action::Back | Action::Refresh => true,
            Action::Down | Action::Up | Action::Top | Action::Bottom => !self.rows().is_empty(),
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action) {
        let last = self.rows().len().saturating_sub(1);
        self.selected = match action {
            Action::Down => (self.selected + 1).min(last),
            Action::Up => self.selected.saturating_sub(1),
            Action::Top => 0,
            Action::Bottom => last,
            _ => self.selected,
        };
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
        (left <= AHEAD && self.failure.is_none()).then_some(Call::Conversations {
            before: Some(before),
        })
    }

    /// A first page replaces the list and keeps the selection on its conversation; a later one adds
    /// the conversations it brings.
    pub fn land(&mut self, before: Option<&str>, page: ConversationPage) {
        let kept = self
            .selected()
            .map(|conversation| conversation.party.chat());
        self.failure = None;
        match (before, &mut self.loaded) {
            (None, _) => {
                self.loaded = Some(Loaded {
                    rows: page.conversations,
                    next: page.next,
                });
            }
            (Some(before), Some(loaded)) if loaded.next.as_deref() == Some(before) => {
                for conversation in page.conversations {
                    let chat = conversation.party.chat();
                    if !loaded.rows.iter().any(|row| row.party.is(&chat)) {
                        loaded.rows.push(conversation);
                    }
                }
                loaded.next = page.next.filter(|next| next != before);
            }
            (Some(_), _) => return,
        }
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        order(&mut loaded.rows);
        self.selected = kept
            .and_then(|chat| (loaded.rows.iter()).position(|row| row.party.is(&chat)))
            .unwrap_or(0);
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
        if loaded.rows.is_empty() {
            let mut lines = vec![vec![Span::styled(
                "$ ls /dev/remotes",
                Style::new().fg(theme::FOREGROUND),
            )]];
            let key = keys::key_for(Action::Commit(Act::Message)).unwrap_or_default();
            let empty =
                format!("No conversations yet. Press {key} on a commit or a person to start one.");
            feed::wrap(&[(empty, theme::muted())], width, &mut lines);
            frame.render_widget(
                Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()),
                area,
            );
            return;
        }
        let height = usize::from(area.height).max(1);
        self.selected = self.selected.min(loaded.rows.len() - 1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
        let lines: Vec<Line> = (loaded.rows.iter().enumerate())
            .skip(self.offset)
            .take(height)
            .map(|(at, conversation)| row(conversation, at == self.selected, width, me, wall))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// `● Grace Hopper @grace (2)  did you see the patch?          3m`, or for an org
/// `● #unix-room 5 members (4)  grace: pushed v2          3m`, filling `width`.
fn row<Tz: TimeZone>(
    conversation: &Conversation,
    selected: bool,
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
    let mut age = conversation
        .last_at
        .map(|at| feed::relative_time(at, wall))
        .unwrap_or_default();
    if conversation.pinned {
        age = format!("pinned {age}").trim_end().to_owned();
    }
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
    input: Buffer,
    /// The failed send whose id goes again while its text is unchanged.
    retry: Option<Outgoing>,
    /// Sent and not yet answered, shown dimmed under the conversation.
    outbox: Vec<Outgoing>,
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
    access: Access,
    /// The newest message a poll brought. A message this screen sent can be newer than one the
    /// partner sent before it, so polling after the newest message shown would skip theirs.
    polled: Option<MessageId>,
}

impl Thread {
    /// Puts `message` in its place by id, unless it is already there. Says whether it was new.
    fn place(&mut self, message: Message) -> bool {
        let at = (self.messages).partition_point(|each| each.id < message.id);
        let known = self
            .messages
            .get(at)
            .is_some_and(|each| each.id == message.id);
        if !known {
            self.messages.insert(at, message);
        }
        !known
    }
}

#[derive(Debug, Clone)]
struct Outgoing {
    client_id: ClientId,
    body: String,
}

/// What was typed in a conversation and not sent, kept while it is closed.
#[derive(Debug, Default)]
pub struct Unsent {
    text: String,
    retry: Option<Outgoing>,
}

impl Unsent {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Puts a failed send's text back in front of what was typed since. Sent again unchanged, it
    /// goes with the same id, so dev-null.sh keeps one copy if the first one did land.
    pub fn restore(&mut self, client_id: ClientId, body: String) {
        self.text = match self.text.as_str() {
            "" => body.clone(),
            typed => format!("{body}\n{typed}"),
        };
        self.retry = Some(Outgoing { client_id, body });
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
            input: Buffer::new(&unsent.text),
            retry: unsent.retry,
            outbox: Vec::new(),
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

    /// The latest messages, or once they are in, the ones since.
    pub fn poll(&self) -> Call {
        let polled = self.thread.as_ref().and_then(|thread| thread.polled);
        Call::Messages {
            chat: self.chat(),
            paging: polled.map_or(Paging::Latest, Paging::After),
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
        match action {
            Action::Send | Action::Editor => self.writable(),
            Action::PageUp => self.thread.is_some(),
            Action::PageDown => matches!(self.view, View::Back { .. }),
            Action::Back => true,
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action) {
        let back = match self.view {
            View::Stuck => 0,
            View::Back { lines, .. } => lines,
        };
        let back = match action {
            Action::PageUp => back + self.page.saturating_sub(1).max(1),
            Action::PageDown => back.saturating_sub(self.page.saturating_sub(1).max(1)),
            _ => return,
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

    /// Takes a page in, and says how far to mark the conversation read when the partner wrote
    /// something the user hasn't read. A page asked for by a screen closed since, or before
    /// an oldest message that is no longer the oldest, is dropped.
    pub fn land(&mut self, paging: Paging, page: MessagePage) -> Option<MessageId> {
        let oldest = self
            .thread
            .as_ref()
            .map(|thread| thread.messages.first().map(|each| each.id));
        match (paging, oldest) {
            (Paging::After(_), None) | (Paging::Before(_), None) => return None,
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
            (Some(thread), Paging::Latest | Paging::After(_)) => {
                let mut arrived = 0;
                for message in page.messages {
                    let theirs = heard(party, me, &message);
                    if thread.place(message) && theirs {
                        arrived += 1;
                    }
                }
                if let View::Back { unseen, .. } = &mut self.view {
                    *unseen += arrived;
                }
                thread.access = page.access;
                thread.read_through = thread.read_through.max(page.read_through);
                thread.polled = thread.polled.max(newest);
                thread
            }
            (thread, _) => thread.insert(Thread {
                messages: page.messages,
                has_older: page.has_older,
                read_through: page.read_through,
                access: page.access,
                polled: newest,
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
            Paging::Latest | Paging::After(_) => {}
        }
    }

    pub fn typed(&mut self, event: KeyEvent) {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        match event.code {
            KeyCode::Char(ch) if !ctrl && !alt => self.input.insert(ch.encode_utf8(&mut [0; 4])),
            code => compose::edit(&mut self.input, code, ctrl, alt),
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.input.insert(text);
    }

    pub fn text(&self) -> &str {
        self.input.text()
    }

    /// Takes the text back from an editor, which ends a file with a newline a message never wants.
    pub fn replace(&mut self, edited: &str) {
        self.input = Buffer::new(edited.trim_end_matches(['\n', '\r']));
    }

    /// The call that sends what was typed, which waits dimmed at the bottom until it lands, or why
    /// it can't go. Nothing typed sends nothing.
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
        self.outbox.push(Outgoing {
            client_id: client_id.clone(),
            body: body.clone(),
        });
        Some(Ok(Call::SendMessage {
            chat: self.chat(),
            body,
            client_id,
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

    /// A send that failed puts its text back in front of what was typed since, to send again,
    /// even when the send began on an earlier visit and so is not in this screen's outbox.
    pub fn unsent(&mut self, client_id: ClientId, body: String) {
        self.outbox
            .retain(|outgoing| outgoing.client_id != client_id);
        let mut unsent = Unsent {
            text: self.input.text().to_owned(),
            retry: None,
        };
        unsent.restore(client_id, body);
        self.input = Buffer::new(&unsent.text);
        self.retry = unsent.retry;
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
        let history = Rect {
            height: area.height - 1,
            ..area
        };
        let prompt = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        frame.render_widget(self.input_line(usize::from(area.width)), prompt);
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
        let (lines, ends) = self.lines(thread, usize::from(area.width), wall);
        let total = lines.len();
        let page = usize::from(history.height);
        self.page = page;
        let newest = thread.messages.last().map(|message| message.id);
        let (back, unseen) = match &self.view {
            View::Stuck => (0, 0),
            View::Back {
                lines: up,
                newest: then,
                unseen,
            } => {
                let below = then
                    .and_then(|then| ends.iter().find(|(id, _)| *id == then))
                    .map_or(0, |(_, end)| total - end);
                (up + below, *unseen)
            }
        };
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

    /// Every line of the conversation, and the line each message ends before.
    fn lines<Tz: TimeZone>(
        &self,
        thread: &Thread,
        width: usize,
        wall: &DateTime<Tz>,
    ) -> (Vec<Line<'static>>, Vec<(MessageId, usize)>) {
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
        let mut day = None;
        let mut ends = Vec::with_capacity(thread.messages.len());
        for message in &thread.messages {
            let local = message.created_at.with_timezone(&wall.timezone());
            if day != Some(local.date_naive()) {
                day = Some(local.date_naive());
                out.push(Line::styled(
                    format!("── {} ──", feed::day_label(local.date_naive(), wall)),
                    faint,
                ));
            }
            let time = Span::styled(local.time().format("%H:%M ").to_string(), theme::muted());
            if let Some(told) = message.told() {
                let prefix = vec![time, Span::styled("-!- ", faint)];
                let muted = theme::muted();
                said(
                    &mut out,
                    prefix,
                    &told,
                    None,
                    muted,
                    notice_room,
                    notice_indent,
                );
                ends.push((message.id, out.len()));
                continue;
            }
            let (who, style) = match other(&self.party, me, message) {
                Some(author) => (
                    author.handle.as_str(),
                    Style::new()
                        .fg(theme::lane(author.lane))
                        .add_modifier(Modifier::BOLD),
                ),
                None => (me, mine),
            };
            let prefix = vec![
                time,
                Span::raw(" ".repeat(nick - who.width())),
                Span::styled(format!("<{who}> "), style),
            ];
            let mark = message.extra.map(|extra| extra.mark().to_owned());
            said(&mut out, prefix, &message.body, mark, text, room, indent);
            ends.push((message.id, out.len()));
        }
        for outgoing in &self.outbox {
            let prefix = vec![
                Span::styled("--:-- ", faint),
                Span::raw(" ".repeat(nick - me.width())),
                Span::styled(format!("<{me}> "), faint),
            ];
            said(&mut out, prefix, &outgoing.body, None, faint, room, indent);
        }
        (out, ends)
    }

    /// `› typed text`, with the cursor as a block, or why nothing can be sent.
    fn input_line(&self, width: usize) -> Line<'static> {
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
        let cursor = self.input.cursor();
        let shown = |part: &str| part.replace('\n', "↵");
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

/// A message's lines: the prefix, then the text wrapped to `room` and indented under itself, then
/// the mark of what it carries.
fn said(
    out: &mut Vec<Line<'static>>,
    prefix: Vec<Span<'static>>,
    body: &str,
    mark: Option<String>,
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
    let mut prefix = Some(prefix);
    for row in rows {
        let mut line = prefix
            .take()
            .unwrap_or_else(|| vec![Span::raw(" ".repeat(indent))]);
        line.extend(row);
        out.push(Line::from(line));
    }
}
