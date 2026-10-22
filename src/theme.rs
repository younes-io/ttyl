//! The web's dark theme tokens from `resources/css/app.css`.

use ratatui::style::{Color, Modifier, Style};

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

pub const AMBER: Color = LANES[5];

pub fn lane(lane: Lane) -> Color {
    match lane {
        Lane::Palette(index) => LANES[usize::from(index)],
        Lane::Custom { r, g, b } => Color::Rgb(r, g, b),
    }
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
