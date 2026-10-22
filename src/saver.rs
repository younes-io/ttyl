//! The /dev/null wordmark drifting over a blank screen, then melting into the Game of Life,
//! until a key brings the session back.
//!
//! Each glyph is a 10-pixel-high bitmap drawn two pixels to a cell with half blocks, so the
//! pixels are about square and the wordmark moves half a row at a time. Life runs on the same
//! pixels, wrapping at the edges. Each time it settles another wordmark melts into it, and a
//! fresh one drifts in from the opposite side, so the wordmark never leaves the screen.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

use crate::theme;

const WORD: &str = "/dev/null";
const HEIGHT: usize = 10;
/// One pixel each way per step while the wordmark drifts, and one generation per step after.
pub const STEP: Duration = Duration::from_millis(110);
const DRIFT: Duration = Duration::from_secs(3);
/// How long a settled board stays up before the drifting wordmark melts into it.
const LINGER: Duration = Duration::from_secs(2);
/// Gliders can circle a wrapping board for a very long time before it repeats.
const GENERATIONS: u64 = 1500;

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

/// Where `at` lands on a path that runs back and forth across `0..=range`, starting halfway.
fn bounce(at: u64, range: u16) -> u16 {
    let range = u64::from(range);
    if range == 0 {
        return 0;
    }
    let at = (at + range / 2) % (2 * range);
    u16::try_from(at.min(2 * range - at)).expect("within the range")
}

fn steps(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis() / STEP.as_millis()).unwrap_or(u64::MAX)
}

pub struct Saver {
    drift: Drift,
    life: Option<Life>,
}

/// A wordmark drifting since `since`, `offset` steps along each axis from where the first starts.
struct Drift {
    since: Instant,
    offset: (u64, u64),
}

struct Life {
    width: u16,
    height: u16,
    born: Instant,
    generation: u64,
    cells: Vec<Option<Color>>,
    seen: HashSet<u64>,
    settled: Option<Instant>,
}

impl Drift {
    fn along(&self, now: Instant) -> (u64, u64) {
        let steps = steps(now.duration_since(self.since));
        (self.offset.0 + steps, self.offset.1 + steps)
    }
}

impl Saver {
    pub fn new(now: Instant) -> Saver {
        Saver {
            drift: Drift {
                since: now,
                offset: (0, 0),
            },
            life: None,
        }
    }

    /// Each time the board settles, or the first wordmark has drifted a while, the wordmark melts
    /// into the board and a new one starts from the opposite side, so one is always on screen.
    pub fn draw(&mut self, frame: &mut Frame, now: Instant) {
        let area = frame.area();
        let (width, height) = (area.width, area.height * 2);
        if let Some(life) = &self.life
            && (life.width, life.height) != (width, height)
        {
            *self = Saver::new(now);
        }
        let art = pixels();
        let range = (
            width.saturating_sub(u16::try_from(art[0].len()).expect("the wordmark fits a row")),
            height.saturating_sub(HEIGHT as u16),
        );
        let corner = |(x, y): (u64, u64)| (bounce(x, range.0), bounce(y, range.1));
        let logo = |(left, top): (u16, u16), x: u16, y: u16| {
            let row = art.get(usize::from(y.checked_sub(top)?))?;
            *row.get(usize::from(x.checked_sub(left)?))?
        };
        if let Some(life) = &mut self.life {
            life.advance(now);
        }
        let melt = match &self.life {
            None => Some(self.drift.since + DRIFT),
            Some(life) => life.settled.map(|at| at + LINGER),
        };
        if let Some(at) = melt.filter(|at| *at <= now) {
            let along = self.drift.along(at);
            let seed = corner(along);
            let cells = self.life.take().map_or_else(
                || vec![None; usize::from(width) * usize::from(height)],
                |life| life.cells,
            );
            let cells = cells
                .into_iter()
                .enumerate()
                .map(|(i, cell)| {
                    let (x, y) = (i % usize::from(width), i / usize::from(width));
                    logo(seed, x as u16, y as u16).or(cell)
                })
                .collect();
            let mut life = Life::new(width, height, at, cells);
            life.advance(now);
            self.life = Some(life);
            self.drift = Drift {
                since: at,
                offset: (along.0 + u64::from(range.0), along.1 + u64::from(range.1)),
            };
        }
        let at = corner(self.drift.along(now));
        paint(frame, |x, y| {
            logo(at, x, y).or_else(|| {
                let life = self.life.as_ref()?;
                life.cells[usize::from(y) * usize::from(width) + usize::from(x)]
            })
        });
    }
}

impl Life {
    fn new(width: u16, height: u16, born: Instant, cells: Vec<Option<Color>>) -> Life {
        let mut life = Life {
            width,
            height,
            born,
            generation: 0,
            cells,
            seen: HashSet::new(),
            settled: None,
        };
        life.seen.insert(life.fingerprint());
        life
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for cell in &self.cells {
            cell.is_some().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn advance(&mut self, now: Instant) {
        let target = steps(now.duration_since(self.born));
        while self.generation < target && self.settled.is_none() {
            self.step();
            self.generation += 1;
            if !self.seen.insert(self.fingerprint()) || self.generation >= GENERATIONS {
                let after = STEP * u32::try_from(self.generation).unwrap_or(u32::MAX);
                self.settled = Some(self.born + after);
            }
        }
    }

    /// A newborn takes the color most of its three parents share, so the slashes' accent and the
    /// letters' ink spread into each other.
    fn step(&mut self) {
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        let at = |x: usize, y: usize| self.cells[y * width + x];
        self.cells = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let parents: Vec<Color> = (0..3)
                    .flat_map(|dy| (0..3).map(move |dx| (dx, dy)))
                    .filter(|&offset| offset != (1, 1))
                    .filter_map(|(dx, dy)| {
                        at((x + width - 1 + dx) % width, (y + height - 1 + dy) % height)
                    })
                    .collect();
                match (at(x, y), parents.len()) {
                    (Some(color), 2 | 3) => Some(color),
                    (None, 3) if parents[1] == parents[2] => Some(parents[1]),
                    (None, 3) => Some(parents[0]),
                    _ => None,
                }
            })
            .collect();
    }
}

/// Fills the screen, drawing each cell's upper and lower pixel with one half block.
fn paint(frame: &mut Frame, pixel: impl Fn(u16, u16) -> Option<Color>) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::new().bg(theme::BACKGROUND)), area);
    let buffer = frame.buffer_mut();
    for y in 0..area.height {
        for x in 0..area.width {
            let (symbol, fg, bg) = match (pixel(x, 2 * y), pixel(x, 2 * y + 1)) {
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
