//! The server's rules for a commit body, ported from dev-null.sh so the composer counts and folds as it does.

use unicode_segmentation::UnicodeSegmentation;

pub const LIMIT: usize = 4000;
const RAW_LIMIT: usize = 40000;
const URL_MAX: usize = 2048;
const URL_COUNTS_AS: usize = 23;
const PREVIEW_LENGTH: usize = 280;
const PREVIEW_LINES: usize = 8;

const TOP_LEVEL_DOMAINS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "info", "biz", "io", "dev", "app", "co", "ai", "me",
    "xyz", "tech", "gg", "tv", "fm", "ly", "to", "is", "blog", "cloud", "site", "online", "design",
    "studio", "tools", "software", "codes", "wiki", "news", "website", "club", "ninja", "rocks",
    "academy", "guide", "social", "digital", "agency", "fyi", "lol", "live", "uk", "de", "fr",
    "nl", "es", "it", "pt", "br", "ca", "au", "jp", "ch", "se", "no", "dk", "fi", "be", "at", "ie",
    "nz", "us", "eu", "cz", "sk", "hu", "ro", "bg", "gr", "hr", "si", "ee", "lv", "lt", "ru", "ua",
    "tr", "il", "za", "mx", "ar", "kr", "cn", "tw", "hk", "sg", "vn", "th",
];

/// The body as the server stores it: trimmed, with line breaks as `\n` and long runs of blank prose lines cut to one.
pub fn normalize(text: &str) -> String {
    let text = text.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0B']);
    if !text.contains('\r') && text.matches('\n').count() < 3 {
        return text.to_owned();
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    split(&text)
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            if index % 2 == 0 {
                collapse_blank_lines(part)
            } else {
                part.to_owned()
            }
        })
        .collect()
}

/// Characters left under the limit, below zero when over it. A prose link counts as at most 23.
pub fn left(text: &str) -> isize {
    let used = isize::try_from(count(&normalize(text))).unwrap_or(isize::MAX);
    isize::try_from(LIMIT).unwrap_or(isize::MAX) - used
}

/// Why the server would turn this body down, or None when it would take it.
pub fn refusal(text: &str) -> Option<String> {
    let body = normalize(text);
    if body.is_empty() {
        return Some("nothing to commit. the body is empty.".to_owned());
    }
    if body.chars().count() > RAW_LIMIT {
        return Some(format!("over {RAW_LIMIT} characters. nothing sent."));
    }
    if links(&body).any(|link| link.url_len > URL_MAX) {
        return Some("a link is too long. nothing sent.".to_owned());
    }
    let over = count(&body).saturating_sub(LIMIT);
    (over > 0).then(|| format!("{over} over the limit. nothing sent."))
}

/// The character the feed cuts a long body at, or None when it shows whole.
pub fn fold(text: &str) -> Option<usize> {
    let lines: Vec<&str> = text.splitn(PREVIEW_LINES + 1, '\n').collect();
    if lines.len() > PREVIEW_LINES {
        let shown = lines[..PREVIEW_LINES].join("\n").chars().count();
        return Some(shown.min(PREVIEW_LENGTH));
    }
    (text.chars().count() > PREVIEW_LENGTH).then_some(PREVIEW_LENGTH)
}

fn count(body: &str) -> usize {
    let shortened: usize = links(body)
        .filter(|link| !link.bare_scheme && link.url_len <= URL_MAX)
        .map(|link| link.len.saturating_sub(URL_COUNTS_AS))
        .sum();
    body.chars().count() - shortened
}

/// `(?:[ \t]*\n){3,}` replaced by two line breaks.
fn collapse_blank_lines(prose: &str) -> String {
    let chars: Vec<char> = prose.chars().collect();
    let mut out = String::with_capacity(prose.len());
    let mut at = 0;
    while at < chars.len() {
        let mut end = at;
        let mut breaks = 0;
        loop {
            let mut next = end;
            while next < chars.len() && matches!(chars[next], ' ' | '\t') {
                next += 1;
            }
            if chars.get(next) != Some(&'\n') {
                break;
            }
            end = next + 1;
            breaks += 1;
        }
        if breaks >= 3 {
            out.push_str("\n\n");
            at = end;
        } else {
            out.push(chars[at]);
            at += 1;
        }
    }
    out
}

/// Prose and code alternating, prose at the even indexes, as the server's CodeSpans splits them.
fn split(text: &str) -> Vec<&str> {
    if !text.contains('`') {
        return vec![text];
    }
    let mut parts = Vec::new();
    let mut prose = 0;
    let mut at = 0;
    while at < text.len() {
        if let Some(end) = fenced(text, at).or_else(|| inline(text, at)) {
            parts.push(&text[prose..at]);
            parts.push(&text[at..end]);
            prose = end;
            at = end;
        } else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    parts.push(&text[prose..]);
    parts
}

fn backticks(text: &str, at: usize) -> usize {
    text[at..].bytes().take_while(|&byte| byte == b'`').count()
}

fn line_break(text: &str, at: usize) -> Option<usize> {
    let rest = &text.as_bytes()[at..];
    match rest {
        [b'\r', b'\n', ..] => Some(at + 2),
        [b'\n', ..] => Some(at + 1),
        _ => None,
    }
}

/// A closing fence at `at`: three or more backticks, spaces or tabs, then the end of the line.
fn closing_fence(text: &str, at: usize) -> Option<usize> {
    let ticks = backticks(text, at);
    if ticks < 3 {
        return None;
    }
    let end = at
        + ticks
        + text[at + ticks..]
            .bytes()
            .take_while(|&byte| byte == b' ' || byte == b'\t')
            .count();
    matches!(text.as_bytes().get(end), None | Some(b'\r' | b'\n')).then_some(end)
}

fn fenced(text: &str, at: usize) -> Option<usize> {
    if at > 0 && text.as_bytes()[at - 1] != b'\n' {
        return None;
    }
    let ticks = backticks(text, at);
    if ticks < 3 {
        return None;
    }
    let info_end = text[at + ticks..]
        .find(['`', '\r', '\n'])
        .map_or(text.len(), |found| at + ticks + found);
    let body = line_break(text, info_end)?;
    let inner = text[body..]
        .match_indices('\n')
        .map(|(found, _)| body + found + 1)
        .find_map(|after| closing_fence(text, after));
    inner.or_else(|| closing_fence(text, body))
}

fn inline(text: &str, at: usize) -> Option<usize> {
    let rest = text[at..].strip_prefix('`')?;
    let close = rest.find(['`', '\r', '\n'])?;
    (close > 0 && rest[close..].starts_with('`')).then_some(at + 1 + close + 1)
}

struct Link {
    len: usize,
    url_len: usize,
    bare_scheme: bool,
}

fn links(body: &str) -> impl Iterator<Item = Link> {
    let prose: Vec<&str> = split(body).into_iter().step_by(2).collect();
    let prose: Vec<char> = prose.join("\n").chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < prose.len() {
        match scheme_url(&prose, at).or_else(|| bare_domain(&prose, at)) {
            Some(end) => {
                found.push(link(&prose[at..end]));
                at = end;
            }
            None => at += 1,
        }
    }
    found.into_iter()
}

fn link(candidate: &[char]) -> Link {
    let kept = without_trailing_punctuation(candidate);
    let has_scheme = starts_with_scheme(kept);
    let url_len = kept.len() + if has_scheme { 0 } else { "https://".len() };
    Link {
        len: kept.len(),
        url_len,
        bare_scheme: kept.is_empty() || kept.ends_with(&[':', '/', '/']),
    }
}

fn starts_with_scheme(chars: &[char]) -> bool {
    let lower: String = chars
        .iter()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `[\p{L}\p{N}_]`, the letters, numbers and underscore a bare domain cannot follow.
fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// `[\p{L}\p{N}\p{M}\p{Pc}]`, with a mark taken as a character that joins the one before it.
fn scheme_blocker(ch: char) -> bool {
    const CONNECTORS: &str =
        "_\u{203F}\u{2040}\u{2054}\u{FE33}\u{FE34}\u{FE4D}\u{FE4E}\u{FE4F}\u{FF3F}";
    ch.is_alphanumeric()
        || CONNECTORS.contains(ch)
        || format!("a{ch}").graphemes(true).nth(1).is_none()
}

fn url_char(ch: char) -> bool {
    !ch.is_whitespace() && ch != '<' && ch != '>'
}

fn scheme_url(chars: &[char], at: usize) -> Option<usize> {
    if at > 0 && scheme_blocker(chars[at - 1]) {
        return None;
    }
    let lower: String = chars[at..]
        .iter()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|scheme| lower.starts_with(scheme))?;
    let start = at + scheme.len();
    let rest = chars[start..]
        .iter()
        .take_while(|&&ch| url_char(ch))
        .count();
    (rest > 0).then_some(start + rest)
}

fn label_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-'
}

fn bare_domain(chars: &[char], at: usize) -> Option<usize> {
    if at > 0 && (word_char(chars[at - 1]) || matches!(chars[at - 1], '.' | '/' | '@' | '-')) {
        return None;
    }
    let mut label_ends = Vec::new();
    let mut start = at;
    loop {
        let len = chars[start..]
            .iter()
            .take_while(|&&ch| label_char(ch))
            .count();
        let label = &chars[start..start + len];
        let valid = label.first().is_some_and(char::is_ascii_alphanumeric)
            && label.last().is_some_and(char::is_ascii_alphanumeric);
        if !valid || chars.get(start + len) != Some(&'.') {
            break;
        }
        start += len + 1;
        label_ends.push(start);
    }
    let domain_end = label_ends
        .into_iter()
        .rev()
        .find_map(|tld_start| top_level_domain(chars, tld_start))?;
    if !matches!(chars.get(domain_end), Some('/' | '?' | '#')) {
        return Some(domain_end);
    }
    let path = chars[domain_end + 1..]
        .iter()
        .take_while(|&&ch| url_char(ch))
        .count();
    Some(domain_end + 1 + path)
}

/// The end of an allowed top-level domain at `at` that ends the word, or None.
fn top_level_domain(chars: &[char], at: usize) -> Option<usize> {
    let len = chars[at..].iter().take_while(|&&ch| word_char(ch)).count();
    let tld: String = chars[at..at + len].iter().collect();
    let allowed = TOP_LEVEL_DOMAINS
        .iter()
        .any(|known| tld == *known || tld == known.to_ascii_uppercase());
    let end = at + len;
    let ends_word = match chars.get(end) {
        Some('(' | '-') => false,
        Some('.') => !chars.get(end + 1).is_some_and(char::is_ascii_alphanumeric),
        _ => true,
    };
    (allowed && ends_word).then_some(end)
}

fn without_trailing_punctuation(mut url: &[char]) -> &[char] {
    loop {
        let before = url.len();
        while let [rest @ .., last] = url
            && ".,;:!?'\"…”’".contains(*last)
        {
            url = rest;
        }
        for (open, close) in [('(', ')'), ('[', ']')] {
            let opens = url.iter().filter(|&&ch| ch == open).count();
            let closes = url.iter().filter(|&&ch| ch == close).count();
            if url.last() == Some(&close) && opens < closes {
                url = &url[..url.len() - 1];
            }
        }
        if url.len() == before {
            return url;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_cuts_blank_runs_in_prose_only() {
        assert_eq!(normalize("  hi \n"), "hi");
        assert_eq!(normalize("a\n\n\nb"), "a\n\nb");
        assert_eq!(normalize("a\r\nb"), "a\nb");
        assert_eq!(normalize("a  \n \n\t\n\nb"), "a\n\nb");
        assert_eq!(
            normalize("```\nx\n\n\n\ny\n```\n\n\n\nz"),
            "```\nx\n\n\n\ny\n```\n\nz"
        );
    }

    #[test]
    fn a_prose_link_counts_as_23_and_code_counts_literally() {
        let url = format!("https://example.com/{}", "a".repeat(100));
        assert_eq!(count(&url), 23);
        assert_eq!(count(&format!("see {url}.")), 4 + 23 + 1);
        assert_eq!(count(&format!("`{url}`")), url.chars().count() + 2);
        assert_eq!(count("nativephp.com"), 13);
        let bare = format!("nativephp.com/{}", "b".repeat(50));
        assert_eq!(count(&bare), 23);
        assert_eq!(count(&format!("socket.io.js/{}", "c".repeat(40))), 53);
        assert_eq!(count(&format!("me@x.com/{}", "d".repeat(40))), 49);
        assert_eq!(count(&format!("bug.It/{}", "e".repeat(40))), 47);
        assert_eq!(count("https://"), 8);
        let after_connector = format!("\u{203F}https://x.com/{}", "f".repeat(40));
        assert_eq!(
            count(&after_connector),
            1 + 14 + 40,
            "a scheme can't follow a connector"
        );
        let after_mark = format!("e\u{301}https://x.com/{}", "f".repeat(40));
        assert_eq!(count(&after_mark), 2 + 14 + 40, "nor a mark");
        let ends_in_scheme = format!("a.co/{}https://", "g".repeat(40));
        assert_eq!(
            count(&ends_in_scheme),
            5 + 40 + 8,
            "a link ending in :// counts whole"
        );
    }

    #[test]
    fn trailing_punctuation_and_unpaired_brackets_are_not_part_of_a_link() {
        let path = "p".repeat(30);
        let link = format!("https://x.com/{path}");
        let counted = 23;
        assert_eq!(count(&format!("({link})")), 1 + counted + 1);
        assert_eq!(
            count(&format!("https://x.com/a_(b){path}")),
            counted,
            "a paired bracket stays"
        );
        assert_eq!(count(&format!("{link}…”")), counted + 2);
    }

    #[test]
    fn refusal_says_why_and_counts_the_way_the_server_does() {
        assert!(refusal(" \n\t").unwrap().contains("empty"));
        assert_eq!(refusal(&"a".repeat(LIMIT)), None);
        assert_eq!(
            refusal(&"a".repeat(LIMIT + 5)).unwrap(),
            "5 over the limit. nothing sent."
        );
        let long_link = format!("https://x.com/{}", "a".repeat(URL_MAX));
        assert!(refusal(&long_link).unwrap().contains("link"));
        let links = format!("https://x.com/{} ", "a".repeat(200)).repeat(100);
        assert_eq!(refusal(&links), None, "each link counts as 23");
        assert_eq!(left(&"a".repeat(3900)), 100);
    }

    #[test]
    fn fold_is_at_280_characters_or_the_end_of_the_eighth_line() {
        assert_eq!(fold(&"a".repeat(280)), None);
        assert_eq!(fold(&"a".repeat(281)), Some(280));
        let lines = "line\n".repeat(9);
        assert_eq!(fold(&lines), Some(8 * 5 - 1));
        assert_eq!(fold(&"x\n".repeat(7)), None);
    }

    #[test]
    fn an_unclosed_fence_is_prose() {
        assert_eq!(split("```\nopen"), ["```\nopen"]);
        assert_eq!(split("a `b` c"), ["a ", "`b`", " c"]);
        assert_eq!(split("```\n```"), ["", "```\n```", ""]);
    }
}
