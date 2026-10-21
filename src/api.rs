use std::fmt;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ureq::Agent;
use ureq::http::Request;

use crate::device::{DeviceKey, Nonce};
use crate::model::{CommitId, Log, LogPage};

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
    Redeem { code: String, verifier: String },
    Page(Log),
    Older { log: Log, cursor: String },
    Since { log: Log, since: CommitId },
}

#[derive(Debug)]
pub enum Reply {
    Account(Account),
    Page(LogPage),
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
    /// The server answered with JSON whose shape this build does not read.
    Malformed(String),
}

impl Error {
    /// The server does not know this device, or the user has not approved it yet.
    pub fn needs_sign_in(&self) -> bool {
        matches!(self, Error::Api { code: Some(code), .. } if code == "device_not_approved")
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
            Error::Malformed(detail) => write!(f, "can't read what /dev/null sent: {detail}"),
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
        }
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
        LogPage::parse(&text).map_err(|error| malformed(status, error))
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
    serde_json::from_str(text).map_err(|error| malformed(status, error))
}

/// JSON that does not parse at all is a page from something in front of the server. JSON that parses but has the
/// wrong shape came from the server itself, so its detail names the field to fix.
fn malformed(status: u16, error: serde_json::Error) -> Error {
    match error.classify() {
        serde_json::error::Category::Data => Error::Malformed(error.to_string()),
        _ => Error::Unexpected(status),
    }
}
