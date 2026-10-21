use devnull::model::{Entry, Lane, Log, LogPage, Segment, Status};

fn latest() -> LogPage {
    LogPage::parse(include_str!("../fixtures/log-latest.json")).expect("fixture parses")
}

fn commits(page: &LogPage) -> Vec<&devnull::model::Commit> {
    page.entries
        .iter()
        .map(|entry| match entry {
            Entry::Commit(commit) => commit.as_ref(),
            Entry::Tombstone(t) => panic!("unexpected tombstone {:?}", t.id),
        })
        .collect()
}

#[test]
fn log_envelope_carries_page_and_cursor() {
    let page = latest();
    assert_eq!(page.log, Log::Latest);
    assert!(!page.is_ranked);
    assert_eq!(page.entries.len(), 6);
    assert_eq!(page.new_commits_count, None);
    assert!(page.suggestions.is_empty());
    assert_eq!(
        page.next_cursor.as_deref(),
        Some("eyJjcmVhdGVkX2F0IjoiMjAyNi0xMC0yMCAxNzowMzowMCJ9")
    );
}

#[test]
fn lanes_parse_as_palette_index_or_custom_hex() {
    let page = latest();
    let commits = commits(&page);
    assert_eq!(commits[0].author.lane, Lane::Palette(0));
    assert_eq!(
        commits[2].author.lane,
        Lane::Custom {
            r: 0x70,
            g: 0x48,
            b: 0xe8
        }
    );
}

#[test]
fn segments_keep_prose_inline_code_and_blocks_apart() {
    let page = latest();
    let commits = commits(&page);
    assert_eq!(
        commits[0].body_segments[1],
        Segment::Code {
            code: "cursorPaginate(20)".into()
        }
    );
    assert!(
        matches!(&commits[2].body_segments[1], Segment::Block { language: Some(l), .. } if l == "php")
    );
    assert_eq!(
        commits[5].body_segments[1],
        Segment::Block {
            language: None,
            html: "x\ny".into()
        }
    );
}

#[test]
fn long_commit_is_folded_and_short_ones_are_not() {
    let page = latest();
    let commits = commits(&page);
    assert!(commits[4].preview_segments.is_some());
    assert!(
        commits
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || c.preview_segments.is_none())
    );
}

#[test]
fn status_and_reply_preview_come_through() {
    let page = latest();
    let commits = commits(&page);
    assert_eq!(commits[3].status, Status::Pending);
    let preview = commits[0].reply_preview.as_ref().expect("reply preview");
    assert_eq!(preview.data.len(), 1);
    assert!(preview.next_cursor.is_some());
}

#[test]
fn deleted_commit_with_replies_parses_as_tombstone() {
    let json =
        r#"{"id":"a","parent_id":"b","deleted_at":"2026-10-21T10:00:00Z","replies_count":2}"#;
    let entry: Entry = serde_json::from_str(json).unwrap();
    assert!(matches!(entry, Entry::Tombstone(t) if t.replies_count == Some(2)));
}

#[test]
fn lane_outside_palette_is_rejected() {
    for lane in ["6", "\"#zzzzzz\"", "\"7048e8\""] {
        let json = format!(
            r#"{{"id":"u","username":"x","name":null,"avatar_url":null,"html_url":"h","lane":{lane}}}"#
        );
        assert!(
            serde_json::from_str::<devnull::model::User>(&json).is_err(),
            "accepted lane {lane}"
        );
    }
}

#[test]
fn malformed_commit_reports_the_missing_field() {
    let json = r#"{"id":"a","deleted_at":null}"#;
    let err = serde_json::from_str::<Entry>(json).unwrap_err().to_string();
    assert!(err.contains("missing field"), "{err}");
}
