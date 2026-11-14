use std::borrow::Cow;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde::de::{self, DeserializeOwned, Deserializer};

use crate::code;

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

/// A block is highlighted once on arrival, since feed layout reruns on every frame.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "WireSegment")]
pub enum Segment {
    Text {
        text: String,
    },
    Code {
        code: String,
    },
    Block {
        language: Option<String>,
        lines: Vec<code::Line>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireSegment {
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

impl From<WireSegment> for Segment {
    fn from(wire: WireSegment) -> Self {
        match wire {
            WireSegment::Text { text } => Segment::Text { text },
            WireSegment::Code { code } => Segment::Code { code },
            WireSegment::Block { language, html } => Segment::Block {
                lines: code::highlight(language.as_deref(), &html),
                language,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: UserId,
    pub handle: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub html_url: String,
    pub lane: Lane,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mention {
    pub handle: String,
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
    /// The option the user is on while they pick one to vote for. The server never sends it.
    #[serde(skip)]
    pub picking: Option<usize>,
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
    /// `None` where the server didn't load it, as `search/commits` and `commits/{id}/related` on
    /// dev-null.sh do, which says nothing about the commit's replies.
    #[serde(default)]
    pub reply_preview: Option<ReplyPreview>,
    #[serde(skip)]
    pub deletion: Option<Deletion>,
    /// Why the log folds the commit away: the user just muted or blocked its author.
    #[serde(skip)]
    pub fold: Option<Fold>,
    /// Whether the body shows a translation, or one is on its way.
    #[serde(skip)]
    pub showing: Showing,
}

/// What the body of a commit reads as: the words its author wrote, or the reader's language.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Showing {
    #[default]
    Original,
    /// The original, while its translation is on its way.
    Translating,
    /// The translation, from the language named.
    Translation { from: String },
}

/// What dev-null.sh made of translating a commit into the reader's language.
#[derive(Debug, Clone)]
pub enum Translation {
    Done(Translated),
    Skipped(Untranslated),
}

#[derive(Debug, Clone)]
pub struct Translated {
    /// The language it was written in, named in English, such as `French`.
    pub from: String,
    pub body_segments: Vec<Segment>,
    pub preview_segments: Option<Vec<Segment>>,
    /// The translation as Markdown, for what a pipe reads.
    pub body: String,
}

/// Why a commit has no translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Untranslated {
    /// It is already in the reader's language, named in English.
    Already(String),
    /// It is in no language, such as emoji or a link alone.
    NoLanguage,
}

impl Translation {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            translation: Option<Display>,
        }

        #[derive(Deserialize)]
        struct Display {
            language: String,
            source_language: Option<String>,
            source_name: Option<String>,
            available: bool,
            translated: Option<Body>,
        }

        #[derive(Deserialize)]
        struct Body {
            body_segments: Vec<Segment>,
            preview_segments: Option<Vec<Segment>>,
        }

        let Some(display) = serde_json::from_str::<Wire>(json)?.translation else {
            return Ok(Translation::Skipped(Untranslated::NoLanguage));
        };
        Ok(
            match (display.available, display.translated, display.source_name) {
                (true, Some(body), Some(from)) => Translation::Done(Translated {
                    from,
                    body: markdown(&body.body_segments),
                    body_segments: body.body_segments,
                    preview_segments: body.preview_segments,
                }),
                (_, _, Some(name))
                    if display.source_language.as_ref() == Some(&display.language) =>
                {
                    Translation::Skipped(Untranslated::Already(name))
                }
                _ => Translation::Skipped(Untranslated::NoLanguage),
            },
        )
    }
}

/// Writes segments back as the Markdown they were split from. dev-null.sh drops the line breaks
/// around a code block's fences, so they go back in.
pub fn markdown(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            Segment::Text { text } => out.push_str(text),
            Segment::Code { code } => {
                out.push('`');
                out.push_str(code);
                out.push('`');
            }
            Segment::Block { language, lines } => {
                if !out.is_empty() {
                    out.push('\n');
                }
                let longest = (lines.iter())
                    .map(|line| {
                        let text: String = line.iter().map(|span| span.text.as_str()).collect();
                        let text = text.trim_start_matches(' ');
                        text.len() - text.trim_start_matches('`').len()
                    })
                    .max()
                    .unwrap_or(0);
                // A fence closes on a line of as many backticks, so it outruns any the code starts a line with.
                let fence = "`".repeat((longest + 1).max(3));
                out.push_str(&fence);
                out.push_str(language.as_deref().unwrap_or(""));
                for line in lines {
                    out.push('\n');
                    for span in line {
                        out.push_str(&span.text);
                    }
                }
                out.push('\n');
                out.push_str(&fence);
                out.push('\n');
            }
        }
    }
    if matches!(segments.last(), Some(Segment::Block { .. })) {
        out.pop();
    }
    out
}

/// What the user set on dev-null.sh that ttyl reads.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The language commits translate into, or `None` to follow the device.
    pub translation_language: Option<String>,
    /// The codes of the languages dev-null.sh translates into.
    pub languages: Vec<String>,
}

impl Settings {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Data,
        }

        #[derive(Deserialize)]
        struct Data {
            translation_language: Option<String>,
            translation_languages: Vec<Language>,
        }

        #[derive(Deserialize)]
        struct Language {
            code: String,
        }

        let data = serde_json::from_str::<Wire>(json)?.data;
        Ok(Settings {
            translation_language: data.translation_language,
            languages: (data.translation_languages.into_iter())
                .map(|language| language.code)
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    Muted,
    Blocked,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct NotificationId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Followed,
    Replied,
    Mentioned,
    Approved,
    Rejected,
    Quoted,
    Starred,
    LeveledUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    #[default]
    All,
    Replies,
    Quotes,
    Stars,
    Mentions,
    Follows,
}

impl Filter {
    pub const ALL: [Filter; 6] = [
        Filter::All,
        Filter::Replies,
        Filter::Quotes,
        Filter::Stars,
        Filter::Mentions,
        Filter::Follows,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Replies => "replies",
            Filter::Quotes => "quotes",
            Filter::Stars => "stars",
            Filter::Mentions => "mentions",
            Filter::Follows => "follows",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: NotificationId,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub reason: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub actor: User,
    pub commit: Option<Shown<Commit>>,
}

impl Event {
    pub fn commit_id(&self) -> Option<&CommitId> {
        match self.commit.as_ref()? {
            Shown::Live(commit) => Some(&commit.id),
            Shown::Deleted(tombstone) => Some(&tombstone.id),
        }
    }

    /// The one person a row about this event is about, whom it folds under once blocked:
    /// whoever followed, or the commit's author. A group of several people has none.
    pub fn subject(&self, alone: bool) -> Option<&str> {
        match (&self.kind, &self.commit) {
            _ if !alone => None,
            (Kind::Followed, _) => Some(&self.actor.handle),
            (_, Some(Shown::Live(commit))) => Some(&commit.author.handle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    #[serde(flatten)]
    pub event: Event,
    pub event_count: u32,
    pub actors_count: u32,
}

impl Group {
    /// One event on its own, as the group list shows each.
    pub fn single(event: Event) -> Self {
        Group {
            event,
            event_count: 1,
            actors_count: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotificationPage {
    pub filter: Filter,
    pub groups: Vec<Group>,
    pub read_at: DateTime<Utc>,
    pub next_cursor: Option<String>,
}

impl NotificationPage {
    pub fn parse(json: &str, filter: Filter) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Group>,
            meta: Meta,
            read_at: DateTime<Utc>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(NotificationPage {
            filter,
            groups: wire.data,
            read_at: wire.read_at,
            next_cursor: wire.meta.next_cursor,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GroupPage {
    pub events: Vec<Event>,
    pub next_cursor: Option<String>,
}

impl GroupPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Event>,
            meta: Meta,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(GroupPage {
            events: wire.data,
            next_cursor: wire.meta.next_cursor,
        })
    }
}

/// Someone as their profile and the lists of people show them.
#[derive(Debug, Clone, Deserialize)]
pub struct Person {
    #[serde(flatten)]
    pub user: User,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    #[serde(default)]
    pub github_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub commits_count: Option<u32>,
    pub followers_count: Option<u32>,
    pub following_count: Option<u32>,
    pub is_followed: Option<bool>,
    pub follows_you: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub person: Person,
    pub is_following: bool,
    pub follows_you: bool,
    pub pinned: Option<Row>,
    pub activity: Option<Activity>,
    pub streaks: Option<Streaks>,
    pub level: Option<Level>,
}

/// How far back an activity graph reaches, the smallest window that holds all of someone's commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    Day,
    Week,
    Month,
    Year,
}

/// Commits per hour (a day's graph) or per day (the others), oldest first.
#[derive(Debug, Clone)]
pub struct Activity {
    pub scale: Scale,
    /// The day the first bucket falls on, which lines a year's weeks up by weekday.
    pub first: NaiveDate,
    pub counts: Vec<u32>,
}

impl<'de> Deserialize<'de> for Activity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Bucket {
            date: String,
            count: u32,
        }
        #[derive(Deserialize)]
        struct Wire {
            scale: Scale,
            buckets: Vec<Bucket>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let first = match wire.buckets.first() {
            Some(bucket) => bucket
                .date
                .get(..10)
                .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
                .ok_or_else(|| de::Error::custom(format!("bad bucket date {:?}", bucket.date)))?,
            None => NaiveDate::default(),
        };
        Ok(Activity {
            scale: wire.scale,
            first,
            counts: wire
                .buckets
                .into_iter()
                .map(|bucket| bucket.count)
                .collect(),
        })
    }
}

/// Days in a row with a commit. `current` is 0 once a day passes without one.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Streaks {
    pub longest: u32,
    pub current: u32,
}

/// Their Unix rank, from the XP their commits, streaks, stars and replies earn.
#[derive(Debug, Clone, Deserialize)]
pub struct Level {
    pub level: u32,
    pub rank: String,
    pub xp: u32,
    /// None at the top rank.
    pub next_level_xp: Option<u32>,
    /// Percent of the way from this rank to the next.
    pub level_progress: u8,
}

impl Profile {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Person,
            is_following: bool,
            follows_you: bool,
            pinned_commit: Option<Row>,
            #[serde(default)]
            activity: Option<Activity>,
            #[serde(default)]
            streaks: Option<Streaks>,
            #[serde(default)]
            achievements: Option<Level>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(Profile {
            person: wire.data,
            is_following: wire.is_following,
            follows_you: wire.follows_you,
            pinned: wire.pinned_commit,
            activity: wire.activity,
            streaks: wire.streaks,
            level: wire.achievements,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RowPage {
    pub rows: Vec<Row>,
    pub next_cursor: Option<String>,
}

impl RowPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Row>,
            meta: Meta,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(RowPage {
            rows: wire.data,
            next_cursor: wire.meta.next_cursor,
        })
    }

    /// The commits a search finds, closest first, all on one page: `search/commits` and `commits/{id}/related`.
    pub fn parse_search(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Row>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(RowPage {
            rows: wire.data,
            next_cursor: None,
        })
    }
}

/// The people on either side of someone's follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Circle {
    Followers,
    Following,
}

impl Circle {
    pub fn slug(self) -> &'static str {
        match self {
            Circle::Followers => "followers",
            Circle::Following => "following",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeoplePage {
    pub people: Vec<Person>,
    pub next_cursor: Option<String>,
}

impl PeoplePage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Person>,
            meta: Meta,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(PeoplePage {
            people: wire.data,
            next_cursor: wire.meta.next_cursor,
        })
    }

    /// The people `search/profiles` finds. Organizations, which ttyl has no page for, are left out.
    pub fn parse_search(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            users: Vec<Person>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(PeoplePage {
            people: wire.users,
            next_cursor: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct HeadlineId(pub String);

/// A topic dev-null.sh is talking about right now, as the web's hot topics list it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Headline {
    pub id: HeadlineId,
    pub name: String,
    /// A short recap, only while one is fresh and none of its sources is hidden from the viewer.
    pub summary: Option<String>,
}

impl Headline {
    /// The hot headlines `headlines` lists, hottest first.
    pub fn parse_list(json: &str) -> serde_json::Result<Vec<Headline>> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Headline>,
        }

        Ok(serde_json::from_str::<Wire>(json)?.data)
    }
}

/// One headline's commits from the past week, newest first, with its name and recap.
#[derive(Debug, Clone)]
pub struct HeadlinePage {
    pub name: String,
    pub summary: Option<String>,
    pub rows: Vec<Row>,
}

impl HeadlinePage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Row>,
            name: String,
            summary: Option<String>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(HeadlinePage {
            name: wire.name,
            summary: wire.summary,
            rows: wire.data,
        })
    }
}

/// A message's place in its conversation. Later messages have higher ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(transparent)]
pub struct MessageId(pub u64);

/// The id a sender gives a message, so that sending it again lands it once.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct ClientId(pub String);

/// What a message carries besides its text, shown as a mark since a terminal can't play or show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extra {
    Images,
    Voice,
    Ping,
    Call,
}

impl Extra {
    pub fn mark(self) -> &'static str {
        match self {
            Extra::Images => "[image]",
            Extra::Voice => "[voice note]",
            Extra::Ping => "[ping]",
            Extra::Call => "[call]",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub user_id: UserId,
    /// Who wrote it, which dev-null.sh says in an org's thread and in the list of conversations.
    pub author: Option<User>,
    pub client_id: Option<ClientId>,
    /// Empty when the message is only an image, a voice note or an event.
    pub body: String,
    pub extra: Option<Extra>,
    pub notice: Option<Notice>,
    pub created_at: DateTime<Utc>,
}

/// A change to an org that its history records between messages, with the handle of whom it was
/// about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The body holds the name it was created with.
    Created,
    Added(String),
    Removed(String),
    Left,
    /// The body holds the new name, empty when the name was taken away.
    Renamed,
    Promoted(String),
    Demoted(String),
    /// The last maintainer left, and this member took over.
    Inherited,
}

impl Notice {
    fn parse(event: &str, subject: Option<User>) -> Option<Self> {
        let subject = || subject.map_or_else(|| "someone".to_owned(), |user| user.handle);
        Some(match event {
            "created" => Notice::Created,
            "added" => Notice::Added(subject()),
            "removed" => Notice::Removed(subject()),
            "left" => Notice::Left,
            "renamed" => Notice::Renamed,
            "promoted" => Notice::Promoted(subject()),
            "demoted" => Notice::Demoted(subject()),
            "inherited" => Notice::Inherited,
            _ => return None,
        })
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            id: MessageId,
            user_id: UserId,
            #[serde(default)]
            user: Option<User>,
            client_id: Option<ClientId>,
            body: Option<String>,
            #[serde(default)]
            images: Option<Vec<serde::de::IgnoredAny>>,
            #[serde(default)]
            voice: Option<serde::de::IgnoredAny>,
            event: Option<String>,
            #[serde(default)]
            subject: Option<User>,
            created_at: DateTime<Utc>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let body = wire.body.unwrap_or_default();
        let notice = (wire.event.as_deref()).and_then(|event| Notice::parse(event, wire.subject));
        let extra = match wire.event.as_deref() {
            _ if notice.is_some() => None,
            Some("ping") => Some(Extra::Ping),
            Some(event) if event.starts_with("call_") => Some(Extra::Call),
            _ if wire.voice.is_some() => Some(Extra::Voice),
            _ if wire.images.is_some_and(|images| !images.is_empty()) => Some(Extra::Images),
            // The inbox leaves a last message's images out, and only images let a message go without text.
            _ if body.is_empty() => Some(Extra::Images),
            _ => None,
        };
        Ok(Message {
            id: wire.id,
            user_id: wire.user_id,
            author: wire.user,
            client_id: wire.client_id,
            body,
            extra,
            notice,
            created_at: wire.created_at,
        })
    }
}

impl Message {
    /// The first line of the text, or what a notice says, or the mark of what it carries instead.
    pub fn preview(&self) -> Cow<'_, str> {
        if let Some(told) = self.told() {
            return Cow::Owned(told);
        }
        Cow::Borrowed(
            match (
                self.body.lines().find(|line| !line.trim().is_empty()),
                self.extra,
            ) {
                (Some(line), _) => line.trim(),
                (None, Some(extra)) => extra.mark(),
                (None, None) => "",
            },
        )
    }

    /// What a notice says, as `grace added linus`.
    pub fn told(&self) -> Option<String> {
        let notice = self.notice.as_ref()?;
        let actor = (self.author.as_ref()).map_or("someone", |author| author.handle.as_str());
        let name = self.body.trim();
        Some(match notice {
            Notice::Created if name.is_empty() => format!("{actor} created the group"),
            Notice::Created => format!("{actor} created #{name}"),
            Notice::Renamed if name.is_empty() => format!("{actor} took the group's name away"),
            Notice::Renamed => format!("{actor} renamed the group #{name}"),
            Notice::Added(who) => format!("{actor} added {who}"),
            Notice::Removed(who) => format!("{actor} removed {who}"),
            Notice::Left => format!("{actor} left"),
            Notice::Promoted(who) => format!("{actor} made {who} a maintainer"),
            Notice::Demoted(who) => format!("{actor} made {who} a contributor"),
            Notice::Inherited => format!("{actor} took over as maintainer"),
        })
    }
}

/// An org's id on dev-null.sh, a UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct OrgId(pub String);

/// Which conversation: the one with a person, by handle, or an org's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Chat {
    Direct(String),
    Org(OrgId),
}

/// A group conversation, which dev-null.sh calls an org.
#[derive(Debug, Clone)]
pub struct Org {
    pub id: OrgId,
    pub name: Option<String>,
    /// Maintainers first, then by name, the user among them.
    pub members: Vec<User>,
}

impl Org {
    /// The name, or for an org without one, who is in it.
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => (self.members.iter())
                .map(|member| member.handle.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// Who a conversation is with: one person, or the members of an org.
#[derive(Debug, Clone)]
pub enum Party {
    Person(User),
    Org(Org),
}

impl Party {
    pub fn chat(&self) -> Chat {
        match self {
            Party::Person(user) => Chat::Direct(user.handle.clone()),
            Party::Org(org) => Chat::Org(org.id.clone()),
        }
    }

    pub fn is(&self, chat: &Chat) -> bool {
        match (self, chat) {
            (Party::Person(user), Chat::Direct(handle)) => user.handle == *handle,
            (Party::Org(org), Chat::Org(id)) => org.id == *id,
            _ => false,
        }
    }

    /// `@grace`, or `#unix-room` the way an IRC client names a channel.
    pub fn title(&self) -> String {
        match self {
            Party::Person(user) => format!("@{}", user.handle),
            Party::Org(org) => format!("#{}", org.label()),
        }
    }
}

#[derive(Deserialize)]
struct Member {
    user: User,
}

/// Which of the user's conversations a list shows, as dev-null.sh files them, like mutt's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Folder {
    #[default]
    Open,
    /// Conversations someone started with the user, waiting to be merged or dropped.
    Requests,
    Stashed,
}

impl Folder {
    pub const ALL: [Folder; 3] = [Folder::Open, Folder::Requests, Folder::Stashed];

    pub fn slug(self) -> &'static str {
        match self {
            Folder::Open => "open",
            Folder::Requests => "requests",
            Folder::Stashed => "stashed",
        }
    }

    /// The tab dev-null.sh's inbox calls it.
    pub fn tab(self) -> &'static str {
        match self {
            Folder::Open => "open",
            Folder::Requests => "fetched",
            Folder::Stashed => "stashed",
        }
    }
}

/// How the user keeps a conversation: pinned to the top, stashed out of the open ones, muted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Filed {
    pub pinned: bool,
    pub stashed: bool,
    pub muted: bool,
}

/// How long a conversation stays muted, as dev-null.sh offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hush {
    Hour,
    /// Until the day turns.
    Tomorrow,
    Week,
    Forever,
}

impl Hush {
    pub const ALL: [Hush; 4] = [Hush::Hour, Hush::Tomorrow, Hush::Week, Hush::Forever];

    pub fn wire(self) -> &'static str {
        match self {
            Hush::Hour => "hour",
            Hush::Tomorrow => "tomorrow",
            Hush::Week => "week",
            Hush::Forever => "forever",
        }
    }

    /// How long, as `muted @grace for an hour` says it.
    pub fn label(self) -> &'static str {
        match self {
            Hush::Hour => "for an hour",
            Hush::Tomorrow => "till tomorrow",
            Hush::Week => "for a week",
            Hush::Forever => "forever",
        }
    }
}

/// One change to how the user keeps a conversation, each its own route on dev-null.sh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refile {
    Pin(bool),
    Stash(bool),
    /// Mutes it for that long, or with none unmutes it.
    Mute(Option<Hush>),
}

/// A conversation as the list of them shows it.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub party: Party,
    pub last: Option<Message>,
    pub last_at: Option<DateTime<Utc>>,
    /// Messages from the others the user hasn't read.
    pub unread: u32,
    pub filed: Filed,
}

#[derive(Debug, Clone)]
pub struct ConversationPage {
    pub conversations: Vec<Conversation>,
    /// The conversation the next page continues before.
    pub next: Option<String>,
    /// How many requests wait on the user, whichever folder the page is from.
    pub requests: u32,
}

impl ConversationPage {
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<Row>,
            has_more: bool,
            requests_count: u32,
        }

        #[derive(Deserialize)]
        struct Row {
            id: String,
            name: Option<String>,
            partner: Option<User>,
            members: Option<Vec<Member>>,
            last_message: Option<Message>,
            last_message_at: Option<DateTime<Utc>>,
            ahead: u32,
            pinned_at: Option<String>,
            /// Null once the mute is over.
            muted_until_at: Option<String>,
            is_archived: bool,
        }

        let wire: Wire = serde_json::from_str(json)?;
        let next = (wire.has_more)
            .then(|| wire.data.last().map(|row| row.id.clone()))
            .flatten();
        let conversations = (wire.data.into_iter())
            .filter_map(|row| {
                let party = match (row.partner, row.members) {
                    (Some(partner), _) => Party::Person(partner),
                    (None, Some(members)) => Party::Org(Org {
                        id: OrgId(row.id),
                        name: row.name,
                        members: members.into_iter().map(|member| member.user).collect(),
                    }),
                    (None, None) => return None,
                };
                Some(Conversation {
                    party,
                    last: row.last_message,
                    last_at: row.last_message_at,
                    unread: row.ahead,
                    filed: Filed {
                        pinned: row.pinned_at.is_some(),
                        stashed: row.is_archived,
                        muted: row.muted_until_at.is_some(),
                    },
                })
            })
            .collect();
        Ok(ConversationPage {
            conversations,
            next,
            requests: wire.requests_count,
        })
    }
}

/// Someone a search found to start a conversation with, and dev-null.sh's reason they can't be
/// messaged, if there is one.
#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub user: User,
    pub reason: Option<String>,
}

impl Candidate {
    pub fn parse_list(json: &str) -> serde_json::Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct Wire {
            candidates: Vec<Candidate>,
        }

        Ok(serde_json::from_str::<Wire>(json)?.candidates)
    }
}

/// Whether the user can push to the partner, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Open,
    /// The partner's request waits here to be merged.
    Request,
    /// The user's request waits on the partner.
    Waiting,
    Restricted,
    Closed,
}

impl Access {
    /// Why the user can't push to `handle`, in dev-null.sh's words.
    pub fn refusal(self, handle: &str) -> Option<String> {
        match self {
            Access::Open => None,
            Access::Request => Some(format!(
                "merge @{handle}'s request before replying, with a in gm's requests."
            )),
            Access::Waiting => Some(format!("@{handle} hasn't merged your request yet.")),
            Access::Restricted => Some(format!(
                "@{handle} doesn't follow you, so you can't push to them yet."
            )),
            Access::Closed => Some(format!("@{handle} isn't taking remotes.")),
        }
    }
}

/// A page of a conversation, oldest first.
#[derive(Debug, Clone)]
pub struct MessagePage {
    pub party: Party,
    pub messages: Vec<Message>,
    pub has_older: bool,
    pub read_through: MessageId,
    pub access: Access,
}

impl MessagePage {
    /// A page of the thread with a partner, or of an org's, which names the org and its members
    /// instead.
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: Data,
        }

        #[derive(Deserialize)]
        struct Data {
            partner: Option<User>,
            org: Option<OrgName>,
            members: Option<Vec<Member>>,
            messages: Vec<Message>,
            has_older: bool,
            /// Null until the user has read anything here.
            read_through_id: Option<MessageId>,
            access: Access,
        }

        #[derive(Deserialize)]
        struct OrgName {
            id: OrgId,
            name: Option<String>,
        }

        let data = serde_json::from_str::<Wire>(json)?.data;
        let party = match (data.partner, data.org) {
            (Some(partner), _) => Party::Person(partner),
            (None, Some(org)) => Party::Org(Org {
                id: org.id,
                name: org.name,
                members: (data.members.into_iter().flatten())
                    .map(|member| member.user)
                    .collect(),
            }),
            (None, None) => return Err(de::Error::custom("a thread with no partner and no org")),
        };
        Ok(MessagePage {
            party,
            messages: data.messages,
            has_older: data.has_older,
            read_through: data.read_through_id.unwrap_or(MessageId(0)),
            access: data.access,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct ScheduledId(pub String);

/// A commit queued to go out later, which only its author sees until then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduled {
    pub id: ScheduledId,
    pub at: DateTime<Utc>,
    pub body: String,
    /// The organization it goes out as, when not as the user.
    pub author: Option<String>,
    pub poll: bool,
    pub images: usize,
}

/// A scheduled commit as the API sends it. Once it went out, its body is gone and only the status
/// says what became of it.
#[derive(Deserialize)]
struct ScheduledWire {
    id: ScheduledId,
    author: Option<Handle>,
    body: Option<String>,
    poll_options: Option<Vec<String>>,
    image_count: usize,
    scheduled_at: DateTime<Utc>,
    status: String,
}

#[derive(Deserialize)]
struct Handle {
    handle: String,
}

impl ScheduledWire {
    /// The commit while it still waits to go out.
    fn waiting(self) -> Option<Scheduled> {
        let body = self.body.filter(|_| self.status == "scheduled")?;
        Some(Scheduled {
            id: self.id,
            at: self.scheduled_at,
            body,
            author: self.author.map(|author| author.handle),
            poll: self.poll_options.is_some(),
            images: self.image_count,
        })
    }
}

impl Scheduled {
    /// The one the server just queued.
    pub fn parse(json: &str) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Wire {
            data: ScheduledWire,
        }

        let wire: Wire = serde_json::from_str(json)?;
        wire.data
            .waiting()
            .ok_or_else(|| de::Error::custom("a commit just scheduled has gone out"))
    }

    /// The ones still waiting, soonest first. The queue lists those already sent after them,
    /// and at most 10 wait, so the first page holds them all.
    pub fn parse_queue(json: &str) -> serde_json::Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct Wire {
            data: Vec<ScheduledWire>,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(wire
            .data
            .into_iter()
            .filter_map(ScheduledWire::waiting)
            .collect())
    }

    /// Whether a scheduled commit sent early waits for review, from the queue entry the server answers with.
    pub fn parse_pending(json: &str) -> serde_json::Result<bool> {
        #[derive(Deserialize)]
        struct Wire {
            data: ScheduledWire,
        }

        let wire: Wire = serde_json::from_str(json)?;
        Ok(wire.data.status == "pending")
    }
}
