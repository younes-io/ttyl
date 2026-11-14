use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::time::Duration;
use std::{fs, io};

use chrono::{DateTime, FixedOffset, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ssh_key::sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::http::Request;

use crate::at;
use crate::attach::{self, Attachment, Image};
use crate::cache::Cache;
use crate::config::{self, Change};
use crate::device::{DeviceKey, Nonce, Signed};
use crate::draft::Target;
use crate::model::{
    Candidate, Chat, Circle, ClientId, CommitId, CommitPage, ConversationPage, Device, DeviceId,
    Filter, Focus, Folder, GroupPage, Headline, HeadlineId, HeadlinePage, Lane, Log, LogPage,
    Message, MessageId, MessagePage, NotificationId, NotificationPage, OrganizationList,
    OrganizationPage, PeoplePage, Profile, Reason, Refile, RepliesPage, RowPage, Scheduled,
    ScheduledId, Segment, Settings, Translation, Who,
};

pub const BASE: &str = "https://dev-null.sh";
/// Up to 30 MB of images can take a slow uplink well past the 30 seconds the JSON requests get.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Talks to the API as one device. Every request carries that device's signature.
pub struct Client {
    base: String,
    key: DeviceKey,
    agent: Agent,
    cache: Option<Cache>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub handle: String,
    pub name: Option<String>,
    #[serde(default)]
    pub pasteis_mode: bool,
    #[serde(default)]
    pub pasteis_remaining: Option<u32>,
    /// The lane or color the user picked, or `None` for the automatic one.
    #[serde(default)]
    pub preferred_lane: Option<Lane>,
    #[serde(default)]
    pub automatic_lane: u8,
    /// Who can push to the user, while Remotes are on.
    #[serde(default)]
    pub remotes: Option<config::Remotes>,
    /// The organizations the user is a member of, oldest membership first, which they may post as.
    #[serde(default)]
    pub organizations: Vec<Membership>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Membership {
    pub handle: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    #[serde(rename = "data")]
    pub me: Me,
    #[serde(default, rename = "unread_notifications_count")]
    pub unread: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Me,
    Redeem {
        code: String,
        verifier: String,
    },
    Page(Log),
    Older {
        log: Log,
        cursor: String,
    },
    Since {
        log: Log,
        since: CommitId,
    },
    Commit(CommitId),
    Replies {
        id: CommitId,
        cursor: Option<String>,
    },
    Star {
        id: CommitId,
        on: bool,
    },
    Follow {
        who: Who,
        on: bool,
    },
    /// Mutes someone, silently as the web does, or unmutes them.
    Mute {
        handle: String,
        on: bool,
    },
    /// What the user set on dev-null.sh, for the language commits translate into.
    Settings,
    /// The commit in `language`, translated on dev-null.sh.
    Translate {
        id: CommitId,
        language: String,
    },
    /// Blocks someone, which ends the follows both ways, or unblocks them, which leaves the follows ended.
    Block {
        handle: String,
        on: bool,
    },
    /// The people the user mutes, last muted first.
    Muted {
        cursor: Option<String>,
    },
    /// The people the user blocks, last blocked first.
    Blocked {
        cursor: Option<String>,
    },
    /// The devices that may sign requests as the user, newest approval first.
    Devices,
    /// Revokes a device. Revoking the one signing signs ttyl out.
    Revoke(DeviceId),
    /// Changes one of the user's settings, answered as `me` answers.
    Configure(Change),
    User(String),
    /// An organization's page.
    Organization(String),
    /// An organization's commits, or with `mentions` the commits that mention it.
    OrganizationCommits {
        handle: String,
        mentions: bool,
        cursor: Option<String>,
    },
    /// The organizations someone follows, which their following list shows above the people.
    FollowedOrganizations {
        handle: String,
        cursor: Option<String>,
    },
    /// Someone's commits, or with `replies` their replies, as their profile lists them.
    Commits {
        handle: String,
        replies: bool,
        cursor: Option<String>,
    },
    People {
        handle: String,
        circle: Circle,
        cursor: Option<String>,
    },
    /// The signed-in user's starred commits, newest star first.
    Starred {
        cursor: Option<String>,
    },
    /// Commits nearest in meaning to the words, as the web's search finds them.
    SearchCommits(String),
    /// People whose name or handle matches.
    SearchPeople(String),
    /// The few commits nearest in meaning to this one, as the web lists them under a thread.
    Related(CommitId),
    /// The topics dev-null.sh is talking about right now, hottest first.
    Headlines,
    /// One headline's commits, with its recap.
    Headline(HeadlineId),
    Delete(CommitId),
    /// Undoes deleting the user's commit, within the server's 30-second window.
    Restore(CommitId),
    Pastel(CommitId),
    /// Pins the user's commit to the top of their profile in place of the one pinned before, or unpins it.
    Pin {
        id: CommitId,
        on: bool,
    },
    Vote {
        id: CommitId,
        option: String,
    },
    Report {
        id: CommitId,
        reason: Reason,
        details: Option<String>,
    },
    Post {
        target: Target,
        body: String,
        attachment: Attachment,
        /// The organization it goes out as, by handle, or `None` for the user.
        author: Option<String>,
    },
    /// Queues a new commit to go out at `at`. Only the user sees it until then.
    Schedule {
        body: String,
        attachment: Attachment,
        at: DateTime<FixedOffset>,
        author: Option<String>,
    },
    /// The user's scheduled commits still waiting, soonest first.
    Queue,
    /// Cancels a scheduled commit before it goes out.
    Unschedule(ScheduledId),
    /// Commits a scheduled one now, ahead of its time.
    SendNow(ScheduledId),
    /// A draft rendered as the commit it would become. Nothing is kept.
    Preview(String),
    Notifications {
        filter: Filter,
        cursor: Option<String>,
    },
    Group {
        id: NotificationId,
        filter: Filter,
        cursor: Option<String>,
    },
    /// A folder of the user's conversations, pinned first then newest, those matching `query`
    /// when it isn't empty, continuing before a conversation.
    Conversations {
        folder: Folder,
        query: String,
        before: Option<String>,
    },
    /// Pins, stashes or mutes a conversation for the user alone, or undoes it.
    File {
        chat: Chat,
        change: Refile,
    },
    /// Merges the request from `handle` into the open conversations, or drops it without telling
    /// them.
    Answer {
        handle: String,
        merge: bool,
    },
    /// The people matching a search, with why each one can't be messaged, if they can't.
    Candidates(String),
    /// A page of a conversation.
    Messages {
        chat: Chat,
        paging: Paging,
    },
    /// Marks a conversation read up to `through`.
    MarkRead {
        chat: Chat,
        through: MessageId,
    },
    /// Sends a message, starting the conversation with a person if it is the first.
    SendMessage {
        chat: Chat,
        body: String,
        client_id: ClientId,
    },
}

/// Which messages of a conversation a page holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paging {
    Latest,
    /// The page before a message, scrolling back.
    Before(MessageId),
    /// Everything after a message, polling for new ones.
    After(MessageId),
}

#[derive(Debug)]
pub enum Reply {
    Account(Account),
    Page(LogPage),
    Thread(Box<CommitPage>),
    Replies(RepliesPage),
    Commit(Box<Focus>),
    Pastel {
        commit: Box<Focus>,
        remaining: u32,
    },
    User(Box<Profile>),
    Organization(Box<OrganizationPage>),
    Organizations(OrganizationList),
    Rows(RowPage),
    People(PeoplePage),
    Headlines(Vec<Headline>),
    Headline(Box<HeadlinePage>),
    Notifications(NotificationPage),
    Group(GroupPage),
    Preview(Vec<Segment>),
    Settings(Settings),
    Translation(Box<Translation>),
    Devices(Vec<Device>),
    Scheduled(Box<Scheduled>),
    Queue(Vec<Scheduled>),
    /// A scheduled commit went out early, and whether it waits for review.
    Submitted {
        pending: bool,
    },
    Conversations(ConversationPage),
    Candidates(Vec<Candidate>),
    Messages(Box<MessagePage>),
    Message(Box<Message>),
    Done,
}

#[derive(Debug)]
pub enum Error {
    /// The server refused the request and said why.
    Api {
        status: u16,
        code: Option<String>,
        message: String,
        retry_after: Option<Duration>,
    },
    Http(ureq::Error),
    /// The server answered with something other than the JSON it documents, such as a proxy's error page.
    Unexpected(u16),
    WrongShape(String),
    /// An image could not be read from disk, so nothing was sent.
    Image(String),
}

impl Error {
    /// The server does not know this device, or the user has not approved it yet.
    pub fn needs_sign_in(&self) -> bool {
        self.is("device_not_approved")
    }

    pub fn is(&self, code: &str) -> bool {
        matches!(self, Error::Api { code: Some(given), .. } if given == code)
    }

    pub fn may_have_landed(&self) -> bool {
        match self {
            Error::Http(
                ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect),
            ) => false,
            Error::Http(ureq::Error::Io(error)) => !matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::HostUnreachable
                    | io::ErrorKind::NetworkUnreachable
                    | io::ErrorKind::AddrNotAvailable
            ),
            Error::Http(_) | Error::WrongShape(_) | Error::Unexpected(502 | 504) => true,
            Error::Api { .. } | Error::Unexpected(_) | Error::Image(_) => false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Error::Api { status: 404, .. } | Error::Unexpected(404)
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Api { message, .. } => f.write_str(message),
            Error::Http(error) => write!(f, "can't reach /dev/null: {error}"),
            Error::Unexpected(status) => write!(
                f,
                "/dev/null answered {status} with a page instead of data. The site may be down for maintenance."
            ),
            Error::WrongShape(detail) => write!(f, "can't read what /dev/null sent: {detail}"),
            Error::Image(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Down,
    Offline(String),
    Refused(String),
}

impl Failure {
    pub fn of(error: &Error) -> Self {
        match error {
            Error::Unexpected(_) => Failure::Down,
            Error::Http(error) => Failure::Offline(match error {
                ureq::Error::HostNotFound => "Could not resolve host: dev-null.sh".to_owned(),
                ureq::Error::ConnectionFailed => "Failed to connect to dev-null.sh".to_owned(),
                ureq::Error::Timeout(_) => "Operation timed out".to_owned(),
                error => error.to_string(),
            }),
            Error::Api { message, .. } => Failure::Refused(message.clone()),
            Error::WrongShape(_) | Error::Image(_) => Failure::Refused(error.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct Refusal {
    message: String,
    code: Option<String>,
    /// Laravel's 422 lists each field's messages and folds them into `message` with a count.
    #[serde(default)]
    errors: HashMap<String, Vec<String>>,
}

impl Client {
    pub fn new(base: impl Into<String>, key: DeviceKey) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("ttyl/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Client {
            base: base.into(),
            key,
            agent,
            cache: None,
        }
    }

    /// Keeps the account and first log pages on disk for the next launch, and forgets them once the server
    /// stops knowing this device.
    pub fn with_cache(self, cache: Cache) -> Self {
        Client {
            cache: Some(cache),
            ..self
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn key(&self) -> &DeviceKey {
        &self.key
    }

    pub fn call(&self, call: &Call) -> Result<Reply, Error> {
        match call {
            Call::Me => self.me().map(Reply::Account),
            Call::Redeem { code, verifier } => self.redeem(code, verifier).map(Reply::Account),
            Call::Page(log) => self.page(*log, None).map(Reply::Page),
            Call::Older { log, cursor } => {
                self.page(*log, Some(("cursor", cursor))).map(Reply::Page)
            }
            Call::Since { log, since } => {
                self.page(*log, Some(("since", &since.0))).map(Reply::Page)
            }
            Call::Commit(id) => self.commit(id).map(|page| Reply::Thread(Box::new(page))),
            Call::Replies { id, cursor } => self.replies(id, cursor.as_deref()).map(Reply::Replies),
            Call::Star { id, on } => self
                .star(id, *on)
                .map(|focus| Reply::Commit(Box::new(focus))),
            Call::Follow { who, on } => {
                let path = match who {
                    Who::Person(handle) => format!("/api/v1/users/{}/follow", escape(handle)),
                    Who::Organization(handle) => {
                        format!("/api/v1/organizations/{}/follow", escape(handle))
                    }
                };
                self.send(if *on { "POST" } else { "DELETE" }, &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Mute { handle, on } => {
                let path = format!("/api/v1/users/{}/mute", escape(handle));
                self.send(if *on { "POST" } else { "DELETE" }, &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Settings => self.settings().map(Reply::Settings),
            Call::Translate { id, language } => {
                let path = format!("/api/v1/commits/{}/translation", escape(&id.0));
                let body = serde_json::json!({ "language": language }).to_string();
                let (status, text) = self.send("POST", &path, body)?;
                (Translation::parse(&text))
                    .map(|translation| Reply::Translation(Box::new(translation)))
                    .map_err(|error| unreadable(status, error))
            }
            Call::Block { handle, on } => {
                let path = format!("/api/v1/users/{}/block", escape(handle));
                self.send(if *on { "POST" } else { "DELETE" }, &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Muted { cursor } => self.muted(cursor.as_deref()).map(Reply::People),
            Call::Blocked { cursor } => self.blocked(cursor.as_deref()).map(Reply::People),
            Call::Devices => self.devices().map(Reply::Devices),
            Call::Revoke(id) => self.revoke(id).map(|()| Reply::Done),
            Call::Configure(change) => self.configure(change).map(Reply::Account),
            Call::User(handle) => self
                .user(handle)
                .map(|profile| Reply::User(Box::new(profile))),
            Call::Organization(handle) => {
                let path = format!("/api/v1/organizations/{}", escape(handle));
                let (status, text) = self.send("GET", &path, String::new())?;
                (OrganizationPage::parse(&text))
                    .map(|page| Reply::Organization(Box::new(page)))
                    .map_err(|error| unreadable(status, error))
            }
            Call::OrganizationCommits {
                handle,
                mentions,
                cursor,
            } => {
                let list = if *mentions { "mentions" } else { "commits" };
                let mut path = format!("/api/v1/organizations/{}/{list}", escape(handle));
                if let Some(cursor) = cursor {
                    path.push_str(&format!("?cursor={}", escape(cursor)));
                }
                let (status, text) = self.send("GET", &path, String::new())?;
                (RowPage::parse(&text))
                    .map(Reply::Rows)
                    .map_err(|error| unreadable(status, error))
            }
            Call::FollowedOrganizations { handle, cursor } => {
                let mut path = format!("/api/v1/users/{}/following/organizations", escape(handle));
                if let Some(cursor) = cursor {
                    path.push_str(&format!("?cursor={}", escape(cursor)));
                }
                let (status, text) = self.send("GET", &path, String::new())?;
                (OrganizationList::parse(&text))
                    .map(Reply::Organizations)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Commits {
                handle,
                replies,
                cursor,
            } => self
                .commits(handle, *replies, cursor.as_deref())
                .map(Reply::Rows),
            Call::People {
                handle,
                circle,
                cursor,
            } => self
                .people(handle, *circle, cursor.as_deref())
                .map(Reply::People),
            Call::Starred { cursor } => self.starred(cursor.as_deref()).map(Reply::Rows),
            Call::SearchCommits(query) => {
                let path = format!("/api/v1/search/commits?q={}", escape(query));
                let (status, text) = self.send("GET", &path, String::new())?;
                (RowPage::parse_search(&text))
                    .map(Reply::Rows)
                    .map_err(|error| unreadable(status, error))
            }
            Call::SearchPeople(name) => {
                let path = format!("/api/v1/search/profiles?q={}", escape(name));
                let (status, text) = self.send("GET", &path, String::new())?;
                (PeoplePage::parse_search(&text))
                    .map(Reply::People)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Related(id) => {
                let path = format!("/api/v1/commits/{}/related", escape(&id.0));
                let (status, text) = self.send("GET", &path, String::new())?;
                (RowPage::parse_search(&text))
                    .map(Reply::Rows)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Headlines => {
                let (status, text) = self.send("GET", "/api/v1/headlines", String::new())?;
                (Headline::parse_list(&text))
                    .map(Reply::Headlines)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Headline(id) => {
                let path = format!("/api/v1/headlines/{}", escape(&id.0));
                let (status, text) = self.send("GET", &path, String::new())?;
                (HeadlinePage::parse(&text))
                    .map(|page| Reply::Headline(Box::new(page)))
                    .map_err(|error| unreadable(status, error))
            }
            Call::Delete(id) => {
                let path = format!("/api/v1/commits/{}", escape(&id.0));
                self.send("DELETE", &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Restore(id) => {
                let path = format!("/api/v1/commits/{}/restoration", escape(&id.0));
                self.focus("POST", &path, String::new())
                    .map(|focus| Reply::Commit(Box::new(focus)))
            }
            Call::Pastel(id) => self.pastel(id),
            Call::Pin { id, on } => {
                let path = format!("/api/v1/commits/{}/pin", escape(&id.0));
                self.focus(if *on { "PUT" } else { "DELETE" }, &path, String::new())
                    .map(|focus| Reply::Commit(Box::new(focus)))
            }
            Call::Vote { id, option } => self
                .vote(id, option)
                .map(|focus| Reply::Commit(Box::new(focus))),
            Call::Report {
                id,
                reason,
                details,
            } => {
                let path = format!("/api/v1/commits/{}/reports", escape(&id.0));
                let body =
                    serde_json::json!({ "reason": reason.slug(), "details": details }).to_string();
                self.send("POST", &path, body).map(|_| Reply::Done)
            }
            Call::Post {
                target,
                body,
                attachment,
                author,
            } => self
                .post(target, body, attachment, author.as_deref())
                .map(|focus| Reply::Commit(Box::new(focus))),
            Call::Schedule {
                body,
                attachment,
                at,
                author,
            } => self
                .schedule(body, attachment, at, author.as_deref())
                .map(|scheduled| Reply::Scheduled(Box::new(scheduled))),
            Call::Queue => {
                let (status, text) =
                    self.send("GET", "/api/v1/scheduled-commits", String::new())?;
                (Scheduled::parse_queue(&text))
                    .map(Reply::Queue)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Unschedule(id) => {
                let path = format!("/api/v1/scheduled-commits/{}", escape(&id.0));
                self.send("DELETE", &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::SendNow(id) => {
                let path = format!("/api/v1/scheduled-commits/{}/submission", escape(&id.0));
                let (status, text) = self.send("POST", &path, String::new())?;
                (Scheduled::parse_pending(&text))
                    .map(|pending| Reply::Submitted { pending })
                    .map_err(|error| unreadable(status, error))
            }
            Call::Preview(body) => self.preview(body).map(Reply::Preview),
            Call::Notifications { filter, cursor } => self
                .notifications(*filter, cursor.as_deref())
                .map(Reply::Notifications),
            Call::Group { id, filter, cursor } => {
                self.group(id, *filter, cursor.as_deref()).map(Reply::Group)
            }
            Call::Conversations {
                folder,
                query,
                before,
            } => self
                .conversations(*folder, query, before.as_deref())
                .map(Reply::Conversations),
            Call::File { chat, change } => {
                let base = remote(chat);
                let (method, path, body) = match change {
                    Refile::Pin(on) => (
                        "PUT",
                        format!("{base}/pin"),
                        serde_json::json!({ "pinned": on }).to_string(),
                    ),
                    Refile::Stash(on) => (
                        if *on { "POST" } else { "DELETE" },
                        format!("{base}/archive"),
                        String::new(),
                    ),
                    Refile::Mute(hush) => {
                        let duration = hush.map_or("off", |hush| hush.wire());
                        let body = serde_json::json!({ "duration": duration }).to_string();
                        ("PUT", format!("{base}/mute"), body)
                    }
                };
                self.send(method, &path, body).map(|_| Reply::Done)
            }
            Call::Answer { handle, merge } => {
                let verb = if *merge { "merge" } else { "drop" };
                let path = format!("{}/{verb}", remote(&Chat::Direct(handle.clone())));
                self.send("POST", &path, String::new()).map(|_| Reply::Done)
            }
            Call::Candidates(query) => {
                let path = format!("/api/v1/remotes/candidates?q={}", escape(query));
                let (status, text) = self.send("GET", &path, String::new())?;
                Candidate::parse_list(&text)
                    .map(Reply::Candidates)
                    .map_err(|error| unreadable(status, error))
            }
            Call::Messages { chat, paging } => {
                let mut path = format!("{}/messages", remote(chat));
                match paging {
                    Paging::Latest => {}
                    Paging::Before(id) => path.push_str(&format!("?before={}", id.0)),
                    Paging::After(id) => path.push_str(&format!("?after={}", id.0)),
                }
                let (status, text) = self.send("GET", &path, String::new())?;
                MessagePage::parse(&text)
                    .map(|page| Reply::Messages(Box::new(page)))
                    .map_err(|error| unreadable(status, error))
            }
            Call::MarkRead { chat, through } => {
                let path = format!("{}/read", remote(chat));
                let body = serde_json::json!({ "through": through.0 }).to_string();
                self.send("PUT", &path, body).map(|_| Reply::Done)
            }
            Call::SendMessage {
                chat,
                body,
                client_id,
            } => self
                .send_message(chat, body, client_id)
                .map(|message| Reply::Message(Box::new(message))),
        }
    }

    pub fn conversations(
        &self,
        folder: Folder,
        query: &str,
        before: Option<&str>,
    ) -> Result<ConversationPage, Error> {
        let mut path = String::from("/api/v1/remotes");
        let mut params = Vec::new();
        if folder != Folder::Open {
            params.push(format!("tab={}", folder.tab()));
        }
        if !query.is_empty() {
            params.push(format!("q={}", escape(query)));
        }
        if let Some(before) = before {
            params.push(format!("before={}", escape(before)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        ConversationPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn send_message(
        &self,
        chat: &Chat,
        body: &str,
        client_id: &ClientId,
    ) -> Result<Message, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Message,
        }

        let path = format!("{}/messages", remote(chat));
        let body = serde_json::json!({ "body": body, "client_id": client_id.0 }).to_string();
        let (status, text) = self.send("POST", &path, body)?;
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    fn post(
        &self,
        target: &Target,
        body: &str,
        attachment: &Attachment,
        author: Option<&str>,
    ) -> Result<Focus, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
        }

        let (parent, quoted) = match target {
            Target::New => (None, None),
            Target::Reply(id) => (Some(&id.0), None),
            Target::Quote(id) => (None, Some(&id.0)),
        };
        let mut json =
            serde_json::json!({ "body": body, "parent_id": parent, "quoted_commit_id": quoted });
        if let Some(author) = author {
            json["author"] = author.into();
        }
        let (status, text) = self.attached("/api/v1/commits", json, attachment)?;
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    fn schedule(
        &self,
        body: &str,
        attachment: &Attachment,
        when: &DateTime<FixedOffset>,
        author: Option<&str>,
    ) -> Result<Scheduled, Error> {
        let mut json = serde_json::json!({ "body": body, "scheduled_at": at::wire(when) });
        if let Some(author) = author {
            json["author"] = author.into();
        }
        let (status, text) = self.attached("/api/v1/scheduled-commits", json, attachment)?;
        Scheduled::parse(&text).map_err(|error| unreadable(status, error))
    }

    /// Posts a new commit's `json` with what it carries: a poll inside the JSON, or images as multipart.
    fn attached(
        &self,
        path: &str,
        mut json: serde_json::Value,
        attachment: &Attachment,
    ) -> Result<(u16, String), Error> {
        match attachment {
            Attachment::None => self.send("POST", path, json.to_string()),
            Attachment::Poll(poll) => {
                json["poll"] = serde_json::json!({ "options": poll.options, "duration": poll.duration.minutes() });
                self.send("POST", path, json.to_string())
            }
            Attachment::Images(images) => {
                let descriptions: Vec<&str> = images
                    .iter()
                    .map(|image| image.description.as_str())
                    .collect();
                json["image_descriptions"] = serde_json::json!(descriptions);
                self.send_images("POST", path, json.to_string(), images)
            }
        }
    }

    fn preview(&self, body: &str) -> Result<Vec<Segment>, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Data,
        }

        #[derive(Deserialize)]
        struct Data {
            body_segments: Vec<Segment>,
        }

        let body = serde_json::json!({ "body": body }).to_string();
        let (status, text) = self.send("POST", "/api/v1/commits/preview", body)?;
        parse::<Wire>(status, &text).map(|wire| wire.data.body_segments)
    }

    fn star(&self, id: &CommitId, on: bool) -> Result<Focus, Error> {
        let path = format!("/api/v1/commits/{}/star", escape(&id.0));
        self.focus(if on { "POST" } else { "DELETE" }, &path, String::new())
    }

    fn vote(&self, id: &CommitId, option: &str) -> Result<Focus, Error> {
        let path = format!("/api/v1/commits/{}/poll/votes", escape(&id.0));
        let body = serde_json::json!({ "option": option }).to_string();
        self.focus("POST", &path, body)
    }

    /// A request answered with the commit as its page shows it.
    fn focus(&self, method: &str, path: &str, body: String) -> Result<Focus, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
        }

        let (status, text) = self.send(method, path, body)?;
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    pub fn user(&self, handle: &str) -> Result<Profile, Error> {
        let path = format!("/api/v1/users/{}", escape(handle));
        let (status, text) = self.send("GET", &path, String::new())?;
        Profile::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn commits(
        &self,
        handle: &str,
        replies: bool,
        cursor: Option<&str>,
    ) -> Result<RowPage, Error> {
        let mut path = format!(
            "/api/v1/users/{}/commits?replies={}",
            escape(handle),
            u8::from(replies)
        );
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        RowPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn starred(&self, cursor: Option<&str>) -> Result<RowPage, Error> {
        let mut path = String::from("/api/v1/starred");
        if let Some(cursor) = cursor {
            path.push_str(&format!("?cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        RowPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn people(
        &self,
        handle: &str,
        circle: Circle,
        cursor: Option<&str>,
    ) -> Result<PeoplePage, Error> {
        let mut path = format!("/api/v1/users/{}/{}", escape(handle), circle.slug());
        if let Some(cursor) = cursor {
            path.push_str(&format!("?cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        PeoplePage::parse(&text).map_err(|error| unreadable(status, error))
    }

    /// The people the user blocks, last blocked first.
    pub fn blocked(&self, cursor: Option<&str>) -> Result<PeoplePage, Error> {
        self.listed("/api/v1/me/blocked", cursor)
    }

    /// The people the user mutes, last muted first.
    pub fn muted(&self, cursor: Option<&str>) -> Result<PeoplePage, Error> {
        self.listed("/api/v1/me/muted", cursor)
    }

    fn listed(&self, path: &str, cursor: Option<&str>) -> Result<PeoplePage, Error> {
        let path = match cursor {
            Some(cursor) => format!("{path}?cursor={}", escape(cursor)),
            None => path.to_owned(),
        };
        let (status, text) = self.send("GET", &path, String::new())?;
        PeoplePage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn settings(&self) -> Result<Settings, Error> {
        let (status, text) = self.send("GET", "/api/v1/me/settings", String::new())?;
        Settings::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn devices(&self) -> Result<Vec<Device>, Error> {
        let (status, text) = self.send("GET", "/api/v1/me/devices", String::new())?;
        Device::parse_list(&text).map_err(|error| unreadable(status, error))
    }

    pub fn revoke(&self, id: &DeviceId) -> Result<(), Error> {
        let path = format!("/api/v1/me/devices/{}", escape(&id.0));
        self.send("DELETE", &path, String::new()).map(|_| ())
    }

    pub fn configure(&self, change: &Change) -> Result<Account, Error> {
        let (path, body) = change.request();
        let (status, text) = self.send("PUT", path, body.to_string())?;
        parse(status, &text)
    }

    fn pastel(&self, id: &CommitId) -> Result<Reply, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
            pasteis_remaining: u32,
        }

        let path = format!("/api/v1/commits/{}/pasteis", escape(&id.0));
        let (status, text) = self.send("POST", &path, String::new())?;
        parse::<Wire>(status, &text).map(|wire| Reply::Pastel {
            commit: Box::new(wire.data),
            remaining: wire.pasteis_remaining,
        })
    }

    /// Trades the code from the browser for this device's approval.
    pub fn redeem(&self, code: &str, verifier: &str) -> Result<Account, Error> {
        let body = serde_json::json!({ "code": code, "code_verifier": verifier }).to_string();
        let (status, text) = self.send("POST", "/api/v1/devices", body)?;
        parse(status, &text)
    }

    pub fn me(&self) -> Result<Account, Error> {
        let (status, text) = self.send("GET", "/api/v1/me", String::new())?;
        let account = parse(status, &text)?;
        if let Some(cache) = &self.cache {
            cache.keep_me(&text);
        }
        Ok(account)
    }

    pub fn page(&self, log: Log, query: Option<(&str, &str)>) -> Result<LogPage, Error> {
        let mut path = format!("/api/v1/log/{}", log.slug());
        if let Some((name, value)) = query {
            path.push_str(&format!("?{name}={}", escape(value)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        let page = LogPage::parse(&text).map_err(|error| unreadable(status, error))?;
        if let (Some(cache), None) = (&self.cache, query) {
            cache.keep_page(log, &text);
        }
        Ok(page)
    }

    pub fn commit(&self, id: &CommitId) -> Result<CommitPage, Error> {
        let path = format!("/api/v1/commits/{}", escape(&id.0));
        let (status, text) = self.send("GET", &path, String::new())?;
        CommitPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn replies(&self, id: &CommitId, cursor: Option<&str>) -> Result<RepliesPage, Error> {
        let mut path = format!("/api/v1/commits/{}/replies", escape(&id.0));
        if let Some(cursor) = cursor {
            path.push_str(&format!("?cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        RepliesPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn notifications(
        &self,
        filter: Filter,
        cursor: Option<&str>,
    ) -> Result<NotificationPage, Error> {
        let mut path = format!("/api/v1/notifications?type={}", filter.slug());
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        NotificationPage::parse(&text, filter).map_err(|error| unreadable(status, error))
    }

    pub fn group(
        &self,
        id: &NotificationId,
        filter: Filter,
        cursor: Option<&str>,
    ) -> Result<GroupPage, Error> {
        let mut path = format!(
            "/api/v1/notifications/{}/group?type={}",
            escape(&id.0),
            filter.slug()
        );
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        GroupPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    fn send(&self, method: &str, path: &str, body: String) -> Result<(u16, String), Error> {
        let signed = self
            .key
            .sign(method, path, &body, Utc::now().timestamp(), Nonce::random());
        let request = self.request(method, path, "application/json", &signed, body.into_bytes())?;
        self.exchange(request)
    }

    /// Sends a JSON payload with images as multipart: the payload in a part of its own, the images in `images[]`.
    fn send_images(
        &self,
        method: &str,
        path: &str,
        payload: String,
        images: &[Image],
    ) -> Result<(u16, String), Error> {
        let files = (images.iter())
            .map(|image| {
                fs::read(&image.path).map_err(|error| {
                    let name = image.path.file_name().unwrap_or(image.path.as_os_str());
                    Error::Image(format!("can't read {}: {error}", name.display()))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let hashes: Vec<String> = (files.iter())
            .map(|bytes| {
                Sha256::digest(bytes)
                    .iter()
                    .fold(String::new(), |mut hex, byte| {
                        let _ = write!(hex, "{byte:02x}");
                        hex
                    })
            })
            .collect();
        let signed = self.key.sign_with_images(
            method,
            path,
            &hashes,
            &payload,
            Utc::now().timestamp(),
            Nonce::random(),
        );
        let boundary = format!("ttyl-{}", Nonce::random());
        let mut body = Vec::new();
        let mut part = |name: &str, file: &str, mime: &str, content: &[u8]| {
            body.extend(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\nContent-Type: {mime}\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend(content);
            body.extend(b"\r\n");
        };
        part(
            "payload",
            "payload.json",
            "application/json",
            payload.as_bytes(),
        );
        // A generic name, so the server never learns what the files were called on this machine.
        for (at, bytes) in files.iter().enumerate() {
            let kind = attach::probe(bytes).map(|probe| probe.kind);
            let (extension, mime) = kind.map_or(("bin", "application/octet-stream"), |kind| {
                (kind.extension(), kind.mime())
            });
            part(
                "images[]",
                &format!("image-{}.{extension}", at + 1),
                mime,
                bytes,
            );
        }
        body.extend(format!("--{boundary}--\r\n").as_bytes());
        let content_type = format!("multipart/form-data; boundary={boundary}");
        let request = self.request(method, path, &content_type, &signed, body)?;
        let request = (self.agent.configure_request(request))
            .timeout_global(Some(UPLOAD_TIMEOUT))
            .build();
        self.exchange(request)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        signed: &Signed,
        body: Vec<u8>,
    ) -> Result<Request<Vec<u8>>, Error> {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base))
            .header("Content-Type", content_type)
            .header("Accept", "application/json");
        for (name, value) in signed.headers() {
            request = request.header(name, value);
        }
        request
            .body(body)
            .map_err(|error| Error::Http(error.into()))
    }

    /// The bytes of an image a commit carries. Its address is public, so the request goes unsigned.
    pub fn picture(&self, url: &str) -> Result<Vec<u8>, Error> {
        let mut response = self.agent.get(url).call().map_err(Error::Http)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(Error::Image(format!("/dev/null answered {status} for it.")));
        }
        (response.body_mut().with_config())
            .limit(attach::MAX_BYTES)
            .read_to_vec()
            .map_err(Error::Http)
    }

    fn exchange(&self, request: Request<Vec<u8>>) -> Result<(u16, String), Error> {
        let mut response = self.agent.run(request).map_err(Error::Http)?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(Error::Http)?;
        if response.status().is_success() {
            return Ok((status, text));
        }
        let refusal: Refusal =
            serde_json::from_str(&text).map_err(|_| Error::Unexpected(status))?;
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
            .map(Duration::from_secs);
        let error = Error::Api {
            status,
            message: refusal
                .errors
                .values()
                .flatten()
                .find(|field| refusal.message.starts_with(field.as_str()))
                .cloned()
                .unwrap_or(refusal.message),
            code: refusal.code,
            retry_after,
        };
        if error.needs_sign_in()
            && let Some(cache) = &self.cache
        {
            cache.forget();
        }
        Err(error)
    }
}

fn escape(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Where a conversation's messages and read mark live.
fn remote(chat: &Chat) -> String {
    match chat {
        Chat::Direct(handle) => format!("/api/v1/remotes/@{}", escape(handle)),
        Chat::Org(org) => format!("/api/v1/remotes/orgs/{}", escape(&org.0)),
    }
}

fn parse<T: DeserializeOwned>(status: u16, text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|error| unreadable(status, error))
}

fn unreadable(status: u16, error: serde_json::Error) -> Error {
    match error.classify() {
        serde_json::error::Category::Data => Error::WrongShape(error.to_string()),
        _ => Error::Unexpected(status),
    }
}
