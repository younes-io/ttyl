use std::env;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::attach;

use base64ct::{Base64, Encoding};

pub fn osc52(text: &str, tmux: bool) -> String {
    let set = format!("\x1b]52;c;{}\x07", Base64::encode_string(text.as_bytes()));
    if tmux {
        // tmux takes the bare sequence when set-clipboard is on and the wrapped one when
        // allow-passthrough is on. Both are off by default, so send both.
        format!("{set}\x1bPtmux;{}\x1b\\", set.replace('\x1b', "\x1b\x1b"))
    } else {
        set
    }
}

pub fn copy(text: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(osc52(text, env::var_os("TMUX").is_some()).as_bytes());
    let _ = out.flush();
    if cfg!(target_os = "macos") {
        spawn("pbcopy", &[], Some(text));
    } else if !spawn("wl-copy", &[], Some(text)) {
        spawn("xclip", &["-selection", "clipboard"], Some(text));
    }
}

pub fn open(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn(opener, &[url], None);
}

fn spawn(program: &str, args: &[&str], input: Option<&str>) -> bool {
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    thread::spawn(move || child.wait());
    true
}

/// A clipboard owner that never answers would hang the composer, so a read gives up after this.
const CLIPBOARD_WAIT: Duration = Duration::from_secs(2);
const IMAGE_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];
const TEXT_TYPES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
];

/// What the clipboard held, best first.
#[derive(Debug, PartialEq, Eq)]
pub enum Clipped {
    Image(Vec<u8>),
    Files(Vec<PathBuf>),
    Text(String),
    Empty,
    /// Neither wl-paste nor xclip could be run, or osascript on macOS.
    Unreachable,
}

/// Reads the clipboard through wl-paste, or xclip where there is no Wayland, or the pasteboard on
/// macOS.
pub fn paste() -> Clipped {
    if cfg!(target_os = "macos") {
        return pasteboard(None);
    }
    let wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    let wl = |kind: Option<&str>| match kind {
        Some(kind) => capture("wl-paste", &["--no-newline", "--type", kind]),
        None => capture("wl-paste", &["--list-types"]),
    };
    let x = |kind: Option<&str>| {
        let kind = kind.unwrap_or("TARGETS");
        capture("xclip", &["-selection", "clipboard", "-t", kind, "-o"])
    };
    // Under Wayland without wl-clipboard, xclip can still reach the clipboard through XWayland.
    let read: &dyn Fn(Option<&str>) -> Option<Vec<u8>>;
    let listed = match wayland.then(|| wl(None)).flatten() {
        Some(listed) => {
            read = &wl;
            listed
        }
        None => match x(None) {
            Some(listed) => {
                read = &x;
                listed
            }
            None => return Clipped::Unreachable,
        },
    };
    let types: Vec<&str> = str::from_utf8(&listed)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .collect();
    let offered = |kinds: &[&'static str]| kinds.iter().copied().find(|kind| types.contains(kind));
    if let Some(kind) = offered(&IMAGE_TYPES)
        && let Some(bytes) = read(Some(kind)).filter(|bytes| !bytes.is_empty())
    {
        return Clipped::Image(bytes);
    }
    if types.contains(&"text/uri-list")
        && let Some(list) = read(Some("text/uri-list"))
        && let Some(paths) = files(&String::from_utf8_lossy(&list))
    {
        return Clipped::Files(paths);
    }
    match offered(&TEXT_TYPES).and_then(|kind| read(Some(kind))) {
        Some(text) if !text.is_empty() => {
            Clipped::Text(String::from_utf8_lossy(&text).into_owned())
        }
        _ => Clipped::Empty,
    }
}

const PASTEBOARD: &str = include_str!("pasteboard.js");

/// Reads the general pasteboard, or the one named `board`. The script prints `files`, `image` or
/// `text` on its first line and the content after it. A Finder copy also carries the file's icon
/// as an image, so files come first.
pub fn pasteboard(board: Option<&str>) -> Clipped {
    let mut args = vec!["-l", "JavaScript", "-e", PASTEBOARD];
    args.extend(board);
    let Some(out) = capture("osascript", &args) else {
        return Clipped::Unreachable;
    };
    let out = String::from_utf8_lossy(&out);
    let out = out.strip_suffix('\n').unwrap_or(&out);
    let (kind, content) = out.split_once('\n').unwrap_or((out, ""));
    match kind {
        "files" => files(content).map_or(Clipped::Empty, Clipped::Files),
        "image" => Base64::decode_vec(content).map_or(Clipped::Empty, Clipped::Image),
        "text" if !content.is_empty() => Clipped::Text(content.to_owned()),
        _ => Clipped::Empty,
    }
}

/// The files a uri list names, its comment lines left out.
fn files(list: &str) -> Option<Vec<PathBuf>> {
    let uris: Vec<&str> = (list.lines().map(str::trim))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    attach::dropped(&uris.join(" "))
}

/// What `program` prints, or None if it can't be run. A program that fails prints nothing.
fn capture(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        bytes
    });
    let until = Instant::now() + CLIPBOARD_WAIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    Some(if status.is_some_and(|status| status.success()) {
        bytes
    } else {
        Vec::new()
    })
}
