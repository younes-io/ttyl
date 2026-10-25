//! The web's dark theme tokens from `resources/css/app.css`.

use std::cell::Cell;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::code::Hl;
use crate::model::Lane;

pub const BACKGROUND: Color = Color::Rgb(0x11, 0x11, 0x12);
pub const FOREGROUND: Color = Color::Rgb(0xec, 0xec, 0xec);
pub const PRIMARY: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
pub const MUTED: Color = Color::Rgb(0x23, 0x23, 0x26);
pub const MUTED_FOREGROUND: Color = Color::Rgb(0xa1, 0xa1, 0xa6);
pub const DESTRUCTIVE: Color = Color::Rgb(0xff, 0x6b, 0x6b);
pub const RAIL: Color = Color::Rgb(0x36, 0x36, 0x3a);
pub const FAINT: Color = Color::Rgb(0x6b, 0x6b, 0x72);
pub const BRAND: Color = Color::Rgb(0xff, 0x2d, 0x20);

const LANES: [Color; 6] = [
    Color::Rgb(0xff, 0x8a, 0x4c),
    Color::Rgb(0x51, 0xcf, 0x66),
    Color::Rgb(0xda, 0x77, 0xf2),
    Color::Rgb(0x3b, 0xc9, 0xdb),
    Color::Rgb(0xf0, 0x65, 0x95),
    Color::Rgb(0xfc, 0xc4, 0x19),
];

pub const ACCENT: Color = LANES[0];
pub const AMBER: Color = LANES[5];

pub fn lane(lane: Lane) -> Color {
    match lane {
        Lane::Palette(index) => LANES[usize::from(index)],
        Lane::Custom { r, g, b } => readable(r, g, b),
    }
}

/// People pick any color on the web, so a dark one is lifted, keeping its hue and saturation,
/// until it reads against the background as clearly as body text must (WCAG's 4.5:1).
fn readable(r: u8, g: u8, b: u8) -> Color {
    let [r, g, b] = [r, g, b].map(|channel| f64::from(channel) / 255.0);
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = f64::midpoint(high, low);
    let chroma = high - low;
    let saturation = if chroma == 0.0 {
        0.0
    } else {
        chroma / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    let hue = if chroma == 0.0 {
        0.0
    } else if high == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if high == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    (0..=100)
        .map(|step| lightness + (1.0 - lightness) * f64::from(step) / 100.0)
        .map(|lightness| {
            let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
            let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
            let (r, g, b) = match hue as u8 {
                0 => (chroma, x, 0.0),
                1 => (x, chroma, 0.0),
                2 => (0.0, chroma, x),
                3 => (0.0, x, chroma),
                4 => (x, 0.0, chroma),
                _ => (chroma, 0.0, x),
            };
            let low = lightness - chroma / 2.0;
            let byte = |channel: f64| ((channel + low) * 255.0).round().clamp(0.0, 255.0) as u8;
            (byte(r), byte(g), byte(b))
        })
        .find(|&(r, g, b)| contrast(Color::Rgb(r, g, b)) >= 4.5)
        .map_or(FOREGROUND, |(r, g, b)| Color::Rgb(r, g, b))
}

/// WCAG's contrast ratio of `color` over the background.
pub fn contrast(color: Color) -> f64 {
    let luminance = |color: Color| {
        let Color::Rgb(r, g, b) = color else {
            unreachable!("theme colors are RGB");
        };
        let linear = |channel: u8| {
            let channel = f64::from(channel) / 255.0;
            if channel <= 0.040_45 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    };
    (luminance(color) + 0.05) / (luminance(BACKGROUND) + 0.05)
}

/// `lane` washed faintly over the background, for the selected card.
pub fn tint(lane: Lane) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(under_r, under_g, under_b)) =
        (self::lane(lane), BACKGROUND)
    else {
        unreachable!("lanes and the background are RGB");
    };
    let mix = |over: u8, under: u8| {
        u8::try_from((u16::from(over) * 3 + u16::from(under) * 22) / 25)
            .expect("a mix of bytes fits")
    };
    Color::Rgb(mix(r, under_r), mix(g, under_g), mix(b, under_b))
}

/// The brand mark, two slashes with the second in the accent, as on the web's logo.
pub fn mark() -> [Span<'static>; 2] {
    let ink = Style::new().fg(FOREGROUND).add_modifier(Modifier::BOLD);
    [Span::styled("/", ink), Span::styled("/", ink.fg(ACCENT))]
}

/// The brand wordmark, /dev/null with its second slash in the accent.
pub fn wordmark() -> [Span<'static>; 3] {
    let ink = Style::new().fg(FOREGROUND).add_modifier(Modifier::BOLD);
    [
        Span::styled("/dev", ink),
        Span::styled("/", ink.fg(ACCENT)),
        Span::styled("null", ink),
    ]
}

/// The mark beside the wordmark, as the web's header shows them.
pub fn logo() -> Vec<Span<'static>> {
    let mut spans = mark().to_vec();
    spans.push(Span::raw(" "));
    spans.extend(wordmark());
    spans
}

pub fn muted() -> Style {
    Style::new().fg(MUTED_FOREGROUND)
}

pub fn hl(hl: Option<Hl>) -> Style {
    let base = Style::new().fg(FOREGROUND).bg(MUTED);
    match hl {
        None => base,
        Some(Hl::Keyword) => base.fg(PRIMARY),
        Some(Hl::Type) => base.fg(BRAND),
        Some(Hl::Property) => base.fg(LANES[3]),
        Some(Hl::Value | Hl::Addition) => base.fg(LANES[1]),
        Some(Hl::Number) => base.fg(LANES[0]),
        Some(Hl::Variable) => base.fg(LANES[4]),
        Some(Hl::Comment) => base.fg(MUTED_FOREGROUND).add_modifier(Modifier::ITALIC),
        Some(Hl::Attribute) => base.add_modifier(Modifier::ITALIC),
        Some(Hl::Deletion) => base.fg(DESTRUCTIVE),
    }
}

/// The marks drawn before counts and attachments.
pub struct Icons {
    pub star: &'static str,
    pub reply: &'static str,
    pub pastel: &'static str,
    pub quote: &'static str,
    pub link: &'static str,
    pub image: &'static str,
    pub poll: &'static str,
    pub sparkle: &'static str,
    pub twinkle: &'static str,
    pub mote: &'static str,
}

// Most monospace fonts lack ★, ❝ and ⇗, so terminals draw them from a fallback font at a fraction of
// the cell. Nerd Font icons fill the cell.
const NERD: Icons = Icons {
    star: "\u{f005}",
    reply: "\u{f112}",
    pastel: "\u{f0198}",
    quote: "\u{f10d}",
    link: "\u{f08e}",
    image: "\u{f03e}",
    poll: "\u{f080}",
    sparkle: "\u{f0ae2}",
    twinkle: "\u{f0ae3}",
    mote: "\u{f09de}",
};

const PLAIN: Icons = Icons {
    star: "★",
    reply: "↩",
    pastel: "◍",
    quote: "❝",
    link: "⇗",
    image: "▣",
    poll: "≡",
    sparkle: "✦",
    twinkle: "✧",
    mote: "·",
};

thread_local! {
    static PASTEIS: Cell<bool> = const { Cell::new(false) };
}

/// Whether the viewer has pastéis mode on. Off, the feed shows pastéis as plain words and no
/// pastel counts, as dev-null.sh does. The engine sets it from the account before each frame.
pub fn pasteis() -> bool {
    PASTEIS.get()
}

pub fn set_pasteis(on: bool) {
    PASTEIS.set(on);
}

/// Nerd Font icons, or plain Unicode when `TTYL_ICONS=plain`.
pub fn icons() -> &'static Icons {
    static ICONS: OnceLock<&Icons> = OnceLock::new();
    ICONS.get_or_init(|| match std::env::var("TTYL_ICONS").as_deref() {
        Ok("plain") => &PLAIN,
        _ => &NERD,
    })
}
