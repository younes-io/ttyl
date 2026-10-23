use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::time::Duration;
use std::{fs, io};

use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ssh_key::sha2::{Digest, Sha256};
use ureq::Agent;
use ureq::http::Request;

use crate::attach::{self, Attachment, Image};
use crate::device::{DeviceKey, Nonce, Signed};
use crate::draft::Target;
use crate::model::{
    Circle, CommitId, CommitPage, Filter, Focus, GroupPage, Log, LogPage, NotificationId,
    NotificationPage, PeoplePage, Profile, Reason, RepliesPage, RowPage,
};

pub const BASE: &str = "https://dev-null.sh";
/// Up to 30 MB of images can take a slow uplink well past the 30 seconds the JSON requests get.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Talks to the API as one device. Every request carries that device's signature.
pub struct Client {
    base: String,
    key: DeviceKey,
    agent: Agent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub username: String,
    pub name: Option<String>,
    #[serde(default)]
    pub pasteis_mode: bool,
    #[serde(default)]
    pub pasteis_remaining: Option<u32>,
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
        username: String,
        on: bool,
    },
    User(String),
    /// Someone's commits, or with `replies` their replies, as their profile lists them.
    Commits {
        username: String,
        replies: bool,
        cursor: Option<String>,
    },
    People {
        username: String,
        circle: Circle,
        cursor: Option<String>,
    },
    Delete(CommitId),
    Pastel(CommitId),
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
    },
    Notifications {
        filter: Filter,
        cursor: Option<String>,
    },
    Group {
        id: NotificationId,
        filter: Filter,
        cursor: Option<String>,
    },
}

#[derive(Debug)]
pub enum Reply {
    Account(Account),
    Page(LogPage),
    Thread(Box<CommitPage>),
    Replies(RepliesPage),
    Commit(Box<Focus>),
    Pastel { commit: Box<Focus>, remaining: u32 },
    User(Box<Profile>),
    Rows(RowPage),
    People(PeoplePage),
    Notifications(NotificationPage),
    Group(GroupPage),
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
            Call::Follow { username, on } => {
                let path = format!("/api/v1/users/{}/follow", escape(username));
                self.send(if *on { "POST" } else { "DELETE" }, &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::User(username) => self
                .user(username)
                .map(|profile| Reply::User(Box::new(profile))),
            Call::Commits {
                username,
                replies,
                cursor,
            } => self
                .commits(username, *replies, cursor.as_deref())
                .map(Reply::Rows),
            Call::People {
                username,
                circle,
                cursor,
            } => self
                .people(username, *circle, cursor.as_deref())
                .map(Reply::People),
            Call::Delete(id) => {
                let path = format!("/api/v1/commits/{}", escape(&id.0));
                self.send("DELETE", &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Pastel(id) => self.pastel(id),
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
            } => self
                .post(target, body, attachment)
                .map(|focus| Reply::Commit(Box::new(focus))),
            Call::Notifications { filter, cursor } => self
                .notifications(*filter, cursor.as_deref())
                .map(Reply::Notifications),
            Call::Group { id, filter, cursor } => {
                self.group(id, *filter, cursor.as_deref()).map(Reply::Group)
            }
        }
    }

    fn post(&self, target: &Target, body: &str, attachment: &Attachment) -> Result<Focus, Error> {
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
        let (status, text) = match attachment {
            Attachment::None => self.send("POST", "/api/v1/commits", json.to_string())?,
            Attachment::Poll(poll) => {
                json["poll"] = serde_json::json!({ "options": poll.options, "duration": poll.duration.minutes() });
                self.send("POST", "/api/v1/commits", json.to_string())?
            }
            Attachment::Images(images) => {
                let descriptions: Vec<&str> = images
                    .iter()
                    .map(|image| image.description.as_str())
                    .collect();
                json["image_descriptions"] = serde_json::json!(descriptions);
                self.send_images("POST", "/api/v1/commits", json.to_string(), images)?
            }
        };
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    fn star(&self, id: &CommitId, on: bool) -> Result<Focus, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
        }

        let path = format!("/api/v1/commits/{}/star", escape(&id.0));
        let (status, text) = self.send(if on { "POST" } else { "DELETE" }, &path, String::new())?;
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    fn vote(&self, id: &CommitId, option: &str) -> Result<Focus, Error> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
        }

        let path = format!("/api/v1/commits/{}/poll/votes", escape(&id.0));
        let body = serde_json::json!({ "option": option }).to_string();
        let (status, text) = self.send("POST", &path, body)?;
        parse::<Wire>(status, &text).map(|wire| wire.data)
    }

    pub fn user(&self, username: &str) -> Result<Profile, Error> {
        let path = format!("/api/v1/users/{}", escape(username));
        let (status, text) = self.send("GET", &path, String::new())?;
        Profile::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn commits(
        &self,
        username: &str,
        replies: bool,
        cursor: Option<&str>,
    ) -> Result<RowPage, Error> {
        let mut path = format!(
            "/api/v1/users/{}/commits?replies={}",
            escape(username),
            u8::from(replies)
        );
        if let Some(cursor) = cursor {
            path.push_str(&format!("&cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        RowPage::parse(&text).map_err(|error| unreadable(status, error))
    }

    pub fn people(
        &self,
        username: &str,
        circle: Circle,
        cursor: Option<&str>,
    ) -> Result<PeoplePage, Error> {
        let mut path = format!("/api/v1/users/{}/{}", escape(username), circle.slug());
        if let Some(cursor) = cursor {
            path.push_str(&format!("?cursor={}", escape(cursor)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        PeoplePage::parse(&text).map_err(|error| unreadable(status, error))
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
        parse(status, &text)
    }

    pub fn page(&self, log: Log, query: Option<(&str, &str)>) -> Result<LogPage, Error> {
        let mut path = format!("/api/v1/log/{}", log.slug());
        if let Some((name, value)) = query {
            path.push_str(&format!("?{name}={}", escape(value)));
        }
        let (status, text) = self.send("GET", &path, String::new())?;
        LogPage::parse(&text).map_err(|error| unreadable(status, error))
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
        Err(Error::Api {
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
        })
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

fn parse<T: DeserializeOwned>(status: u16, text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|error| unreadable(status, error))
}

fn unreadable(status: u16, error: serde_json::Error) -> Error {
    match error.classify() {
        serde_json::error::Category::Data => Error::WrongShape(error.to_string()),
        _ => Error::Unexpected(status),
    }
}
