use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

/// The web's shortcode list, which the server also uses to turn `:name:` into its emoji.
const SHORTCODES: &str = include_str!("emoji.json");
const MIN_QUERY: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emoji {
    pub name: &'static str,
    pub glyph: &'static str,
}

fn all() -> &'static [Emoji] {
    static ALL: OnceLock<Vec<Emoji>> = OnceLock::new();
    ALL.get_or_init(|| {
        let map: BTreeMap<&'static str, &'static str> =
            serde_json::from_str(SHORTCODES).expect("the vendored shortcodes parse");
        map.into_iter()
            .map(|(name, glyph)| Emoji { name, glyph })
            .collect()
    })
}

/// Every emoji whose name holds the query, once each under its best name: names starting with
/// the query first, then names with a later word starting with it, then the rest, shortest first.
pub fn search(query: &str) -> Vec<Emoji> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, Emoji)> = all()
        .iter()
        .filter(|emoji| emoji.name.contains(&query))
        .map(|emoji| (rank(emoji.name, &query), *emoji))
        .collect();
    found.sort_by_key(|(rank, emoji)| (*rank, emoji.name.len(), emoji.name));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .map(|(_, emoji)| emoji)
        .filter(|emoji| seen.insert(emoji.glyph))
        .collect()
}

fn rank(name: &str, query: &str) -> u8 {
    if name.starts_with(query) {
        0
    } else if name.split(['_', '-']).any(|word| word.starts_with(query)) {
        1
    } else {
        2
    }
}

/// Where a `:name` being typed starts, as a byte offset into `before`, the text up to the cursor.
/// It needs two characters after the colon, so a time like 10:30 stays quiet.
pub fn trigger(before: &str) -> Option<usize> {
    let query = before
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'))
        .count();
    let colon = before.len().checked_sub(query + 1)?;
    let mut rest = before[..=colon].chars().rev();
    let opens = rest.next() == Some(':')
        && rest
            .next()
            .is_none_or(|ch| !(ch.is_alphanumeric() || matches!(ch, '_' | ':')));
    (opens && query >= MIN_QUERY).then_some(colon)
}
