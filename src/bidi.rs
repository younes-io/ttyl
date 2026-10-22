//! Terminals draw a line's characters left to right in the order they arrive, so right-to-left script comes out
//! backwards. This puts one line in display order. Joining letters into their connected shapes is left to the
//! terminal.

use ratatui::{style::Style, text::Span};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Left,
    Right,
    Neutral,
}

fn flow(grapheme: &str) -> Flow {
    let Some(ch) = grapheme.chars().next() else {
        return Flow::Neutral;
    };
    match u32::from(ch) {
        0x40 | 0x0660..=0x0669 | 0x06F0..=0x06F9 => Flow::Left,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF => Flow::Right,
        _ if ch.is_alphanumeric() => Flow::Left,
        _ => Flow::Neutral,
    }
}

fn mirror(grapheme: &str) -> &str {
    match grapheme {
        "(" => ")",
        ")" => "(",
        "[" => "]",
        "]" => "[",
        "{" => "}",
        "}" => "{",
        "<" => ">",
        ">" => "<",
        "«" => "»",
        "»" => "«",
        other => other,
    }
}

/// The line's direction is its first letter's. Neutral marks between two letters of one direction take it,
/// and otherwise the line's. Runs against the line's direction keep their own order inside it. Trailing blanks
/// stay at the end so a wrapped line never starts with a gap.
pub(crate) fn visual(line: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let rtl = |span: &Span| span.content.graphemes(true).any(|g| flow(g) == Flow::Right);
    if !line.iter().any(rtl) {
        return line;
    }
    let mut cells: Vec<(String, Style, Flow)> = line
        .iter()
        .flat_map(|span| {
            span.content
                .graphemes(true)
                .map(|g| (g.to_owned(), span.style, flow(g)))
        })
        .collect();
    let end = cells
        .iter()
        .rposition(|(g, ..)| !g.trim().is_empty())
        .map_or(0, |last| last + 1);
    let trailing = cells.split_off(end);

    let strong: Vec<Flow> = cells.iter().map(|cell| cell.2).collect();
    let paragraph = strong
        .iter()
        .copied()
        .find(|&f| f != Flow::Neutral)
        .unwrap_or(Flow::Left);
    for (index, cell) in cells.iter_mut().enumerate() {
        if cell.2 != Flow::Neutral {
            continue;
        }
        let before = strong[..index].iter().rev().find(|&&f| f != Flow::Neutral);
        let after = strong[index + 1..].iter().find(|&&f| f != Flow::Neutral);
        cell.2 = match (before, after) {
            (Some(before), Some(after)) if before == after => *before,
            _ => paragraph,
        };
        if cell.2 == Flow::Right {
            cell.0 = mirror(&cell.0).to_owned();
        }
    }

    let embedded = if paragraph == Flow::Right {
        cells.reverse();
        Flow::Left
    } else {
        Flow::Right
    };
    let mut start = 0;
    while start < cells.len() {
        let run = cells[start..]
            .iter()
            .take_while(|cell| cell.2 == embedded)
            .count();
        cells[start..start + run].reverse();
        start += run.max(1);
    }

    let mut out: Vec<Span<'static>> = Vec::new();
    for (grapheme, style, _) in cells.into_iter().chain(trailing) {
        match out.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(&grapheme),
            _ => out.push(Span::styled(grapheme, style)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(text: &str) -> String {
        visual(vec![Span::raw(text.to_owned())])
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn latin_lines_are_untouched() {
        assert_eq!(
            shown("fix the (index) on created_at "),
            "fix the (index) on created_at "
        );
    }

    #[test]
    fn a_persian_line_reads_from_the_right_with_latin_words_kept_whole() {
        assert_eq!(
            shown("🚀 کدریویو با AI، این بار از Alibaba!"),
            "!Alibaba زا راب نیا ،AI اب ویویردک 🚀"
        );
    }

    #[test]
    fn a_persian_word_inside_english_is_turned_in_place() {
        assert_eq!(shown("say سلام to them"), "say مالس to them");
    }

    #[test]
    fn brackets_inside_persian_face_the_right_way_and_trailing_blanks_stay_last() {
        assert_eq!(shown("متن (باز) "), "(زاب) نتم ");
    }

    #[test]
    fn links_and_numbers_inside_persian_keep_their_order() {
        assert_eq!(
            shown("نسخه 2.5 در t.me/teamyo_ir"),
            "t.me/teamyo_ir رد 2.5 هخسن"
        );
    }

    #[test]
    fn styles_follow_their_letters() {
        let line = visual(vec![
            Span::raw("سلام ".to_owned()),
            Span::styled("@ali".to_owned(), Style::new().bold()),
        ]);
        assert_eq!(line[0].content, "@ali");
        assert_eq!(line[1].content, " مالس");
    }
}
