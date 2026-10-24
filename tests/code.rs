use ttyl::code::{Hl, Span, block_lines};

fn span(text: &str, hl: Option<Hl>) -> Span {
    Span {
        text: text.into(),
        hl,
    }
}

#[test]
fn highlighted_php_keeps_classes_and_plain_text() {
    let lines = block_lines(
        r#"<span class="hl-variable">$name</span> = <span class="hl-value">'devnull'</span>;"#,
    );
    assert_eq!(
        lines,
        vec![vec![
            span("$name", Some(Hl::Variable)),
            span(" = ", None),
            span("'devnull'", Some(Hl::Value)),
            span(";", None),
        ]]
    );
}

#[test]
fn unhighlighted_block_splits_into_lines() {
    assert_eq!(
        block_lines("x\ny"),
        vec![vec![span("x", None)], vec![span("y", None)]]
    );
}

#[test]
fn escaped_markup_comes_back_as_literal_text() {
    assert_eq!(
        block_lines("&lt;script&gt;alert(1)&lt;/script&gt;"),
        vec![vec![span("<script>alert(1)</script>", None)]]
    );
    assert_eq!(
        block_lines("a &amp;&amp; b &quot;&#039;&#x41;"),
        vec![vec![span("a && b \"'A", None)]]
    );
}

#[test]
fn stray_ampersand_is_kept() {
    assert_eq!(
        block_lines("a & b; &bogus;"),
        vec![vec![span("a & b; &bogus;", None)]]
    );
}

#[test]
fn style_carries_across_a_line_break() {
    let lines = block_lines("<span class=\"hl-comment\">/* one\ntwo */</span> x");
    assert_eq!(
        lines,
        vec![
            vec![span("/* one", Some(Hl::Comment))],
            vec![span("two */", Some(Hl::Comment)), span(" x", None)]
        ]
    );
}

#[test]
fn unstyled_inner_span_inherits_and_styled_inner_span_wins() {
    let lines = block_lines(
        r#"<span class="hl-value">"a <span class="hl-operator">+</span> <span class="hl-injection"><span class="hl-variable">$b</span></span>"</span>"#,
    );
    assert_eq!(
        lines,
        vec![vec![
            span("\"a + ", Some(Hl::Value)),
            span("$b", Some(Hl::Variable)),
            span("\"", Some(Hl::Value))
        ]]
    );
}

#[test]
fn hidden_tokens_are_dropped() {
    let lines = block_lines(
        r#"<span class="hl-keyword">fn</span><span style="display: none">secret</span> main"#,
    );
    assert_eq!(
        lines,
        vec![vec![span("fn", Some(Hl::Keyword)), span(" main", None)]]
    );
}

#[test]
fn generic_and_literal_share_type_and_number_styles() {
    let lines = block_lines(
        r#"<span class="hl-generic">T</span><span class="hl-type">U</span><span class="hl-literal">true</span><span class="hl-number">1</span>"#,
    );
    assert_eq!(
        lines,
        vec![vec![
            span("TU", Some(Hl::Type)),
            span("true1", Some(Hl::Number))
        ]]
    );
}

fn block(language: Option<&str>, html: &str) -> Vec<ttyl::code::Line> {
    let json = serde_json::json!({ "type": "block", "language": language, "html": html });
    match serde_json::from_value(json).expect("a block segment") {
        ttyl::model::Segment::Block { lines, .. } => lines,
        other => panic!("not a block: {other:?}"),
    }
}

#[test]
fn a_language_syntect_knows_is_highlighted_locally() {
    assert_eq!(
        block(Some("rust"), "fn main() {}"),
        vec![vec![
            span("fn", Some(Hl::Keyword)),
            span(" ", None),
            span("main", Some(Hl::Property)),
            span("() {}", None),
        ]]
    );
}

#[test]
fn php_snippets_without_an_opening_tag_still_highlight() {
    assert_eq!(
        block(Some("php"), "$name = 'devnull';"),
        vec![vec![
            span("$name", Some(Hl::Variable)),
            span(" = ", None),
            span("'devnull'", Some(Hl::Value)),
            span(";", None),
        ]]
    );
}

#[test]
fn a_language_syntect_does_not_know_keeps_the_server_colors() {
    let html = r#"<span class="hl-keyword">interface</span> A {}"#;
    assert_eq!(block(Some("typescript"), html), block_lines(html));
}

#[test]
fn a_block_without_a_language_keeps_the_server_colors() {
    let html = r#"<span class="hl-keyword">select</span> 1"#;
    assert_eq!(block(None, html), block_lines(html));
}

#[test]
fn local_highlighting_reads_the_unescaped_text() {
    let text: String = block(Some("rust"), "let a = b &lt; c &amp;&amp; d;\nlet e = 1;")
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text, "let a = b < c && d;\nlet e = 1;");
}
