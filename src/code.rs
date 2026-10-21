/// A token class tempest/highlight emits that the web gives a style. Classes it leaves unstyled, such as
/// `hl-operator` and `hl-injection`, inherit from the enclosing span, as CSS does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hl {
    Keyword,
    Type,
    Property,
    Value,
    Number,
    Variable,
    Comment,
    Attribute,
    Addition,
    Deletion,
}

impl Hl {
    fn from_class(class: &str) -> Option<Self> {
        Some(match class {
            "hl-keyword" => Hl::Keyword,
            "hl-type" | "hl-generic" => Hl::Type,
            "hl-property" => Hl::Property,
            "hl-value" => Hl::Value,
            "hl-number" | "hl-literal" => Hl::Number,
            "hl-variable" => Hl::Variable,
            "hl-comment" => Hl::Comment,
            "hl-attribute" => Hl::Attribute,
            "hl-addition" => Hl::Addition,
            "hl-deletion" => Hl::Deletion,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub hl: Option<Hl>,
}

pub type Line = Vec<Span>;

enum Frame {
    Styled(Option<Hl>),
    Hidden,
}

/// Turns a block segment's highlighted HTML into lines of styled text.
pub fn block_lines(html: &str) -> Vec<Line> {
    let mut lines = vec![Line::new()];
    let mut stack: Vec<Frame> = Vec::new();
    let mut rest = html;

    while !rest.is_empty() {
        if let Some(tag) = rest.strip_prefix('<') {
            let end = tag.find('>').map_or(tag.len(), |i| i + 1);
            let (inner, after) = tag.split_at(end);
            let inner = inner.trim_end_matches('>');
            if inner.starts_with("/span") {
                stack.pop();
            } else if inner.starts_with("span") {
                stack.push(open_span(inner));
            }
            rest = after;
            continue;
        }

        let end = rest.find('<').unwrap_or(rest.len());
        let (text, after) = rest.split_at(end);
        rest = after;
        if stack.iter().any(|frame| matches!(frame, Frame::Hidden)) {
            continue;
        }
        let hl = stack.iter().rev().find_map(|frame| match frame {
            Frame::Styled(hl) => *hl,
            Frame::Hidden => None,
        });
        let text = unescape(text);
        for (i, piece) in text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Line::new());
            }
            push(lines.last_mut().expect("lines starts non-empty"), piece, hl);
        }
    }
    lines
}

fn open_span(tag: &str) -> Frame {
    if tag.contains("display: none") {
        return Frame::Hidden;
    }
    let class = tag
        .split_once("class=\"")
        .and_then(|(_, value)| value.split_once('"'))
        .map_or("", |(class, _)| class);
    Frame::Styled(class.split_whitespace().find_map(Hl::from_class))
}

fn push(line: &mut Line, text: &str, hl: Option<Hl>) {
    if text.is_empty() {
        return;
    }
    match line.last_mut() {
        Some(last) if last.hl == hl => last.text.push_str(text),
        _ => line.push(Span {
            text: text.to_owned(),
            hl,
        }),
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest
            .find(';')
            .and_then(|end| Some((entity(&rest[1..end])?, end + 1)));
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let code = name.strip_prefix('#')?;
            let value = match code.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => code.parse().ok()?,
            };
            char::from_u32(value)
        }
    }
}
