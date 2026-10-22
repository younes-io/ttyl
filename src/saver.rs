//! The /dev/null wordmark drifting over a blank screen until a key brings the session back.
//!
//! Each glyph is a 10-pixel-high bitmap drawn two pixels to a cell with half blocks, so the
//! pixels are about square and the wordmark moves half a row at a time.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

use crate::theme;

const WORD: &str = "/dev/null";
const HEIGHT: usize = 10;
/// One pixel each way per step, so the wordmark drifts diagonally.
pub const STEP: Duration = Duration::from_millis(110);

#[rustfmt::skip]
fn glyph(ch: char) -> [&'static str; HEIGHT] {
    match ch {
        '/' => ["....##", "....##", "...##.", "...##.", "..##..", "..##..", ".##...", ".##...", "##....", "##...."],
        'd' => ["....##", "....##", "....##", ".#####", "##..##", "##..##", "##..##", "##..##", "##..##", ".#####"],
        'e' => ["......", "......", "......", ".####.", "##..##", "##..##", "######", "##....", "##....", ".#####"],
        'v' => ["......", "......", "......", "##..##", "##..##", "##..##", "##..##", ".####.", ".####.", "..##.."],
        'n' => ["......", "......", "......", "#####.", "##..##", "##..##", "##..##", "##..##", "##..##", "##..##"],
        'u' => ["......", "......", "......", "##..##", "##..##", "##..##", "##..##", "##..##", "##..##", ".#####"],
        'l' => ["##"; HEIGHT],
        _ => unreachable!("the wordmark is only /dev/null"),
    }
}

/// The wordmark as rows of pixels, the slashes in the accent and the letters in ink.
fn pixels() -> Vec<Vec<Option<Color>>> {
    let mut rows = vec![Vec::new(); HEIGHT];
    for (at, ch) in WORD.chars().enumerate() {
        let color = if ch == '/' {
            theme::ACCENT
        } else {
            theme::FOREGROUND
        };
        for (row, bits) in rows.iter_mut().zip(glyph(ch)) {
            if at > 0 {
                row.push(None);
            }
            row.extend(bits.chars().map(|bit| (bit == '#').then_some(color)));
        }
    }
    rows
}

/// Where `steps` lands on a path that runs back and forth across `0..=range`, starting halfway.
fn bounce(steps: u64, range: u16) -> u16 {
    let range = u64::from(range);
    if range == 0 {
        return 0;
    }
    let at = (steps + range / 2) % (2 * range);
    u16::try_from(at.min(2 * range - at)).expect("within the range")
}

pub fn draw(frame: &mut Frame, since: Instant, now: Instant) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::new().bg(theme::BACKGROUND)), area);
    let art = pixels();
    let width = u16::try_from(art[0].len()).expect("the wordmark fits a row");
    let steps =
        u64::try_from(now.duration_since(since).as_millis() / STEP.as_millis()).unwrap_or(u64::MAX);
    let left = bounce(steps, area.width.saturating_sub(width));
    let top = bounce(steps, (area.height * 2).saturating_sub(HEIGHT as u16));
    let pixel = |x: u16, y: u16| {
        let row = art.get(usize::from(y.checked_sub(top)?))?;
        *row.get(usize::from(x.checked_sub(left)?))?
    };
    let buffer = frame.buffer_mut();
    for y in 0..area.height {
        for x in left..(left + width).min(area.width) {
            let (upper, lower) = (pixel(x, 2 * y), pixel(x, 2 * y + 1));
            let (symbol, fg, bg) = match (upper, lower) {
                (None, None) => continue,
                (Some(upper), None) => ("▀", upper, theme::BACKGROUND),
                (None, Some(lower)) => ("▄", lower, theme::BACKGROUND),
                (Some(upper), Some(lower)) if upper == lower => ("█", upper, theme::BACKGROUND),
                (Some(upper), Some(lower)) => ("▀", upper, lower),
            };
            buffer[(area.x + x, area.y + y)]
                .set_symbol(symbol)
                .set_fg(fg)
                .set_bg(bg);
        }
    }
}
