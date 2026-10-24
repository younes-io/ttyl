use ttyl::model::{
    CommitPage, Filter, GroupPage, Kind, Lane, Log, LogPage, NotificationPage, RepliesPage, Row,
    Segment, Shown, Status, Suggestion,
};

fn page(log: &str) -> LogPage {
    LogPage::parse(&fixture(&format!("log-{log}"))).expect("fixture parses")
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
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
            serde_json::from_str::<ttyl::model::User>(&json).is_err(),
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

#[test]
fn a_commit_page_carries_its_ancestors_root_first_with_tombstones() {
    let page = CommitPage::parse(&fixture("commit-thread")).expect("fixture parses");
    assert_eq!(page.commit.commit.author.username, "grace");
    assert_eq!(page.replies_count, 0);
    let [Shown::Live(root), Shown::Deleted(gone)] = page.ancestors.as_slice() else {
        panic!("a root and a tombstone: {:?}", page.ancestors);
    };
    assert_eq!(root.commit.short_hash, "e4f5a61");
    assert_eq!(root.commit.parent_id, None);
    assert_eq!(gone.parent_id.as_ref(), Some(&root.commit.id));
    assert_eq!(page.commit.commit.parent_id.as_ref(), Some(&gone.id));
    assert!(page.earlier.is_none());
}

#[test]
fn a_cut_chain_keeps_twenty_ancestors_and_the_parent_of_the_first() {
    let page = CommitPage::parse(&fixture("commit-cut")).expect("fixture parses");
    assert_eq!(page.ancestors.len(), 20);
    let Some(Shown::Live(earlier)) = &page.earlier else {
        panic!("the parent of the first ancestor: {:?}", page.earlier);
    };
    let Shown::Live(first) = &page.ancestors[0] else {
        panic!("a live first ancestor");
    };
    assert_eq!(first.commit.parent_id.as_ref(), Some(&earlier.commit.id));
    assert_eq!(earlier.commit.body, "step 4 of 25 down this chain.");
}

#[test]
fn the_commit_page_embeds_the_commit_it_quotes() {
    let page = CommitPage::parse(&fixture("commit-quote")).expect("fixture parses");
    let Some(Shown::Live(quote)) = &page.commit.quoted_commit else {
        panic!("a live quote: {:?}", page.commit.quoted_commit);
    };
    assert_eq!(quote.commit.short_hash, "e4f5a61");
    assert_eq!(page.commit.commit.parent_id, None);
    assert!(page.ancestors.is_empty());
}

#[test]
fn a_replies_page_lists_replies_quotes_and_tombstones_oldest_first() {
    let page = RepliesPage::parse(&fixture("replies-thread")).expect("fixture parses");
    assert_eq!(page.replies.len(), 12);
    assert_eq!(page.next_cursor, None);
    let live: Vec<_> = page
        .replies
        .iter()
        .filter_map(|reply| match reply {
            Shown::Live(reply) => Some(reply),
            Shown::Deleted(_) => None,
        })
        .collect();
    assert!(
        live.windows(2)
            .all(|pair| pair[0].commit.created_at <= pair[1].commit.created_at)
    );
    let quote = live
        .iter()
        .find(|reply| reply.commit.parent_id.is_none())
        .expect("a quote");
    assert_eq!(
        quote.commit.quoted_commit_id.as_ref(),
        live[0].commit.parent_id.as_ref()
    );
    assert!(page.replies.iter().any(
        |reply| matches!(reply, Shown::Deleted(tombstone) if tombstone.replies_count == Some(1))
    ));
    assert_eq!(live[0].replies_count, 1);
}

#[test]
fn a_replies_page_with_more_carries_the_cursor_from_meta() {
    let page = RepliesPage::parse(&fixture("replies-paged")).expect("fixture parses");
    assert_eq!(page.replies.len(), 3);
    assert!(page.next_cursor.is_some_and(|cursor| !cursor.is_empty()));
}

#[test]
fn a_notifications_page_reads_each_group_and_the_moment_it_marked_them_read() {
    let page = NotificationPage::parse(&fixture("notifications"), Filter::All).expect("parses");
    assert_eq!(page.filter, Filter::All);
    assert_eq!(page.read_at.to_rfc3339(), "2026-10-21T09:00:00+00:00");
    assert_eq!(page.next_cursor.as_deref(), Some("bm90aWZpY2F0aW9ucy0y"));
    let kinds: Vec<Kind> = page.groups.iter().map(|group| group.event.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Replied,
            Kind::Starred,
            Kind::Followed,
            Kind::Quoted,
            Kind::Mentioned,
            Kind::Starred,
            Kind::Approved,
            Kind::Rejected,
            Kind::LeveledUp,
        ]
    );

    let stars = &page.groups[1];
    assert_eq!((stars.event_count, stars.actors_count), (5, 5));
    assert_eq!(stars.event.actor.name, "Grace");
    assert!(
        matches!(&stars.event.commit, Some(Shown::Live(commit)) if commit.short_hash == "e4f5a61")
    );
    assert_eq!(stars.event.read_at, Some(page.read_at));
    assert_eq!(page.groups[0].event.read_at, None);

    let rejected = &page.groups[7].event;
    assert_eq!(rejected.reason.as_deref(), Some("Links to a paid course."));
    assert!(page.groups[8].event.commit.is_none());
}

#[test]
fn a_group_page_lists_its_events_without_counts() {
    let page = GroupPage::parse(&fixture("notifications-group")).expect("parses");
    let names: Vec<&str> = page
        .events
        .iter()
        .map(|event| event.actor.name.as_str())
        .collect();
    assert_eq!(names, ["Grace", "Ken", "Linus", "Margaret", "Ada"]);
    assert!(page.events.iter().all(|event| event.kind == Kind::Starred));
    assert_eq!(page.next_cursor, None);
}

#[test]
fn a_dark_custom_lane_is_lifted_until_it_reads_and_a_bright_one_is_kept() {
    use ratatui::style::Color;
    use ttyl::model::Lane;
    use ttyl::theme;
    for (r, g, b) in [
        (0, 0, 0),
        (0x0b, 0x2a, 0x5c),
        (0x1e, 0x5b, 0x1e),
        (0, 0, 0xff),
        (0x30, 0x30, 0x30),
    ] {
        let lifted = theme::lane(Lane::Custom { r, g, b });
        assert!(
            theme::contrast(lifted) >= 4.5,
            "#{r:02x}{g:02x}{b:02x} became {lifted:?}"
        );
    }
    assert_eq!(
        theme::lane(Lane::Custom {
            r: 0x3b,
            g: 0xc9,
            b: 0xdb
        }),
        Color::Rgb(0x3b, 0xc9, 0xdb)
    );
}
