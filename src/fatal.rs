use std::time::Duration;

use ratatui::style::Style;
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::api::Failure;
use crate::theme;

pub fn screen(failure: &Failure, retry: Option<Duration>) -> Vec<Line<'static>> {
    let fatal = Style::new().fg(theme::DESTRUCTIVE);
    let mut lines = match failure {
        Failure::Down => vec![
            Line::styled("fatal: unable to access 'https://dev-null.sh/':", fatal),
            Line::raw("       the site is in maintenance"),
            Line::default(),
            Line::styled(
                "We are not down, we're making sure the only",
                theme::muted(),
            ),
            Line::styled("remote code running here is ours.", theme::muted()),
        ],
        Failure::Offline(detail) => vec![
            Line::styled("fatal: unable to access 'https://dev-null.sh/':", fatal),
            Line::raw(format!("       {detail}")),
        ],
        Failure::Refused(message) => vec![Line::styled(format!("fatal: {message}"), fatal)],
    };
    if let Some(retry) = retry {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("retrying in {}s", retry.as_secs()),
            theme::muted(),
        ));
    }
    lines
}

/// The footer line for `failure`, shortened to `width` by dropping the reassurance first,
/// then cutting the cause, and last dropping the countdown. `None` when not even a cut
/// cause fits.
pub fn hint(failure: &Failure, retry: Option<Duration>, width: usize) -> Option<String> {
    let cause = match failure {
        Failure::Down => "the site is in maintenance",
        Failure::Offline(detail) | Failure::Refused(detail) => detail,
    };
    let retry = retry
        .map(|retry| format!(" · retrying in {}s", retry.as_secs()))
        .unwrap_or_default();
    let full = format!("fatal: {cause} · your log is still here{retry}");
    if full.width() <= width {
        return Some(full);
    }
    let short = format!("fatal: {cause}{retry}");
    if short.width() <= width {
        return Some(short);
    }
    let room = width.saturating_sub("fatal: ".width() + retry.width());
    if room >= 4 {
        return Some(format!("fatal: {}{retry}", cut(cause, room)));
    }
    let room = width.saturating_sub("fatal: ".width());
    (room >= 4).then(|| format!("fatal: {}", cut(cause, room)))
}

pub fn cut(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut kept = String::new();
    let mut room = width.saturating_sub(1);
    for grapheme in text.graphemes(true) {
        if grapheme.width() > room {
            break;
        }
        room -= grapheme.width();
        kept.push_str(grapheme);
    }
    kept.push('…');
    kept
}
