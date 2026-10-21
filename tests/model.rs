use devnull::model::{Lane, Log, LogPage, Row, Segment, Shown, Status, Suggestion};

fn page(log: &str) -> LogPage {
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/log-{log}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    LogPage::parse(&json).expect("fixture parses")
}

fn row<'a>(page: &'a LogPage, short_hash: &str) -> &'a Row {
    page.rows
        .iter()
        .find(|row| row.commit.short_hash == short_hash)
        .unwrap_or_else(|| panic!("{short_hash} is in the fixture"))
}

#[test]
fn log_envelope_carries_rows_and_the_cursor_from_meta() {
    let page = page("latest");
    assert_eq!(page.log, Log::Latest);
    assert!(!page.is_ranked);
    assert_eq!(page.rows.len(), 11);
    assert_eq!(page.new_commits_count, None);
    assert!(page.suggestions.is_empty());
    assert!(page.next_cursor.as_deref().is_some_and(|c| !c.is_empty()));

    let trending = self::page("trending");
    assert!(trending.is_ranked);
    assert_eq!(trending.next_cursor, None);
}

#[test]
fn a_plain_row_carries_its_card_data() {
    let page = page("latest");
    let row = row(&page, "e4f5a61");
    assert_eq!(row.commit.author.name, "Ada");
    assert_eq!(row.commit.author.lane, Lane::Palette(0));
    assert_eq!(row.commit.status, Status::Approved);
    assert_eq!(
        (row.stars_count, row.starred, row.replies_count),
        (12, true, 3)
    );
    assert_eq!(
        row.commit.body_segments[1],
        Segment::Code {
            code: "cursorPaginate(20)".into()
        }
    );
    assert!(row.commit.preview_segments.is_none());
    assert!(row.media.link_preview.is_none() && row.media.images.is_empty());
}

#[test]
fn lanes_parse_as_palette_index_or_custom_hex() {
    let page = page("latest");
    assert_eq!(
        row(&page, "0000201").commit.author.lane,
        Lane::Custom {
            r: 0x70,
            g: 0x48,
            b: 0xe8
        }
    );
    assert!(matches!(
        &row(&page, "0000201").commit.body_segments[1],
        Segment::Block { language: Some(language), .. } if language == "php"
    ));
}

#[test]
fn only_the_long_commit_carries_a_preview() {
    let page = page("latest");
    let folded: Vec<_> = page
        .rows
        .iter()
        .filter(|row| row.commit.preview_segments.is_some())
        .map(|row| row.commit.short_hash.as_str())
        .collect();
    assert_eq!(folded, ["0000401"]);
}

#[test]
fn your_own_commits_carry_their_review_status() {
    let page = page("latest");
    assert_eq!(row(&page, "e4f5a62").commit.status, Status::Pending);
    assert_eq!(row(&page, "0000901").commit.status, Status::Rejected);
}

#[test]
fn a_quote_is_a_live_commit_with_its_media() {
    let page = page("latest");
    let row = row(&page, "e4f5a62");
    let Some(Shown::Live(quote)) = &row.quoted_commit else {
        panic!("a live quote: {:?}", row.quoted_commit);
    };
    assert_eq!(quote.commit.short_hash, "0000401");
    assert_eq!(row.commit.quoted_commit_id.as_ref(), Some(&quote.commit.id));
}

#[test]
fn a_deleted_quote_is_a_tombstone_with_the_servers_short_hash() {
    let page = page("latest");
    let row = row(&page, "0000a01");
    let Some(Shown::Deleted(tombstone)) = &row.quoted_commit else {
        panic!("a tombstone: {:?}", row.quoted_commit);
    };
    assert_eq!(tombstone.replies_count, None);
    assert_eq!(tombstone.short_hash().len(), 7);
    assert!(
        tombstone
            .id
            .0
            .replace('-', "")
            .ends_with(&tombstone.short_hash()),
        "{tombstone:?}"
    );
}

#[test]
fn a_link_preview_keeps_its_title_and_url() {
    let page = page("latest");
    let preview = row(&page, "0000601")
        .media
        .link_preview
        .as_ref()
        .expect("link preview");
    assert_eq!(
        preview.title.as_deref(),
        Some("We need tool support for keyset pagination")
    );
    assert!(preview.url.starts_with("https://use-the-index-luke.com/"));
    assert!(!preview.is_pending);
}

#[test]
fn images_and_polls_come_through() {
    let page = page("latest");
    assert_eq!(row(&page, "0000701").media.images.len(), 2);
    let poll = row(&page, "0000801").media.poll.as_ref().expect("poll");
    assert_eq!(poll.options.len(), 2);
    assert!(!poll.is_closed && poll.closes_at.is_some());
    assert_eq!(poll.total_votes, None);
}

#[test]
fn the_reply_preview_holds_the_newest_reply_or_its_tombstone() {
    let page = page("latest");
    let first = &row(&page, "e4f5a61").reply_preview;
    assert!(
        matches!(first.data.as_slice(), [Shown::Live(reply)] if reply.commit.author.username == "grace")
    );
    assert!(first.next_cursor.is_some());

    let deleted = &row(&page, "0000201").reply_preview;
    assert!(matches!(
        deleted.data.as_slice(),
        [Shown::Deleted(tombstone)] if tombstone.replies_count == Some(0)
    ));

    let whole = &row(&page, "0000501").reply_preview;
    assert_eq!(whole.data.len(), 1);
    assert_eq!(whole.next_cursor, None);

    assert!(row(&page, "0000301").reply_preview.data.is_empty());
}

#[test]
fn following_suggestions_parse_without_the_row_only_fields() {
    let page = page("following");
    let authors: Vec<_> = page
        .suggestions
        .iter()
        .map(|suggestion| suggestion.commit.author.username.as_str())
        .collect();
    assert_eq!(authors, ["grace", "linus", "margaret"]);
    assert!(
        page.rows
            .iter()
            .all(|row| row.commit.author.username == "ada")
    );
}

#[test]
fn a_tombstone_in_a_row_slot_is_refused() {
    let json = r#"{"data":[{"id":"a","parent_id":null,"deleted_at":"2026-10-21T10:00:00Z"}],"meta":{"next_cursor":null},"log":"latest","is_ranked":false,"suggestions":[]}"#;
    assert!(LogPage::parse(json).is_err());
}

#[test]
fn lane_outside_palette_is_rejected() {
    for lane in ["6", "\"#zzzzzz\"", "\"7048e8\""] {
        let json = format!(
            r#"{{"id":"u","username":"x","name":"X","avatar_url":null,"html_url":"h","lane":{lane}}}"#
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
    let err = serde_json::from_str::<Suggestion>(json)
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing field"), "{err}");
}
