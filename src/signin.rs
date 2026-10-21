use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use base64ct::{Base64UrlUnpadded, Encoding};
use ssh_key::rand_core::{OsRng, RngCore};
use ssh_key::sha2::{Digest, Sha256};

use crate::device::Nonce;

pub const REDIRECT_URI: &str = "devnull://signed-in";
const MIME: &str = "x-scheme-handler/devnull";
const DESKTOP_FILE: &str = "devnull-tui.desktop";

/// One trip through the browser. The verifier stays here and only its challenge leaves, and the state ties the
/// redirect back to this attempt.
pub struct Attempt {
    pub verifier: String,
    pub state: String,
}

impl Attempt {
    pub fn random() -> Self {
        let mut bytes = [0; 32];
        OsRng.fill_bytes(&mut bytes);
        Attempt {
            verifier: Base64UrlUnpadded::encode_string(&bytes),
            state: Nonce::random().to_string(),
        }
    }

    pub fn challenge(&self) -> String {
        Base64UrlUnpadded::encode_string(&Sha256::digest(self.verifier.as_bytes()))
    }

    pub fn authorize_url(&self, base: &str, fingerprint: &str, device_name: &str) -> String {
        let query = [
            ("fingerprint", fingerprint),
            ("device_name", device_name),
            ("code_challenge", &self.challenge()),
            ("code_challenge_method", "S256"),
            ("state", &self.state),
            ("redirect_uri", REDIRECT_URI),
        ]
        .map(|(name, value)| format!("{name}={}", encode(value)))
        .join("&");
        format!("{base}/mobile/authorize?{query}")
    }

    /// Reads the redirect the browser hands back. Anything that is not this attempt's redirect, such as a stale tab
    /// from an earlier attempt, gives `None` so the caller keeps waiting.
    pub fn callback(&self, url: &str) -> Option<Callback> {
        let query = url.strip_prefix(REDIRECT_URI)?.strip_prefix('?')?;
        let mut code = None;
        let mut error = None;
        let mut state = None;
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=')?;
            let slot = match name {
                "code" => &mut code,
                "error" => &mut error,
                "state" => &mut state,
                _ => continue,
            };
            *slot = Some(decode(value)?);
        }
        if state? != self.state {
            return None;
        }
        match (code, error) {
            (Some(code), None) => Some(Callback::Approved(code)),
            (None, Some(_)) => Some(Callback::Denied),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Callback {
    Approved(String),
    Denied,
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn decode(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.as_bytes();
    while let [first, tail @ ..] = rest {
        rest = tail;
        match first {
            b'+' => bytes.push(b' '),
            b'%' => {
                let [high, low, tail @ ..] = rest else {
                    return None;
                };
                let digit = |byte: &u8| char::from(*byte).to_digit(16);
                bytes.push(u8::try_from(digit(high)? * 16 + digit(low)?).ok()?);
                rest = tail;
            }
            _ => bytes.push(*first),
        }
    }
    String::from_utf8(bytes).ok()
}

/// Where the waiting app listens for the redirect. The runtime directory is private to the user.
pub fn socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("devnull-tui.sock")
}

/// What the desktop runs when the browser opens a `devnull://` link: hand the link to the waiting app and exit.
pub fn forward(url: &str) -> io::Result<()> {
    UnixStream::connect(socket_path())?.write_all(url.as_bytes())
}

pub struct Inbox {
    listener: UnixListener,
    path: PathBuf,
}

impl Inbox {
    pub fn bind() -> io::Result<Self> {
        let path = socket_path();
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Inbox { listener, path })
    }

    /// The next link a handler forwarded, if one is waiting. Returns at once so the screen keeps answering keys.
    pub fn poll(&self) -> io::Result<Option<String>> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut url = String::new();
        stream.take(4096).read_to_string(&mut url)?;
        Ok(Some(url))
    }
}

impl Drop for Inbox {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Makes this binary the `devnull://` handler while sign-in runs, then gives the scheme back to whatever owned it
/// before, so another client's handler keeps working afterwards.
pub struct Handler {
    desktop_file: PathBuf,
    previous: Option<String>,
}

impl Handler {
    pub fn install() -> io::Result<Self> {
        let applications = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .ok_or_else(|| io::Error::other("no home directory"))?
            .join("applications");
        fs::create_dir_all(&applications)?;
        let exe = env::current_exe()?;
        // xdg-open reads the first word of Exec as the binary and does not undo desktop entry quoting.
        if exe
            .to_str()
            .is_none_or(|exe| exe.contains(|ch: char| ch.is_whitespace() || "\"'\\`$".contains(ch)))
        {
            return Err(io::Error::other(format!(
                "{} has a space or quote in its path, which the desktop cannot launch",
                exe.display()
            )));
        }
        let desktop_file = applications.join(DESKTOP_FILE);
        fs::write(
            &desktop_file,
            format!(
                "[Desktop Entry]\nType=Application\nName=devnull-tui sign-in\nExec={} --redirect %u\nMimeType={MIME};\nNoDisplay=true\nTerminal=false\n",
                exe.display()
            ),
        )?;
        let previous = xdg_mime(&["query", "default", MIME])?;
        xdg_mime(&["default", DESKTOP_FILE, MIME])?;
        Ok(Handler {
            desktop_file,
            previous: previous.filter(|previous| previous != DESKTOP_FILE),
        })
    }
}

impl Drop for Handler {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            let _ = xdg_mime(&["default", previous, MIME]);
        }
        let _ = fs::remove_file(&self.desktop_file);
    }
}

fn xdg_mime(args: &[&str]) -> io::Result<Option<String>> {
    let output = Command::new("xdg-mime").args(args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!stdout.is_empty()).then_some(stdout))
}
