use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{self, DeserializeOwned, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct CommitId(pub String);

impl CommitId {
    /// Derived the way the server derives `short_hash`, for a commit the client has no copy of.
    pub fn short_hash(&self) -> String {
        let hex: Vec<char> = self.0.chars().filter(|c| *c != '-').collect();
        hex[hex.len().saturating_sub(7)..].iter().collect()
    }
}

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

impl Log {
    pub fn slug(self) -> &'static str {
        match self {
            Log::Following => "following",
            Log::Latest => "latest",
            Log::Trending => "trending",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Spam,
    Harassment,
    Unsafe,
    Other,
}

impl Reason {
    pub const ALL: [Reason; 4] = [
        Reason::Spam,
        Reason::Harassment,
        Reason::Unsafe,
        Reason::Other,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Reason::Spam => "spam",
            Reason::Harassment => "harassment",
            Reason::Unsafe => "unsafe",
            Reason::Other => "other",
        }
    }

    /// The web's words for each reason.
    pub fn label(self) -> &'static str {
        match self {
            Reason::Spam => "Spam or misleading links",
            Reason::Harassment => "Harassment or hate",
            Reason::Unsafe => "Harmful or private information",
            Reason::Other => "Something else",
        }
    }
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
    if digits.len() != 6
        || !digits
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
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
    pub name: String,
    pub avatar_url: Option<String>,
    pub html_url: String,
    pub lane: Lane,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mention {
    pub username: String,
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    pub short_hash: String,
    pub body: String,
    pub parent_id: Option<CommitId>,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub author: User,
    pub body_segments: Vec<Segment>,
    pub preview_segments: Option<Vec<Segment>>,
    pub mentions: Vec<Mention>,
    pub status: Status,
    pub is_pinned: bool,
    pub pasteis_count: u32,
    pub quoted_commit_id: Option<CommitId>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub site_name: Option<String>,
    pub is_pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub description: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Poll {
    pub id: String,
    pub closes_at: Option<DateTime<Utc>>,
    pub is_closed: bool,
    pub has_voted: bool,
    pub total_votes: Option<u32>,
    pub options: Vec<PollOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PollOption {
    pub id: String,
    pub label: String,
    pub votes_count: Option<u32>,
    pub voted: bool,
    pub percentage: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Media {
    pub link_preview: Option<LinkPreview>,
    pub images: Vec<Image>,
    pub poll: Option<Poll>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    #[serde(flatten)]
    pub commit: Commit,
    #[serde(flatten)]
    pub media: Media,
    pub stars_count: u32,
    pub starred: bool,
    pub replies_count: u32,
    pub quoted_commit: Option<Shown<Quote>>,
    pub reply_preview: ReplyPreview,
    #[serde(skip)]
    pub deletion: Option<Deletion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deletion {
    Running,
    Done,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Card {
    #[serde(flatten)]
    pub commit: Commit,
    #[serde(flatten)]
    pub media: Media,
    pub stars_count: u32,
    pub starred: bool,
    pub quoted_commit: Option<Shown<Quote>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Quote {
    #[serde(flatten)]
    pub commit: Commit,
    #[serde(flatten)]
    pub media: Media,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reply {
    #[serde(flatten)]
    pub commit: Commit,
    #[serde(flatten)]
    pub media: Media,
    pub stars_count: u32,
    pub starred: bool,
    pub replies_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Suggestion {
    #[serde(flatten)]
    pub commit: Commit,
    pub stars_count: u32,
    pub starred: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tombstone {
    pub id: CommitId,
    pub parent_id: Option<CommitId>,
    pub deleted_at: DateTime<Utc>,
    pub replies_count: Option<u32>,
}

impl Tombstone {
    pub fn short_hash(&self) -> String {
        self.id.short_hash()
    }
}

/// Branching on `deleted_at` keeps serde's error for a malformed commit instead of untagged's "did not match any
/// variant".
#[derive(Debug, Clone)]
pub enum Shown<T> {
    Live(Box<T>),
    Deleted(Tombstone),
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Shown<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let deleted = value.get("deleted_at").is_some_and(|at| !at.is_null());
        if deleted {
            Tombstone::deserialize(value)
                .map(Shown::Deleted)
                .map_err(de::Error::custom)
        } else {
            T::deserialize(value)
                .map(|live| Shown::Live(Box::new(live)))
                .map_err(de::Error::custom)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplyPreview {
    pub data: Vec<Shown<Reply>>,
    pub next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Meta {
    next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LogPage {
    pub log: Log,
    pub rows: Vec<Row>,
    pub next_cursor: Option<String>,
    pub is_ranked: bool,
    pub new_commits_count: Option<u32>,
    pub suggestions: Vec<Suggestion>,
}

impl LogPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Row>,
            meta: Meta,
            log: Log,
            is_ranked: bool,
            new_commits_count: Option<u32>,
            suggestions: Vec<Suggestion>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(LogPage {
            log: wire.log,
            rows: wire.data,
            next_cursor: wire.meta.next_cursor,
            is_ranked: wire.is_ranked,
            new_commits_count: wire.new_commits_count,
            suggestions: wire.suggestions,
        })
    }
}

/// A commit as its own page shows it, which is also what a star or a pastel answers with.
#[derive(Debug, Clone, Deserialize)]
pub struct Focus {
    #[serde(flatten)]
    pub card: Card,
    pub replies_count: u32,
}

#[derive(Debug, Clone)]
pub struct CommitPage {
    pub commit: Card,
    pub replies_count: u32,
    pub ancestors: Vec<Shown<Card>>,
    pub earlier: Option<Shown<Card>>,
}

impl CommitPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Focus,
            ancestors: Vec<Shown<Ancestor>>,
        }

        #[derive(Deserialize)]
        struct Ancestor {
            #[serde(flatten)]
            card: Card,
            parent: Option<Shown<Card>>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        let mut earlier = None;
        let ancestors = wire
            .ancestors
            .into_iter()
            .enumerate()
            .map(|(at, ancestor)| match ancestor {
                Shown::Live(ancestor) => {
                    let Ancestor { card, parent } = *ancestor;
                    if at == 0 {
                        earlier = parent;
                    }
                    Shown::Live(Box::new(card))
                }
                Shown::Deleted(tombstone) => Shown::Deleted(tombstone),
            })
            .collect();
        Ok(CommitPage {
            commit: wire.data.card,
            replies_count: wire.data.replies_count,
            ancestors,
            earlier,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RepliesPage {
    pub replies: Vec<Shown<Reply>>,
    pub next_cursor: Option<String>,
}

impl RepliesPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Shown<Reply>>,
            meta: Meta,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(RepliesPage {
            replies: wire.data,
            next_cursor: wire.meta.next_cursor,
        })
    }
}
