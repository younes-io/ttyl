use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{self, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct CommitId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Log {
    Following,
    Latest,
    Trending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Palette(u8),
    Custom { r: u8, g: u8, b: u8 },
}

pub const PALETTE_LEN: u8 = 6;

impl<'de> Deserialize<'de> for Lane {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Index(u8),
            Hex(String),
        }

        match Wire::deserialize(deserializer)? {
            Wire::Index(index) if index < PALETTE_LEN => Ok(Lane::Palette(index)),
            Wire::Index(index) => Err(de::Error::custom(format!(
                "lane {index} is outside the palette"
            ))),
            Wire::Hex(hex) => parse_hex(&hex)
                .map(|(r, g, b)| Lane::Custom { r, g, b })
                .ok_or_else(|| de::Error::custom(format!("lane {hex:?} is not a #rrggbb color"))),
        }
    }
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Segment {
    Text {
        text: String,
    },
    Code {
        code: String,
    },
    Block {
        language: Option<String>,
        html: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub html_url: String,
    pub lane: Lane,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    pub short_hash: String,
    pub body: String,
    pub parent_id: Option<CommitId>,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub stars_count: Option<u32>,
    pub replies_count: Option<u32>,
    pub author: User,
    pub body_segments: Vec<Segment>,
    pub preview_segments: Option<Vec<Segment>>,
    pub status: Status,
    pub is_pinned: bool,
    pub starred: Option<bool>,
    pub quoted_commit_id: Option<CommitId>,
    pub reply_preview: Option<ReplyPreview>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tombstone {
    pub id: CommitId,
    pub parent_id: Option<CommitId>,
    pub deleted_at: DateTime<Utc>,
    pub replies_count: Option<u32>,
}

/// The server sends a deleted commit with replies as a tombstone, told apart from a live commit by a non-null
/// `deleted_at`. Branching on that field keeps serde's error for a malformed commit instead of untagged's
/// "did not match any variant".
#[derive(Debug, Clone)]
pub enum Entry {
    Commit(Box<Commit>),
    Tombstone(Tombstone),
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let deleted = value.get("deleted_at").is_some_and(|at| !at.is_null());
        if deleted {
            Tombstone::deserialize(value)
                .map(Entry::Tombstone)
                .map_err(de::Error::custom)
        } else {
            Commit::deserialize(value)
                .map(|c| Entry::Commit(Box::new(c)))
                .map_err(de::Error::custom)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyPreview {
    pub data: Vec<Entry>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LogPage {
    pub log: Log,
    pub entries: Vec<Entry>,
    pub next_cursor: Option<String>,
    pub is_ranked: bool,
    pub new_commits_count: Option<u32>,
    pub suggestions: Vec<Entry>,
}

impl LogPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Entry>,
            meta: Meta,
            log: Log,
            is_ranked: bool,
            new_commits_count: Option<u32>,
            suggestions: Vec<Entry>,
        }

        #[derive(Deserialize)]
        struct Meta {
            next_cursor: Option<String>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(LogPage {
            log: wire.log,
            entries: wire.data,
            next_cursor: wire.meta.next_cursor,
            is_ranked: wire.is_ranked,
            new_commits_count: wire.new_commits_count,
            suggestions: wire.suggestions,
        })
    }
}
