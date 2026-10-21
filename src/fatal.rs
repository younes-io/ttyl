use std::time::Duration;

use ratatui::style::Style;
use ratatui::text::Line;
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

pub fn hint(failure: &Failure, retry: Option<Duration>, width: usize) -> String {
    let cause = match failure {
        Failure::Down => "the site is in maintenance",
        Failure::Offline(detail) | Failure::Refused(detail) => detail,
    };
    let retry = retry
        .map(|retry| format!(" · retrying in {}s", retry.as_secs()))
        .unwrap_or_default();
    let hint = format!("fatal: {cause} · your log is still here{retry}");
    if hint.width() <= width {
        hint
    } else {
        format!("fatal: {cause}{retry}")
    }
}
