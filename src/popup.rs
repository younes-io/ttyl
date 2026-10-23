use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Clear};

use crate::theme;

/// What a box casts on the screen below it.
const SHADOW: Color = Color::Rgb(0x08, 0x08, 0x09);

/// Fades everything in `area` but `lit`, so a box raised over it reads as closer than the screen.
pub fn dim(buf: &mut Buffer, area: Rect, lit: Option<Rect>) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if lit.is_some_and(|lit| lit.contains((x, y).into())) {
                continue;
            }
            buf[(x, y)]
                .set_fg(theme::FAINT)
                .set_bg(theme::BACKGROUND)
                .modifier = Modifier::empty();
        }
    }
}

/// Draws `block` as a box raised over the screen in `area`, filled a step lighter than the screen
/// with an accent edge and a shadow, and returns the room inside it.
pub fn raise(frame: &mut Frame, area: Rect, block: Block) -> Rect {
    let screen = frame.area();
    let buf = frame.buffer_mut();
    let shade = |buf: &mut Buffer, x: u16, y: u16| {
        if screen.contains((x, y).into()) {
            buf[(x, y)].set_bg(SHADOW).set_fg(theme::RAIL);
        }
    };
    for y in area.top() + 1..=area.bottom() {
        shade(buf, area.right(), y);
    }
    for x in area.left() + 1..area.right() {
        shade(buf, x, area.bottom());
    }
    let block = block
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::PRIMARY))
        .style(Style::new().bg(theme::MUTED).fg(theme::FOREGROUND));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    inner
}

/// The option a box has selected, filled in.
pub fn chosen() -> Style {
    Style::new()
        .fg(theme::BACKGROUND)
        .bg(theme::PRIMARY)
        .add_modifier(Modifier::BOLD)
}

/// Where a box `width` by `height` goes in `screen`: just under `near` when it fits there, else
/// just above it, else in the middle. The box keeps a cell clear for its shadow.
pub fn place(screen: Rect, width: u16, height: u16, near: Option<Rect>) -> Rect {
    let width = width.min(screen.width.saturating_sub(1));
    let height = height.min(screen.height.saturating_sub(1));
    let x = screen.x + (screen.width - width) / 2;
    let fits = |y: u16| y >= screen.y && y + height < screen.bottom();
    let y = near
        .and_then(|near| {
            [Some(near.bottom()), near.y.checked_sub(height + 1)]
                .into_iter()
                .flatten()
                .find(|y| fits(*y))
        })
        .unwrap_or(screen.y + (screen.height - height) / 2);
    Rect {
        x,
        y,
        width,
        height,
    }
}
