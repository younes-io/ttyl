use std::fs;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::api::Me;
use crate::model::{Lane, Language, PALETTE_LEN, Settings, Zone, parse_hex};

/// The palette's colors by the names the web gives them, in lane order.
pub const LANES: [&str; PALETTE_LEN as usize] =
    ["orange", "green", "violet", "teal", "rose", "ochre"];

/// A setting by the name `ttyl config` and the settings screen give it, `section.name` as git
/// config names its keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Pasteis,
    Color,
    Timezone,
    LocalTime,
    Language,
    AutoTranslate,
    Remotes,
    RemotesFrom,
    PullReceipts,
}

impl Key {
    pub const ALL: [Key; 9] = [
        Key::Pasteis,
        Key::Color,
        Key::Timezone,
        Key::LocalTime,
        Key::Language,
        Key::AutoTranslate,
        Key::Remotes,
        Key::RemotesFrom,
        Key::PullReceipts,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Key::Pasteis => "account.pasteis",
            Key::Color => "account.color",
            Key::Timezone => "region.timezone",
            Key::LocalTime => "region.local-time",
            Key::Language => "region.language",
            Key::AutoTranslate => "region.auto-translate",
            Key::Remotes => "remotes.enabled",
            Key::RemotesFrom => "remotes.from",
            Key::PullReceipts => "remotes.pull-receipts",
        }
    }

    /// The key by its name, in any case, as git config reads keys.
    pub fn named(name: &str) -> Option<Key> {
        Key::ALL
            .into_iter()
            .find(|key| key.name().eq_ignore_ascii_case(name))
    }

    pub fn section(self) -> &'static str {
        self.name()
            .split_once('.')
            .map_or("", |(section, _)| section)
    }

    /// The name within its section.
    pub fn short(self) -> &'static str {
        self.name().split_once('.').map_or("", |(_, short)| short)
    }

    /// What it changes, as the settings screen says beside it.
    pub fn about(self) -> &'static str {
        match self {
            Key::Pasteis => "give and count pastéis",
            Key::Color => "your lane on every card",
            Key::Timezone => "shown where people hover your commits",
            Key::LocalTime => "your time on your profile",
            Key::Language => "what t translates into",
            Key::AutoTranslate => "translated commits show first",
            Key::Remotes => "direct messages",
            Key::RemotesFrom => "who can message you",
            Key::PullReceipts => "show people when you've read theirs",
        }
    }

    /// The values it takes, as `ttyl --help` lists them.
    pub fn takes(self) -> &'static str {
        match self {
            Key::Color => "auto, orange, green, violet, teal, rose, ochre or #rrggbb",
            Key::Timezone => "an IANA name such as Europe/Lisbon, empty to unset",
            Key::Language => "a code such as fr, empty to unset",
            Key::RemotesFrom => "following or everyone",
            Key::Pasteis
            | Key::LocalTime
            | Key::AutoTranslate
            | Key::Remotes
            | Key::PullReceipts => "true or false",
        }
    }

    /// Whether it takes typing on the bottom row, rather than flipping to its next value.
    pub fn typed(self) -> bool {
        matches!(self, Key::Timezone | Key::Language)
    }

    fn field(self) -> Field {
        match self {
            Key::Pasteis => Field::Pasteis,
            Key::Color => Field::Color,
            Key::Timezone | Key::LocalTime => Field::Timezone,
            Key::Language => Field::Language,
            Key::AutoTranslate => Field::AutoTranslate,
            Key::Remotes => Field::Remotes,
            Key::RemotesFrom | Key::PullReceipts => Field::Prefs,
        }
    }

    /// Whether its value comes from `me/settings` rather than `me`.
    fn regional(self) -> bool {
        self.field().regional()
    }
}

/// What one request sets. Some keys share one: `region.local-time` goes with the timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Pasteis,
    Color,
    Timezone,
    Language,
    AutoTranslate,
    Remotes,
    Prefs,
}

impl Field {
    fn regional(self) -> bool {
        matches!(
            self,
            Field::Timezone | Field::Language | Field::AutoTranslate
        )
    }

    /// Puts this setting's value in `from` into `to`, for the settings `me` carries.
    pub fn carry(self, from: &Me, to: &mut Me) {
        match self {
            Field::Pasteis => to.pasteis_mode = from.pasteis_mode,
            Field::Color => to.preferred_lane = from.preferred_lane,
            Field::Remotes => to.remotes = from.remotes,
            Field::Prefs => {
                if let (Some(from), Some(to)) = (from.remotes, &mut to.remotes) {
                    to.from = from.from;
                    to.pull_receipts = from.pull_receipts;
                }
            }
            Field::Timezone | Field::Language | Field::AutoTranslate => {}
        }
    }

    /// Puts this setting's value in `from` into `to`, for the settings `me/settings` carries.
    pub fn carry_regional(self, from: &Settings, to: &mut Settings) {
        match self {
            Field::Timezone => to.timezone.clone_from(&from.timezone),
            Field::Language => to
                .translation_language
                .clone_from(&from.translation_language),
            Field::AutoTranslate => to.automatic_translation = from.automatic_translation,
            Field::Pasteis | Field::Color | Field::Remotes | Field::Prefs => {}
        }
    }
}

/// Who can push to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    Following,
    Everyone,
}

impl Audience {
    fn name(self) -> &'static str {
        match self {
            Audience::Following => "following",
            Audience::Everyone => "everyone",
        }
    }
}

/// The settings menu of the web's Remotes page, which `me` carries while Remotes are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Remotes {
    pub from: Audience,
    pub pull_receipts: bool,
    /// Whether dev-null.sh lets anyone at all push to people who choose so.
    #[serde(default)]
    pub everyone: bool,
}

/// One setting request, with what it sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Pasteis(bool),
    /// A lane from the palette or a color of the user's own, or `None` for the automatic one.
    Color(Option<Lane>),
    Timezone(Option<Zone>),
    Language(Option<String>),
    AutoTranslate(bool),
    Remotes(bool),
    Prefs {
        from: Audience,
        pull_receipts: bool,
    },
}

impl Change {
    pub fn field(&self) -> Field {
        match self {
            Change::Pasteis(_) => Field::Pasteis,
            Change::Color(_) => Field::Color,
            Change::Timezone(_) => Field::Timezone,
            Change::Language(_) => Field::Language,
            Change::AutoTranslate(_) => Field::AutoTranslate,
            Change::Remotes(_) => Field::Remotes,
            Change::Prefs { .. } => Field::Prefs,
        }
    }

    /// Where the change is put, and what it sends.
    pub fn request(&self) -> (&'static str, serde_json::Value) {
        use serde_json::json;
        match self {
            Change::Pasteis(on) => ("/api/v1/me/pasteis-mode", json!({ "enabled": on })),
            // Only an organization goes without a lane, which is what `auto` asks for.
            Change::Color(None | Some(Lane::Site)) => ("/api/v1/me/lane", json!({ "lane": null })),
            Change::Color(Some(Lane::Palette(index))) => {
                ("/api/v1/me/lane", json!({ "lane": index }))
            }
            Change::Color(Some(Lane::Custom { r, g, b })) => (
                "/api/v1/me/lane",
                json!({ "lane": null, "color": format!("#{r:02x}{g:02x}{b:02x}") }),
            ),
            Change::Timezone(zone) => (
                "/api/v1/me/timezone",
                json!({
                    "timezone": zone.as_ref().map(|zone| &zone.name),
                    "show_local_time": zone.as_ref().is_some_and(|zone| zone.local_time),
                }),
            ),
            Change::Language(code) => (
                "/api/v1/me/translation-language",
                json!({ "translation_language": code }),
            ),
            Change::AutoTranslate(on) => {
                ("/api/v1/me/automatic-translation", json!({ "enabled": on }))
            }
            Change::Remotes(on) => ("/api/v1/me/remotes", json!({ "enabled": on })),
            Change::Prefs {
                from,
                pull_receipts,
            } => (
                "/api/v1/me/remote-preferences",
                json!({ "from": from.name(), "pull_receipts": pull_receipts }),
            ),
        }
    }

    /// Takes a confirmed change into what `me/settings` said, as its answer doesn't repeat it.
    pub fn confirm(&self, settings: &mut Settings) {
        match self {
            Change::Timezone(zone) => settings.timezone = zone.clone(),
            Change::Language(code) => settings.translation_language = code.clone(),
            Change::AutoTranslate(on) => settings.automatic_translation = *on,
            Change::Pasteis(_) | Change::Color(_) | Change::Remotes(_) | Change::Prefs { .. } => {}
        }
    }
}

/// Changes the user made that dev-null.sh hasn't confirmed, the latest for each request.
#[derive(Debug, Default)]
pub struct Wants(Vec<Change>);

impl Wants {
    pub const NONE: &Wants = &Wants(Vec::new());

    pub fn want(&mut self, change: Change) {
        let field = change.field();
        self.0.retain(|want| want.field() != field);
        self.0.push(change);
    }

    fn get(&self, field: Field) -> Option<&Change> {
        self.0.iter().find(|want| want.field() == field)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Change> {
        self.0.iter()
    }

    /// Drops the wants dev-null.sh already holds.
    pub fn retain(&mut self, keep: impl FnMut(&Change) -> bool) {
        self.0.retain(keep);
    }
}

/// A value the bottom row offers while a key is typed, and how it reads there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub value: String,
    pub label: String,
}

/// The account's settings as the user wants them: what dev-null.sh said, with their unconfirmed
/// changes over it. `settings` is `None` until `me/settings` answers.
#[derive(Clone, Copy)]
pub struct Config<'a> {
    me: &'a Me,
    settings: Option<&'a Settings>,
    wants: &'a Wants,
}

impl<'a> Config<'a> {
    pub fn new(me: &'a Me, settings: Option<&'a Settings>, wants: &'a Wants) -> Self {
        Config {
            me,
            settings,
            wants,
        }
    }

    /// What dev-null.sh holds, without the changes still on their way.
    pub fn server(self) -> Config<'a> {
        Config {
            wants: Wants::NONE,
            ..self
        }
    }

    fn want(&self, field: Field) -> Option<&'a Change> {
        self.wants.get(field)
    }

    /// Whether `key` has a value to show: the regional ones wait for `me/settings`, and the
    /// remotes ones for Remotes to be on.
    pub fn has(&self, key: Key) -> bool {
        match key {
            _ if key.regional() => self.settings.is_some(),
            Key::RemotesFrom | Key::PullReceipts => self.prefs().is_some(),
            _ => true,
        }
    }

    pub fn pasteis(&self) -> bool {
        match self.want(Field::Pasteis) {
            Some(Change::Pasteis(on)) => *on,
            _ => self.me.pasteis_mode,
        }
    }

    pub fn color(&self) -> Option<Lane> {
        match self.want(Field::Color) {
            Some(Change::Color(lane)) => *lane,
            _ => self.me.preferred_lane,
        }
    }

    /// The lane dev-null.sh picks for the user when they pick none.
    pub fn automatic_lane(&self) -> u8 {
        self.me.automatic_lane
    }

    pub fn zone(&self) -> Option<&'a Zone> {
        match self.want(Field::Timezone) {
            Some(Change::Timezone(zone)) => zone.as_ref(),
            _ => self.settings?.timezone.as_ref(),
        }
    }

    pub fn language(&self) -> Option<&'a str> {
        match self.want(Field::Language) {
            Some(Change::Language(code)) => code.as_deref(),
            _ => self.settings?.translation_language.as_deref(),
        }
    }

    pub fn languages(&self) -> &'a [Language] {
        self.settings.map_or(&[], |settings| &settings.languages)
    }

    fn auto_translate(&self) -> bool {
        match self.want(Field::AutoTranslate) {
            Some(Change::AutoTranslate(on)) => *on,
            _ => self
                .settings
                .is_some_and(|settings| settings.automatic_translation),
        }
    }

    pub fn remotes(&self) -> bool {
        match self.want(Field::Remotes) {
            Some(Change::Remotes(on)) => *on,
            _ => self.me.remotes.is_some(),
        }
    }

    /// Who can push and whether pulls are shared, known once dev-null.sh says Remotes are on.
    fn prefs(&self) -> Option<(Audience, bool)> {
        if !self.remotes() {
            return None;
        }
        let server = self.me.remotes?;
        Some(match self.want(Field::Prefs) {
            Some(Change::Prefs {
                from,
                pull_receipts,
            }) => (*from, *pull_receipts),
            _ => (server.from, server.pull_receipts),
        })
    }

    fn everyone_offered(&self) -> bool {
        self.me.remotes.is_some_and(|remotes| remotes.everyone)
    }

    /// Whether `key` shows a change dev-null.sh hasn't confirmed yet.
    pub fn saving(&self, key: Key) -> bool {
        self.want(key.field()).is_some() && self.get(key) != self.server().get(key)
    }

    /// The request that would make dev-null.sh hold what `field` shows here.
    pub fn change(&self, field: Field) -> Option<Change> {
        Some(match field {
            Field::Pasteis => Change::Pasteis(self.pasteis()),
            Field::Color => Change::Color(self.color()),
            Field::Language => {
                self.settings?;
                Change::Language(self.language().map(str::to_owned))
            }
            Field::AutoTranslate => {
                self.settings?;
                Change::AutoTranslate(self.auto_translate())
            }
            Field::Timezone => {
                self.settings?;
                Change::Timezone(self.zone().cloned())
            }
            Field::Remotes => Change::Remotes(self.remotes()),
            Field::Prefs => {
                let (from, pull_receipts) = self.prefs()?;
                Change::Prefs {
                    from,
                    pull_receipts,
                }
            }
        })
    }

    /// The value `ttyl config` prints, or `None` while it's unset or doesn't apply, as git config
    /// leaves out a key that isn't set.
    pub fn get(&self, key: Key) -> Option<String> {
        if !self.has(key) {
            return None;
        }
        let flag = |on: bool| Some(on.to_string());
        match key {
            Key::Pasteis => flag(self.pasteis()),
            Key::Color => Some(match self.color() {
                None => "auto".to_owned(),
                Some(lane) => lane_name(lane),
            }),
            Key::Timezone => self.zone().map(|zone| zone.name.clone()),
            Key::LocalTime => flag(self.zone().is_some_and(|zone| zone.local_time)),
            Key::Language => self.language().map(str::to_owned),
            Key::AutoTranslate => flag(self.auto_translate()),
            Key::Remotes => flag(self.remotes()),
            Key::RemotesFrom => self.prefs().map(|(from, _)| from.name().to_owned()),
            Key::PullReceipts => self.prefs().and_then(|(_, pull)| flag(pull)),
        }
    }

    /// The change that sets `key` to `value`, read the way `ttyl config <key> <value>` takes it.
    /// An empty value unsets a key that can be unset. The error is the line to print.
    pub fn set(&self, key: Key, value: &str) -> Result<Change, String> {
        let name = key.name();
        let flag = || {
            boolean(value)
                .ok_or_else(|| format!("fatal: bad boolean config value '{value}' for '{name}'"))
        };
        if key.regional() && self.settings.is_none() {
            return Err(format!("fatal: {name} hasn't loaded yet"));
        }
        if matches!(key, Key::RemotesFrom | Key::PullReceipts) && self.prefs().is_none() {
            return Err(
                "fatal: remotes are off. turn them on first with remotes.enabled true".to_owned(),
            );
        }
        match key {
            Key::Pasteis => flag().map(Change::Pasteis),
            Key::Color => color(value).map(Change::Color).ok_or_else(|| {
                format!(
                    "fatal: bad color '{value}' for '{name}'. use auto, {}, or #rrggbb",
                    LANES.join(", ")
                )
            }),
            Key::Timezone => Ok(Change::Timezone(match value.trim() {
                "" => None,
                typed => Some(Zone {
                    name: (ZONES.iter())
                        .find(|zone| zone.eq_ignore_ascii_case(typed))
                        .map_or_else(|| typed.to_owned(), Clone::clone),
                    local_time: self.zone().is_some_and(|zone| zone.local_time),
                }),
            })),
            Key::LocalTime => {
                let on = flag()?;
                match self.zone() {
                    Some(zone) => Ok(Change::Timezone(Some(Zone {
                        name: zone.name.clone(),
                        local_time: on,
                    }))),
                    None if !on => Ok(Change::Timezone(None)),
                    None => Err(format!(
                        "fatal: {name} needs a timezone. set region.timezone first"
                    )),
                }
            }
            Key::Language => match value.trim() {
                "" => Ok(Change::Language(None)),
                typed => (self.languages().iter())
                    .find(|language| {
                        [&language.code, &language.name, &language.native_name]
                            .iter()
                            .any(|each| each.eq_ignore_ascii_case(typed))
                    })
                    .map(|language| Change::Language(Some(language.code.clone())))
                    .ok_or_else(|| format!("fatal: dev-null.sh doesn't translate into '{typed}'")),
            },
            Key::AutoTranslate => flag().map(Change::AutoTranslate),
            Key::Remotes => flag().map(Change::Remotes),
            Key::RemotesFrom => {
                let from = match value.to_ascii_lowercase().as_str() {
                    "following" => Audience::Following,
                    "everyone" => Audience::Everyone,
                    _ => {
                        return Err(format!(
                            "fatal: bad value '{value}' for '{name}'. use following or everyone"
                        ));
                    }
                };
                self.prefs_with(Some(from), None)
            }
            Key::PullReceipts => self.prefs_with(None, Some(flag()?)),
        }
    }

    fn prefs_with(&self, from: Option<Audience>, pull: Option<bool>) -> Result<Change, String> {
        let (was_from, was_pull) = self.prefs().ok_or("fatal: remotes are off")?;
        let from = from.unwrap_or(was_from);
        if from == Audience::Everyone && was_from != from && !self.everyone_offered() {
            return Err(
                "fatal: dev-null.sh only lets people you follow push to you for now".to_owned(),
            );
        }
        Ok(Change::Prefs {
            from,
            pull_receipts: pull.unwrap_or(was_pull),
        })
    }

    /// The change enter makes on the settings screen: a switch flips, a choice moves to the next.
    pub fn next(&self, key: Key) -> Result<Change, String> {
        let flip = |value: Option<String>| (value.as_deref() != Some("true")).to_string();
        match key {
            Key::Color => Ok(Change::Color(match self.color() {
                None => Some(Lane::Palette(0)),
                Some(Lane::Palette(index)) if index + 1 < PALETTE_LEN => {
                    Some(Lane::Palette(index + 1))
                }
                Some(_) => None,
            })),
            Key::RemotesFrom => match self.prefs() {
                Some((Audience::Following, _)) => self.set(key, "everyone"),
                _ => self.set(key, "following"),
            },
            Key::Timezone | Key::Language => {
                unreachable!("{} is typed, not flipped", key.name())
            }
            Key::Pasteis
            | Key::LocalTime
            | Key::AutoTranslate
            | Key::Remotes
            | Key::PullReceipts => self.set(key, &flip(self.get(key))),
        }
    }

    /// What the bottom row offers for `typed` while `key` is typed, the closest first.
    pub fn candidates(&self, key: Key, typed: &str) -> Vec<Candidate> {
        let typed = typed.trim().to_lowercase().replace('_', " ");
        let rank = |words: &[&str]| -> Option<u8> {
            let words: Vec<String> = (words.iter())
                .map(|word| word.to_lowercase().replace('_', " "))
                .collect();
            if words.contains(&typed) {
                Some(0)
            } else if (words.iter()).any(|word| {
                word.starts_with(&typed) || word.split('/').any(|part| part.starts_with(&typed))
            }) {
                Some(1)
            } else if words.iter().any(|word| word.contains(&typed)) {
                Some(2)
            } else {
                None
            }
        };
        let mut ranked: Vec<(u8, Candidate)> = match key {
            Key::Timezone => (ZONES.iter())
                .filter_map(|zone| {
                    let candidate = Candidate {
                        value: zone.clone(),
                        label: zone.clone(),
                    };
                    Some((rank(&[zone])?, candidate))
                })
                .collect(),
            Key::Language => (self.languages().iter())
                .filter_map(|language| {
                    let words = [
                        language.code.as_str(),
                        &language.name,
                        &language.native_name,
                    ];
                    let label = if language.native_name.eq_ignore_ascii_case(&language.name) {
                        format!("{}  {}", language.code, language.name)
                    } else {
                        format!(
                            "{}  {} · {}",
                            language.code, language.name, language.native_name
                        )
                    };
                    let candidate = Candidate {
                        value: language.code.clone(),
                        label,
                    };
                    Some((rank(&words)?, candidate))
                })
                .collect(),
            _ => Vec::new(),
        };
        ranked.sort_by_key(|(rank, _)| *rank);
        ranked.into_iter().map(|(_, candidate)| candidate).collect()
    }
}

/// A lane by its palette name, or a color of the user's own as `#rrggbb`.
pub fn lane_name(lane: Lane) -> String {
    match lane {
        Lane::Palette(index) => LANES
            .get(usize::from(index))
            .map_or_else(|| index.to_string(), |name| (*name).to_owned()),
        Lane::Custom { r, g, b } => format!("#{r:02x}{g:02x}{b:02x}"),
        Lane::Site => "auto".to_owned(),
    }
}

/// `auto`, a palette name or `#rrggbb`, in any case.
fn color(value: &str) -> Option<Option<Lane>> {
    let value = value.trim().to_ascii_lowercase();
    if value == "auto" {
        return Some(None);
    }
    if let Some(index) = LANES.iter().position(|name| *name == value) {
        return Some(Some(Lane::Palette(u8::try_from(index).ok()?)));
    }
    let (r, g, b) = parse_hex(&value)?;
    Some(Some(Lane::Custom { r, g, b }))
}

/// A boolean the way git config reads one.
fn boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The timezone names this machine knows, which are the ones dev-null.sh accepts, for the bottom
/// row to complete. Empty where the system keeps no `zone.tab`; the server still checks the name.
static ZONES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let table = fs::read_to_string("/usr/share/zoneinfo/zone.tab").unwrap_or_default();
    let mut zones: Vec<String> = (table.lines())
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(2))
        .map(str::to_owned)
        .collect();
    if !zones.is_empty() {
        zones.push("UTC".to_owned());
    }
    zones.sort_unstable();
    zones
});
