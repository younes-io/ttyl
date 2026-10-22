use devnull::text::Buffer;
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone)]
enum Op {
    Insert(String),
    Left,
    Right,
    Up(usize),
    Down(usize),
    Home,
    End,
    WordLeft,
    WordRight,
    Backspace,
    Delete,
    DeleteWord,
    DeleteLine,
}

fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z ]{1,8}",
        "\\PC{1,6}",
        prop::sample::select(vec![
            "👩‍💻",
            "🇫",
            "🇷",
            "🇫🇷",
            "e",
            "\u{301}",
            "\u{200d}",
            "❤️",
            "\u{fe0f}",
            "漢字",
            "\n",
            "\r\n",
            "\r",
            "\t",
            "a\u{0}b",
            "_x_",
        ])
        .prop_map(str::to_owned),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => piece().prop_map(Op::Insert),
        1 => Just(Op::Left),
        1 => Just(Op::Right),
        1 => (1usize..12).prop_map(Op::Up),
        1 => (1usize..12).prop_map(Op::Down),
        1 => Just(Op::Home),
        1 => Just(Op::End),
        1 => Just(Op::WordLeft),
        1 => Just(Op::WordRight),
        1 => Just(Op::Backspace),
        1 => Just(Op::Delete),
        1 => Just(Op::DeleteWord),
        1 => Just(Op::DeleteLine),
    ]
}

fn apply(buffer: &mut Buffer, op: &Op) {
    match op {
        Op::Insert(text) => buffer.insert(text),
        Op::Left => buffer.left(),
        Op::Right => buffer.right(),
        Op::Up(width) => buffer.up(*width),
        Op::Down(width) => buffer.down(*width),
        Op::Home => buffer.home(),
        Op::End => buffer.end(),
        Op::WordLeft => buffer.word_left(),
        Op::WordRight => buffer.word_right(),
        Op::Backspace => buffer.backspace(),
        Op::Delete => buffer.delete(),
        Op::DeleteWord => buffer.delete_word(),
        Op::DeleteLine => buffer.delete_line(),
    }
}

fn on_a_grapheme(text: &str, at: usize) -> bool {
    at == text.len() || text.grapheme_indices(true).any(|(start, _)| start == at)
}

proptest! {
    #[test]
    fn edits_keep_the_cursor_on_a_grapheme_inside_the_text(
        start in "\\PC{0,20}",
        ops in prop::collection::vec(op(), 0..60),
        width in 1usize..30,
    ) {
        let mut buffer = Buffer::new(&start);
        for op in &ops {
            apply(&mut buffer, op);
            let text = buffer.text();
            prop_assert!(buffer.cursor() <= text.len(), "{op:?} put the cursor past {text:?}");
            prop_assert!(
                on_a_grapheme(text, buffer.cursor()),
                "{op:?} split a grapheme in {text:?} at {}",
                buffer.cursor()
            );
            prop_assert!(!text.contains('\r'));
            let rows = buffer.rows(width);
            let (row, column) = buffer.row_and_column(width);
            prop_assert!(row < rows.len());
            prop_assert!(column <= width.max(4) + 1);
            for range in &rows {
                prop_assert!(on_a_grapheme(text, range.start) && on_a_grapheme(text, range.end));
            }
        }
    }

    #[test]
    fn inserting_at_the_end_appends(start in "\\PC{0,20}", tail in "[a-z ]{1,10}") {
        let mut buffer = Buffer::new(&start);
        let before = buffer.text().to_owned();
        buffer.insert(&tail);
        prop_assert_eq!(buffer.text(), format!("{before}{tail}"));
        prop_assert_eq!(buffer.cursor(), buffer.text().len());
    }
}

#[test]
fn backspace_removes_a_whole_grapheme() {
    let mut buffer = Buffer::new("hi 👩‍💻");
    buffer.backspace();
    assert_eq!(buffer.text(), "hi ");
    let mut buffer = Buffer::new("cafe\u{301}");
    buffer.backspace();
    assert_eq!(buffer.text(), "caf");
}

#[test]
fn arrows_step_over_flags_and_accents() {
    let mut buffer = Buffer::new("a🇫🇷e\u{301}");
    buffer.left();
    assert_eq!(&buffer.text()[buffer.cursor()..], "e\u{301}");
    buffer.left();
    assert_eq!(&buffer.text()[buffer.cursor()..], "🇫🇷e\u{301}");
    buffer.delete();
    assert_eq!(buffer.text(), "ae\u{301}");
}

#[test]
fn paste_keeps_newlines_and_drops_carriage_returns() {
    let mut buffer = Buffer::new("");
    buffer.insert("fn main() {\r\n    run();\r\n}\r");
    assert_eq!(buffer.text(), "fn main() {\n    run();\n}\n");
}

#[test]
fn word_moves_and_deletes() {
    let mut buffer = Buffer::new("git commit --amend");
    buffer.word_left();
    assert_eq!(&buffer.text()[buffer.cursor()..], "amend");
    buffer.word_left();
    assert_eq!(&buffer.text()[buffer.cursor()..], "commit --amend");
    buffer.word_right();
    assert_eq!(&buffer.text()[buffer.cursor()..], " --amend");
    buffer.end();
    buffer.delete_word();
    assert_eq!(buffer.text(), "git commit --");
    buffer.delete_word();
    assert_eq!(buffer.text(), "git ");
}

#[test]
fn ctrl_u_clears_back_to_the_line_start_only() {
    let mut buffer = Buffer::new("first\nsecond line");
    buffer.delete_line();
    assert_eq!(buffer.text(), "first\n");
    buffer.delete_line();
    assert_eq!(buffer.text(), "first\n");
}

#[test]
fn home_and_end_stay_on_the_line() {
    let mut buffer = Buffer::new("one\ntwo\nthree");
    buffer.up(80);
    buffer.home();
    buffer.insert(">");
    buffer.end();
    buffer.insert("<");
    assert_eq!(buffer.text(), "one\n>two<\nthree");
}

#[test]
fn up_and_down_keep_the_column() {
    let mut buffer = Buffer::new("abcdef\nab\nabcdef");
    buffer.up(80);
    assert_eq!(buffer.row_and_column(80), (1, 2));
    buffer.up(80);
    assert_eq!(buffer.row_and_column(80), (0, 6));
    buffer.down(80);
    buffer.down(80);
    assert_eq!(buffer.row_and_column(80), (2, 6));
}

#[test]
fn long_lines_wrap_by_cell_width() {
    let buffer = Buffer::new("abcdefgh漢字");
    let rows = buffer.rows(4);
    let shown: Vec<String> = rows.iter().map(|row| buffer.display(row)).collect();
    assert_eq!(shown, ["abcd", "efgh", "漢字"]);
    assert_eq!(buffer.row_and_column(4), (2, 4));
}

#[test]
fn up_and_down_move_through_wrapped_rows() {
    let mut buffer = Buffer::new("abcdefghij");
    buffer.up(4);
    assert_eq!(buffer.row_and_column(4), (1, 2));
    buffer.up(4);
    assert_eq!(buffer.row_and_column(4), (0, 2));
    buffer.up(4);
    assert_eq!(buffer.cursor(), 0);
    buffer.down(4);
    buffer.down(4);
    buffer.down(4);
    assert_eq!(buffer.cursor(), buffer.text().len());
}
