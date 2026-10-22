use std::fmt;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ureq::Agent;
use ureq::http::Request;

use crate::device::{DeviceKey, Nonce};
use crate::model::{CommitId, CommitPage, Focus, Log, LogPage, Reason, RepliesPage};

pub const BASE: &str = "https://dev-null.sh";

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
    Delete(CommitId),
    Pastel(CommitId),
    Report {
        id: CommitId,
        reason: Reason,
        details: Option<String>,
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
    User { is_following: bool },
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
}

impl Error {
    /// The server does not know this device, or the user has not approved it yet.
    pub fn needs_sign_in(&self) -> bool {
        matches!(self, Error::Api { code: Some(code), .. } if code == "device_not_approved")
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
            Error::WrongShape(_) => Failure::Refused(error.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct Refusal {
    message: String,
    code: Option<String>,
}

impl Client {
    pub fn new(base: impl Into<String>, key: DeviceKey) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("devnull-tui/", env!("CARGO_PKG_VERSION")))
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
            Call::User(username) => self.user(username),
            Call::Delete(id) => {
                let path = format!("/api/v1/commits/{}", escape(&id.0));
                self.send("DELETE", &path, String::new())
                    .map(|_| Reply::Done)
            }
            Call::Pastel(id) => self.pastel(id),
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
        }
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

    fn user(&self, username: &str) -> Result<Reply, Error> {
        #[derive(Deserialize)]
        struct Wire {
            is_following: bool,
        }

        let path = format!("/api/v1/users/{}", escape(username));
        let (status, text) = self.send("GET", &path, String::new())?;
        parse::<Wire>(status, &text).map(|wire| Reply::User {
            is_following: wire.is_following,
        })
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

    fn send(&self, method: &str, path: &str, body: String) -> Result<(u16, String), Error> {
        let signed = self
            .key
            .sign(method, path, &body, Utc::now().timestamp(), Nonce::random());
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        for (name, value) in signed.headers() {
            request = request.header(name, value);
        }
        let request = request
            .body(body)
            .map_err(|error| Error::Http(error.into()))?;
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
            code: refusal.code,
            message: refusal.message,
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
