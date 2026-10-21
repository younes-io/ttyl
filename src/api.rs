use std::fmt;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use ureq::Agent;
use ureq::http::Request;

use crate::device::{DeviceKey, Nonce};

pub const BASE: &str = "https://dev-null.sh";

/// Talks to the API as one device. Every request carries that device's signature.
pub struct Client {
    base: String,
    key: DeviceKey,
    agent: Agent,
}

#[derive(Debug, Deserialize)]
pub struct Me {
    pub username: String,
    pub name: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    /// The server refused the request and said why.
    Api {
        status: u16,
        code: Option<String>,
        message: String,
    },
    Http(ureq::Error),
    /// The server answered with something other than the JSON it documents, such as a proxy's error page.
    Unexpected(u16),
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
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
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

    /// Trades the code from the browser for this device's approval.
    pub fn redeem(&self, code: &str, verifier: &str) -> Result<Me, Error> {
        let body = serde_json::json!({ "code": code, "code_verifier": verifier }).to_string();
        self.send("POST", "/api/v1/devices", body)
    }

    pub fn me(&self) -> Result<Me, Error> {
        self.send("GET", "/api/v1/me", String::new())
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: String,
    ) -> Result<T, Error> {
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
            return serde_json::from_str::<Envelope<T>>(&text)
                .map(|envelope| envelope.data)
                .map_err(|_| Error::Unexpected(status));
        }
        let refusal: Refusal =
            serde_json::from_str(&text).map_err(|_| Error::Unexpected(status))?;
        Err(Error::Api {
            status,
            code: refusal.code,
            message: refusal.message,
        })
    }
}
