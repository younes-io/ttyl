//! The bare-paste rules of `@sanity-labs/secret-scan`, which the web's `sensitive-text.ts` runs
//! before a commit goes out.

use std::collections::HashMap;
use std::ops::RangeInclusive;

pub fn detected(text: &str) -> bool {
    TOKENS.iter().any(|token| token.found(text))
        || [
            openai,
            slack,
            slack_webhook,
            sendgrid,
            jwt,
            private_key_header,
            connection_string,
            postgres,
            credentials,
            bearer,
            inngest_event,
            sanity,
        ]
        .iter()
        .any(|rule| rule(text))
}

struct Token {
    prefixes: &'static [&'static str],
    body: fn(u8) -> bool,
    len: RangeInclusive<usize>,
    tail: &'static str,
    edge: bool,
    end: End,
    entropy: f64,
}

#[derive(Clone, Copy)]
enum End {
    Free,
    Boundary,
    Separator,
}

const ANY: RangeInclusive<usize> = 0..=usize::MAX;

const TOKENS: &[Token] = &[
    Token {
        prefixes: &["AKIA", "ABIA", "ACCA"],
        body: |byte| byte.is_ascii_uppercase() || byte.is_ascii_digit(),
        len: 16..=16,
        ..Token::BARE
    },
    Token {
        prefixes: &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"],
        body: word,
        len: 36..=255,
        ..Token::BARE
    },
    Token {
        prefixes: &["sk_live_", "rk_live_"],
        len: 20..=247,
        edge: false,
        end: End::Free,
        ..Token::BARE
    },
    Token {
        prefixes: &["gsk_"],
        len: 52..=52,
        ..Token::BARE
    },
    Token {
        prefixes: &["r8_"],
        body: url_safe,
        len: 37..=37,
        ..Token::BARE
    },
    Token {
        prefixes: &["glpat-"],
        body: |byte| url_safe(byte) || byte == b'=',
        len: 20..=22,
        ..Token::BARE
    },
    Token {
        prefixes: &["npm_"],
        len: 36..=36,
        edge: false,
        end: End::Free,
        ..Token::BARE
    },
    Token {
        prefixes: &["lin_api_"],
        len: 40..=40,
        ..Token::BARE
    },
    Token {
        prefixes: &["sbp_"],
        body: |byte| byte.is_ascii_lowercase() || byte.is_ascii_digit(),
        len: 40..=40,
        ..Token::BARE
    },
    Token {
        prefixes: &["PMAK-"],
        body: |byte| byte.is_ascii_alphanumeric() || byte == b'-',
        len: 59..=59,
        ..Token::BARE
    },
    Token {
        prefixes: &["sk_"],
        body: lower_hex,
        len: 48..=48,
        ..Token::BARE
    },
    Token {
        prefixes: &["signkey-prod-", "signkey-test-", "signkey-staging-"],
        body: lower_hex,
        len: 32..=usize::MAX,
        ..Token::BARE
    },
    Token {
        prefixes: &["sk-ant-api03-", "sk-ant-admin01-"],
        body: url_safe,
        len: 93..=93,
        tail: "AA",
        ..Token::BARE
    },
    Token {
        prefixes: &["sk-"],
        body: url_safe,
        len: 40..=usize::MAX,
        end: End::Separator,
        entropy: 4.5,
        ..Token::BARE
    },
];

impl Token {
    const BARE: Token = Token {
        prefixes: &[],
        body: |byte| byte.is_ascii_alphanumeric(),
        len: ANY,
        tail: "",
        edge: true,
        end: End::Boundary,
        entropy: 0.0,
    };

    fn found(&self, text: &str) -> bool {
        let bytes = text.as_bytes();
        self.prefixes.iter().any(|prefix| {
            starts(text, prefix).any(|at| {
                if self.edge && !boundary(bytes, at) {
                    return false;
                }
                let from = at + prefix.len();
                let run = run(bytes, from, self.body);
                let longest = (*self.len.start()..=run.min(*self.len.end())).rev();
                longest
                    .map(|len| from + len)
                    .filter(|&end| text[end..].starts_with(self.tail))
                    .map(|end| end + self.tail.len())
                    .find(|&end| ends(bytes, end, self.end))
                    .is_some_and(|end| real(&text[at..end], self.entropy))
            })
        })
    }
}

fn openai(text: &str) -> bool {
    const MARK: &str = "T3BlbkFJ";
    let bytes = text.as_bytes();
    starts(text, "sk-").any(|at| {
        let from = at + 3;
        let run_end = from + run(bytes, from, url_safe);
        let Some(mark) = text[from..run_end].find(MARK).map(|mark| from + mark) else {
            return false;
        };
        let middle = &text[from..mark];
        let scoped = ["proj-", "svcacct-", "service-"]
            .iter()
            .any(|scope| middle.len() > scope.len() && middle.starts_with(scope));
        let plain = !middle.is_empty() && middle.bytes().all(|byte| byte.is_ascii_alphanumeric());
        let after = mark + MARK.len();
        boundary(bytes, at)
            && (scoped || plain)
            && (after + 1..=run_end)
                .rev()
                .find(|&end| boundary(bytes, end))
                .is_some_and(|end| real(&text[at..end], 0.0))
    })
}

fn slack(text: &str) -> bool {
    let bytes = text.as_bytes();
    ["xoxb-", "xoxp-", "xoxa-", "xoxr-"].iter().any(|prefix| {
        starts(text, prefix).any(|at| {
            let first = at + prefix.len();
            let digits = run(bytes, first, |byte| byte.is_ascii_digit());
            let second = first + digits + 1;
            (10..=13).contains(&digits)
                && bytes.get(first + digits) == Some(&b'-')
                && run(bytes, second, |byte| byte.is_ascii_digit()) >= 10
                && {
                    let end = second
                        + run(bytes, second, |byte| {
                            byte.is_ascii_alphanumeric() || byte == b'-'
                        });
                    real(&text[at..end], 0.0)
                }
        })
    })
}

fn slack_webhook(text: &str) -> bool {
    const HOOK: &str = "https://hooks.slack.com/services/T";
    let bytes = text.as_bytes();
    let upper = |byte: u8| byte.is_ascii_uppercase() || byte.is_ascii_digit();
    starts(text, HOOK).any(|at| {
        let team = at + HOOK.len();
        let team_end = team + run(bytes, team, upper);
        let bot = team_end + 2;
        let bot_end = bot + run(bytes, bot, upper);
        team_end > team
            && text[team_end..].starts_with("/B")
            && bot_end > bot
            && bytes.get(bot_end) == Some(&b'/')
            && run(bytes, bot_end + 1, |byte| byte.is_ascii_alphanumeric()) >= 23
    })
}

fn sendgrid(text: &str) -> bool {
    let bytes = text.as_bytes();
    let part = |byte: u8| word(byte) || byte == b'-';
    starts(text, "SG.").any(|at| {
        let first = at + 3;
        let first_len = run(bytes, first, part);
        let second = first + first_len + 1;
        let second_len = run(bytes, second, part);
        boundary(bytes, at)
            && (20..=24).contains(&first_len)
            && bytes.get(first + first_len) == Some(&b'.')
            && (39..=second_len.min(50))
                .rev()
                .map(|len| second + len)
                .find(|&end| boundary(bytes, end))
                .is_some_and(|end| real(&text[at..end], 0.0))
    })
}

fn jwt(text: &str) -> bool {
    let bytes = text.as_bytes();
    let segment = |at: usize, heads: &[&str]| -> Option<usize> {
        let head = heads.iter().find(|head| text[at..].starts_with(*head))?;
        let len = run(bytes, at, url_safe);
        let pads = run(bytes, at + len, |byte| byte == b'=').min(2);
        let dot = at + len + pads;
        (len >= head.len() + 12 && bytes.get(dot) == Some(&b'.')).then_some(dot + 1)
    };
    let heads = ["eyJ", "ewogIC", "ewoid"];
    heads.iter().any(|head| {
        starts(text, head).any(|at| {
            let signature = boundary(bytes, at)
                .then(|| segment(at, &heads))
                .flatten()
                .and_then(|payload| segment(payload, &["eyJ", "ewo"]));
            signature.is_some_and(|from| {
                let len = run(bytes, from, url_safe);
                (12..=len).rev().any(|len| boundary(bytes, from + len))
            })
        })
    })
}

fn private_key_header(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let class =
        |byte: u8| byte == b' ' || byte == b'_' || byte == b'-' || byte.is_ascii_alphanumeric();
    starts(&lower, "begin").any(|at| {
        let dashes = lower[..at].trim_end_matches(char::is_whitespace);
        let from = at + 5;
        let run = &lower[from..from + run(lower.as_bytes(), from, class)];
        dashes.ends_with("-----")
            && run.match_indices("private key").any(|(key, _)| {
                lower[from + key + 11..]
                    .trim_start_matches(char::is_whitespace)
                    .starts_with("-----")
            })
    })
}

fn connection_string(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let url = |byte: u8| !byte.is_ascii_whitespace() && !b"<>\"{}|\\^`".contains(&byte);
    [
        "postgres://",
        "postgresql://",
        "mysql://",
        "mongodb://",
        "mongodb+srv://",
        "redis://",
        "amqp://",
    ]
    .iter()
    .any(|scheme| {
        starts(&lower, scheme).any(|at| {
            let from = at + scheme.len();
            let end = from + run(bytes, from, url);
            boundary(bytes, at)
                && lower[from..end]
                    .find('@')
                    .is_some_and(|sign| end - (from + sign + 1) >= 3)
                && ends(bytes, end, End::Separator)
        })
    })
}

/// Any `postgres://` URL, as TruffleHog's postgres detector reads one.
fn postgres(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    ["postgres://", "postgresql://"].iter().any(|scheme| {
        starts(&lower, scheme).any(|at| {
            let from = at + scheme.len();
            let end = from + run(bytes, from, |byte| !byte.is_ascii_whitespace());
            boundary(bytes, at) && bytes[from..end].iter().any(|&byte| word(byte))
        })
    })
}

fn credentials(text: &str) -> bool {
    let bytes = text.as_bytes();
    let host = |byte: u8| word(byte) || b"-.%/:".contains(&byte);
    [
        ("http://", 0, 2),
        ("https://", 0, 2),
        ("ftp://", 3, 1),
        ("redis://", 3, 1),
        ("rediss://", 3, 1),
        ("amqp://", 3, 1),
        ("amqps://", 3, 1),
        ("mongodb://", 3, 1),
        ("mongodb+srv://", 3, 1),
    ]
    .iter()
    .any(|&(scheme, user, least)| {
        starts(text, scheme).any(|at| {
            let from = at + scheme.len();
            let end = from + run(bytes, from, |byte| !byte.is_ascii_whitespace());
            let Some(sign) = text[from..end].rfind('@').map(|sign| from + sign) else {
                return false;
            };
            let login = &text[from..sign];
            let host = &bytes[sign + 1..sign + 1 + run(bytes, sign + 1, host)];
            boundary(bytes, at)
                && login.match_indices(':').any(|(colon, _)| {
                    (user..=50).contains(&colon) && (3..=88).contains(&(login.len() - colon - 1))
                })
                && host.len() >= least
                && host.iter().any(|&byte| word(byte))
                && real(login, 0.0)
        })
    })
}

fn bearer(text: &str) -> bool {
    let bytes = text.as_bytes();
    starts(text, "Bearer").any(|at| {
        let spaces = run(bytes, at + 6, |byte| byte.is_ascii_whitespace());
        let from = at + 6 + spaces;
        let end = from + run(bytes, from, |byte| url_safe(byte) || byte == b'.');
        let token = &text[from..end];
        let first = token.find('.').unwrap_or(token.len());
        let closed = bytes
            .get(end)
            .is_none_or(|&byte| byte.is_ascii_whitespace() || b"'\"`".contains(&byte));
        spaces > 0 && first >= 20 && token.matches('.').count() <= 2 && closed && real(token, 3.5)
    })
}

fn inngest_event(text: &str) -> bool {
    let bytes = text.as_bytes();
    starts(text, "INNGEST_EVENT_KEY").any(|at| {
        let rest = text[at + 17..].trim_start_matches(char::is_whitespace);
        let Some(rest) = rest.strip_prefix(['=', ':']) else {
            return false;
        };
        let rest = rest.trim_start_matches(char::is_whitespace);
        let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest);
        let from = text.len() - rest.len();
        run(bytes, from, |byte| byte.is_ascii_hexdigit()) >= 16
    })
}

fn sanity(text: &str) -> bool {
    let bytes = text.as_bytes();
    starts(text, "sk").any(|at| {
        let len = run(bytes, at + 2, |byte| byte.is_ascii_alphanumeric());
        let end = at + 2 + len;
        boundary(bytes, at)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_uppercase)
            && len >= 59
            && boundary(bytes, end)
            && real(&text[at..end], 4.0)
    })
}

fn starts<'a>(text: &'a str, prefix: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.match_indices(prefix).map(|(at, _)| at)
}

fn run(bytes: &[u8], from: usize, class: impl Fn(u8) -> bool) -> usize {
    bytes.get(from..).map_or(0, |rest| {
        rest.iter().take_while(|&&byte| class(byte)).count()
    })
}

fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn url_safe(byte: u8) -> bool {
    word(byte) || byte == b'-'
}

fn lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn boundary(bytes: &[u8], at: usize) -> bool {
    let before = at > 0 && word(bytes[at - 1]);
    let after = bytes.get(at).is_some_and(|&byte| word(byte));
    before != after
}

fn ends(bytes: &[u8], at: usize, end: End) -> bool {
    match end {
        End::Free => true,
        End::Boundary => boundary(bytes, at),
        End::Separator => match bytes.get(at..) {
            None | Some([]) => true,
            Some([b'\\', b'n' | b'r', ..]) => true,
            Some([byte, ..]) => b"`'\";".contains(byte) || byte.is_ascii_whitespace(),
        },
    }
}

const KNOWN_PREFIXES: &[&str] = &[
    "sk-proj-",
    "sk-svcacct-",
    "sk-admin-",
    "sk-ant-api03-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    "pk_test_",
    "rk_live_",
    "rk_test_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxo-",
    "xapp-",
    "glpat-",
    "glsa_",
    "npm_",
    "lin_api_",
    "gsk_",
    "r8_",
    "sbp_",
    "sk_",
    "SG.",
    "dp.pt.",
    "PMAK-",
    "signkey-prod-",
    "signkey-test-",
    "signkey-staging-",
    "sk-",
    "AKIA",
];

const MIN_SUFFIX_ENTROPY: f64 = 1.5;

fn real(secret: &str, floor: f64) -> bool {
    entropy(secret) >= floor && !placeholder(secret)
}

fn placeholder(secret: &str) -> bool {
    let prefix = KNOWN_PREFIXES
        .iter()
        .filter(|prefix| secret.starts_with(**prefix))
        .max_by_key(|prefix| prefix.len());
    prefix.is_some_and(|prefix| {
        let suffix = &secret[prefix.len()..];
        suffix.is_empty() || entropy(suffix) < MIN_SUFFIX_ENTROPY
    })
}

fn entropy(text: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for ch in text.chars() {
        *counts.entry(ch).or_default() += 1;
    }
    let total = text.chars().count() as f64;
    counts
        .values()
        .map(|&count| {
            let share = count as f64 / total;
            -share * share.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::detected;

    #[test]
    fn bare_tokens_are_caught() {
        for text in [
            "my key AKIAIOSFODNN7EXAMPLE oops",
            "ghp_aB3dE5gH7jK9mN1pQ3sT5vX7zA9cE1gI3kM5",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEow",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "postgres://admin:hunter2@db.example.com:5432/app",
            "postgres://localhost/app",
            "https://deploy:s3cretpass@ci.example.com/hook",
            "Authorization: Bearer f8Kq2LmZ9xR4tW7yB1nC5vD3",
            "xoxb-1234567890-1234567890123-aBcDeFgHiJkL",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "sk-proj-Ab12Cd34Ef56Gh78Ij90Kl12Mn34Op56Qr78St90Uv12Wx34Yz56",
            concat!("sk_", "live_4eC39HqLyjWDarjtT1zdp7dc"),
        ] {
            assert!(detected(text), "{text}");
        }
    }

    #[test]
    fn prose_and_filler_pass() {
        for text in [
            "",
            "just shipped the new parser",
            "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            "AKIA is the prefix AWS keys use",
            "Bearer tokens go in the header",
            "mysql://localhost/app",
            "https://example.com/a@b",
            "-----BEGIN PUBLIC KEY-----",
            "my-sk-is-not-a-key",
        ] {
            assert!(!detected(text), "{text}");
        }
    }
}
