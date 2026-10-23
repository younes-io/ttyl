use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 4;
/// The server counts an option's length in characters.
pub const OPTION_LIMIT: usize = 50;

/// What a commit carries besides its body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attachment {
    #[default]
    None,
    Poll(Poll),
    Images(Vec<Image>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub options: Vec<String>,
    pub duration: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    Hour,
    #[default]
    Day,
    ThreeDays,
    Week,
}

const DURATIONS: [Duration; 4] = [
    Duration::Hour,
    Duration::Day,
    Duration::ThreeDays,
    Duration::Week,
];

impl Duration {
    pub fn minutes(self) -> u32 {
        match self {
            Duration::Hour => 60,
            Duration::Day => 1440,
            Duration::ThreeDays => 4320,
            Duration::Week => 10080,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Duration::Hour => "1 hour",
            Duration::Day => "1 day",
            Duration::ThreeDays => "3 days",
            Duration::Week => "7 days",
        }
    }

    /// The next duration up, or down, coming round at the ends.
    pub fn step(self, longer: bool) -> Duration {
        let at = DURATIONS.iter().position(|d| *d == self).unwrap_or(1);
        let by = if longer { 1 } else { DURATIONS.len() - 1 };
        DURATIONS[(at + by) % DURATIONS.len()]
    }
}

impl Poll {
    /// The poll as the server takes it: options trimmed and the empty ones left out.
    pub fn sent(&self) -> Poll {
        Poll {
            options: self
                .options
                .iter()
                .map(|option| option.trim().to_owned())
                .filter(|option| !option.is_empty())
                .collect(),
            duration: self.duration,
        }
    }

    /// Why the server would turn this poll down, in the words the composer shows.
    pub fn refusal(&self) -> Option<String> {
        // Numbered as the composer shows them, empty rows included.
        let filled: Vec<(usize, String)> = (self.options.iter().enumerate())
            .map(|(at, option)| (at + 1, option.trim().to_lowercase()))
            .filter(|(_, option)| !option.is_empty())
            .collect();
        if filled.len() < MIN_OPTIONS {
            return Some("a poll needs at least two options.".to_owned());
        }
        if let Some((at, _)) =
            (filled.iter()).find(|(_, option)| option.chars().count() > OPTION_LIMIT)
        {
            return Some(format!("option {at} is over {OPTION_LIMIT} characters."));
        }
        for (later, (at, option)) in filled.iter().enumerate() {
            if let Some((first, _)) = filled[..later].iter().find(|(_, o)| o == option) {
                return Some(format!("options {first} and {at} are the same."));
            }
        }
        None
    }
}

pub const MAX_IMAGES: usize = 3;
/// The server counts a description's length in characters.
pub const DESCRIPTION_LIMIT: usize = 1000;
/// The server's 10mb, counted as 10,000 kilobytes of 1024 bytes.
pub const MAX_BYTES: u64 = 10_240_000;
pub const MAX_SIDE: u32 = 6000;

/// An image picked to go with a commit. Only its path is kept, so a draft stays small and the file is read when sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub path: PathBuf,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
}

impl Kind {
    pub fn mime(self) -> &'static str {
        match self {
            Kind::Jpeg => "image/jpeg",
            Kind::Png => "image/png",
            Kind::Webp => "image/webp",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Webp => "webp",
        }
    }
}

/// What an image's own header says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
}

/// Reads the kind and size from the first bytes of a file, trusting its contents over its name.
pub fn probe(bytes: &[u8]) -> Option<Probe> {
    let at = |from: usize, len: usize| bytes.get(from..from + len);
    let be16 = |from| at(from, 2).map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])));
    let le16 = |from| at(from, 2).map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])));
    let le24 = |from| at(from, 3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]));
    let be32 = |from| at(from, 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let sized = |kind, width, height| {
        Some(Probe {
            kind,
            width,
            height,
        })
    };

    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && at(12, 4) == Some(b"IHDR") {
        return sized(Kind::Png, be32(16)?, be32(20)?);
    }
    if bytes.starts_with(b"RIFF") && at(8, 4) == Some(b"WEBP") {
        return match at(12, 4)? {
            b"VP8 " if at(23, 3) == Some(&[0x9d, 0x01, 0x2a]) => {
                sized(Kind::Webp, le16(26)? & 0x3fff, le16(28)? & 0x3fff)
            }
            b"VP8L" if at(20, 1) == Some(&[0x2f]) => {
                let bits = u32::from_le_bytes(at(21, 4)?.try_into().ok()?);
                sized(Kind::Webp, (bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
            }
            b"VP8X" => sized(Kind::Webp, le24(24)? + 1, le24(27)? + 1),
            _ => None,
        };
    }
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    // A JPEG's size sits in its start-of-frame segment, after whatever metadata segments come first.
    let mut from = 2;
    loop {
        while at(from, 1)? == [0xff] && at(from + 1, 1)? == [0xff] {
            from += 1;
        }
        let [0xff, marker] = *at(from, 2)? else {
            return None;
        };
        match marker {
            0x01 | 0xd0..=0xd7 => from += 2,
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return sized(Kind::Jpeg, be16(from + 7)?, be16(from + 5)?);
            }
            0xd9 | 0xda => return None,
            _ => from += 2 + usize::try_from(be16(from + 2)?).ok()?,
        }
    }
}

/// Why the server would turn this file down, in the words the composer shows.
pub fn file_refusal(probe: Option<Probe>, bytes: u64) -> Option<String> {
    let Some(probe) = probe else {
        return Some("not a jpeg, png or webp.".to_owned());
    };
    if bytes > MAX_BYTES {
        return Some("over 10 MB.".to_owned());
    }
    if probe.width > MAX_SIDE || probe.height > MAX_SIDE {
        return Some(format!("over {MAX_SIDE} px a side."));
    }
    None
}

/// What a picked file turned out to be when it was last read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    Fine {
        probe: Probe,
        bytes: u64,
    },
    Refused(String),
    /// Nothing is there, or it isn't a file.
    Gone,
}

/// Reads enough of the file at `path` to know whether the server would take it.
pub fn look(path: &Path) -> Look {
    let Ok(meta) = fs::metadata(path) else {
        return Look::Gone;
    };
    if !meta.is_file() {
        return Look::Gone;
    }
    // A JPEG's frame can sit behind a lot of metadata, so a file small enough to send is read whole.
    let head = if meta.len() > MAX_BYTES {
        1 << 20
    } else {
        meta.len()
    };
    let mut bytes = Vec::new();
    if let Err(error) = File::open(path).and_then(|file| file.take(head).read_to_end(&mut bytes)) {
        return Look::Refused(format!("can't read it: {error}."));
    }
    let probe = probe(&bytes);
    match (file_refusal(probe, meta.len()), probe) {
        (None, Some(probe)) => Look::Fine {
            probe,
            bytes: meta.len(),
        },
        (refusal, _) => Look::Refused(refusal.unwrap_or_default()),
    }
}

/// The files named by text a terminal pasted, the way it pastes a file dropped on it, if that's all the text is.
///
/// Terminals quote or backslash-escape a dropped path, or send it as a `file://` uri, and put several on one line
/// or a line each.
pub fn dropped(text: &str) -> Option<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    let mut chars = text.trim().chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(open), _) if ch == open => quote = None,
            (Some('"') | None, '\\') => word.get_or_insert_default().push(chars.next()?),
            (Some(_), _) => word.get_or_insert_default().push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                word.get_or_insert_default();
            }
            (None, _) if ch.is_whitespace() => paths.extend(word.take()),
            (None, _) => word.get_or_insert_default().push(ch),
        }
    }
    if quote.is_some() {
        return None;
    }
    paths.extend(word);
    let paths: Option<Vec<PathBuf>> = paths.iter().map(|word| path(word)).collect();
    paths.filter(|paths| !paths.is_empty())
}

/// An absolute path, or a local `file://` uri with its escapes undone.
fn path(word: &str) -> Option<PathBuf> {
    let Some(rest) = word.strip_prefix("file://") else {
        return word.starts_with('/').then(|| PathBuf::from(word));
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::new();
    let mut rest = rest.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let hex = [rest.next()?, rest.next()?];
            bytes.push(u8::from_str_radix(str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}
