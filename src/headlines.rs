use ratatui::style::Style;

use crate::feed;
use crate::model::Headline;

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[m";
/// Recaps wrap here, indented the way `git log` indents a commit message.
const WIDTH: usize = 72;
const INDENT: &str = "    ";

/// The headlines as `ttyl headlines` prints them: each name on its own line, with its recap
/// wrapped and indented under it, and the names in bold when `bold`.
pub fn print(headlines: &[Headline], bold: bool) -> String {
    let mut out = String::new();
    for (at, headline) in headlines.iter().enumerate() {
        if at > 0 {
            out.push('\n');
        }
        match bold {
            true => out.push_str(&format!("{BOLD}{}{RESET}\n", headline.name)),
            false => out.push_str(&format!("{}\n", headline.name)),
        }
        let Some(summary) = &headline.summary else {
            continue;
        };
        let mut lines = Vec::new();
        feed::wrap(&[(summary.clone(), Style::new())], WIDTH, &mut lines);
        for line in lines {
            let text: String = line.iter().map(|span| span.content.as_ref()).collect();
            out.push_str(INDENT);
            out.push_str(text.trim_end());
            out.push('\n');
        }
    }
    out
}
