use ttyl::api::{Account, Call, Failure, Me, Reply};
use ttyl::model::{CommitId, CommitPage, Log, LogPage, RepliesPage, Shown};
use ttyl::store::{Branch, Slot, Store};

fn page(name: &str) -> LogPage {
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    LogPage::parse(&json).unwrap()
}

fn store() -> Store {
    Store::new(Account {
        me: Me {
            handle: "ada".to_owned(),
            name: None,
            pasteis_mode: false,
            pasteis_remaining: None,
        },
        unread: 0,
    })
}

fn hashes(page: &LogPage) -> Vec<String> {
    page.rows
        .iter()
        .map(|row| row.commit.short_hash.clone())
        .collect()
}

#[test]
fn a_page_reply_is_read_back_in_order_with_its_suggestions() {
    let mut store = store();
    assert!(matches!(store.slot(Log::Latest), Slot::Loading));
    assert!(store.page(Log::Latest).is_none());

    let latest = page("log-latest");
    let expected = hashes(&latest);
    store.apply(&Call::Page(Log::Latest), Reply::Page(latest));
    store.apply(
        &Call::Page(Log::Following),
        Reply::Page(page("log-following")),
    );

    let shown = store.page(Log::Latest).unwrap();
    assert_eq!(hashes(&shown), expected);
    assert_eq!(shown.log, Log::Latest);
    assert!(!shown.is_ranked);

    let following = store.page(Log::Following).unwrap();
    assert_eq!(following.rows.len(), 3);
    assert_eq!(following.suggestions.len(), 3);
}

#[test]
fn a_failure_shows_only_where_nothing_loaded() {
    let mut store = store();
    store.fail(Log::Trending, Failure::Down);
    assert!(matches!(
        store.slot(Log::Trending),
        Slot::Failed(Failure::Down)
    ));

    store.apply(
        &Call::Page(Log::Trending),
        Reply::Page(page("log-trending")),
    );
    store.fail(Log::Trending, Failure::Down);
    assert!(store.page(Log::Trending).is_some());
}

#[test]
fn an_account_reply_moves_the_unread_count() {
    let mut store = store();
    store.apply(
        &Call::Me,
        Reply::Account(Account {
            me: Me {
                handle: "ada".to_owned(),
                name: Some("Ada".to_owned()),
                pasteis_mode: false,
                pasteis_remaining: None,
            },
            unread: 3,
        }),
    );
    assert_eq!(store.unread, 3);
    assert_eq!(store.me.name.as_deref(), Some("Ada"));
}

#[test]
fn an_older_page_is_appended_without_the_rows_already_listed() {
    let mut store = store();
    let latest = page("log-latest");
    let cursor = latest.next_cursor.clone().unwrap();
    let mut older = page("log-latest");
    older.rows.drain(..10);
    older.next_cursor = None;
    let overlap = older.rows[0].commit.id.0.clone();
    store.apply(&Call::Page(Log::Latest), Reply::Page(latest));
    let before = listed(&store, Log::Latest).0;
    assert!(before.contains(&overlap));

    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor,
        },
        Reply::Page(older),
    );

    assert_eq!(listed(&store, Log::Latest).0, before);
    assert!(store.page(Log::Latest).unwrap().next_cursor.is_none());
}

#[test]
fn an_older_page_for_a_listing_a_refresh_replaced_is_dropped() {
    let mut store = store();
    store.apply(&Call::Page(Log::Latest), Reply::Page(page("log-latest")));
    let before = listed(&store, Log::Latest).0;
    let mut older = page("log-trending");
    older.log = Log::Latest;

    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor: "a cursor from before the refresh".to_owned(),
        },
        Reply::Page(older),
    );

    assert_eq!(listed(&store, Log::Latest).0, before);
}

fn renamed(mut page: LogPage, prefix: &str) -> LogPage {
    for (index, row) in page.rows.iter_mut().enumerate() {
        row.commit.id = CommitId(format!("{prefix}-{index}"));
    }
    page
}

fn listed(store: &Store, log: Log) -> (Vec<String>, Option<String>, u32) {
    let Slot::Loaded(listing) = store.slot(log) else {
        panic!("{log:?} is loaded");
    };
    let ids = listing.ids.iter().map(|id| id.0.clone()).collect();
    (ids, listing.next_cursor.clone(), listing.new_commits)
}

fn latest_and_an_older_page() -> Store {
    let mut store = store();
    let latest = page("log-latest");
    let cursor = latest.next_cursor.clone().unwrap();
    store.apply(&Call::Page(Log::Latest), Reply::Page(latest));
    let mut older = renamed(page("log-latest"), "old");
    older.rows.truncate(3);
    older.next_cursor = Some("after-old".to_owned());
    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor,
        },
        Reply::Page(older),
    );
    store
}

#[test]
fn a_reload_keeps_the_rows_shown_until_it_has_walked_back_as_deep_as_they_went() {
    let mut store = latest_and_an_older_page();
    let before = listed(&store, Log::Latest);
    let mut first = page("log-latest");
    let deleted = first.rows.remove(1).commit.id.0;
    first.next_cursor = Some("after-first".to_owned());

    store.apply(&Call::Page(Log::Latest), Reply::Page(first.clone()));
    assert_eq!(listed(&store, Log::Latest), before, "nothing moves halfway");
    assert_eq!(store.refilling(Log::Latest), Some("after-first"));

    let mut second = renamed(page("log-latest"), "again");
    second.rows.truncate(4);
    second.next_cursor = Some("after-second".to_owned());
    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor: "after-first".to_owned(),
        },
        Reply::Page(second.clone()),
    );

    let (ids, cursor, _) = listed(&store, Log::Latest);
    let expected: Vec<String> = (first.rows.iter().chain(&second.rows))
        .map(|row| row.commit.id.0.clone())
        .collect();
    assert_eq!(ids, expected);
    assert!(
        !ids.contains(&deleted),
        "a commit deleted elsewhere is gone"
    );
    assert_eq!(cursor.as_deref(), Some("after-second"));
    assert_eq!(store.refilling(Log::Latest), None);
}

#[test]
fn a_reload_that_runs_out_of_pages_lands_what_there_is() {
    let mut store = latest_and_an_older_page();
    let mut fresh = renamed(page("log-latest"), "new");
    fresh.rows.truncate(2);
    fresh.next_cursor = None;

    store.apply(&Call::Page(Log::Latest), Reply::Page(fresh));

    let (ids, cursor, _) = listed(&store, Log::Latest);
    assert_eq!(ids, ["new-0", "new-1"]);
    assert_eq!(cursor, None);
}

#[test]
fn a_ranked_log_is_always_replaced() {
    let mut store = store();
    let trending = page("log-trending");
    store.apply(&Call::Page(Log::Trending), Reply::Page(trending.clone()));
    let mut reranked = trending;
    reranked.rows.reverse();
    reranked.rows.truncate(4);
    let expected: Vec<String> = reranked
        .rows
        .iter()
        .map(|row| row.commit.id.0.clone())
        .collect();

    store.apply(&Call::Page(Log::Trending), Reply::Page(reranked));

    assert_eq!(listed(&store, Log::Trending).0, expected);
}

#[test]
fn a_count_of_new_commits_holds_only_for_the_newest_commit_it_was_asked_about() {
    let mut store = latest_and_an_older_page();
    let newest = page("log-latest").rows[0].commit.id.clone();
    let mut counted = page("log-latest");
    counted.new_commits_count = Some(3);
    let since = |id: &CommitId| Call::Since {
        log: Log::Latest,
        since: id.clone(),
    };

    store.apply(
        &since(&CommitId("gone".to_owned())),
        Reply::Page(counted.clone()),
    );
    assert_eq!(listed(&store, Log::Latest).2, 0);
    store.apply(&since(&newest), Reply::Page(counted));
    assert_eq!(listed(&store, Log::Latest).2, 3);
    assert_eq!(
        listed(&store, Log::Latest).0.len(),
        14,
        "the count moves no row"
    );

    let mut fresh = page("log-latest");
    fresh.next_cursor = None;
    store.apply(&Call::Page(Log::Latest), Reply::Page(fresh));
    assert_eq!(listed(&store, Log::Latest).2, 0);
}

#[test]
fn a_page_with_more_replies_than_were_listed_makes_the_thread_ask_again() {
    let mut store = store();
    let latest = page("log-latest");
    let id = latest.rows[0].commit.id.clone();
    store.apply(&Call::Page(Log::Latest), Reply::Page(latest));
    store.no_replies(&id, false);
    let asks = |store: &Store| {
        ttyl::thread::ThreadScreen::new(id.clone(), std::time::Instant::now())
            .wants(store)
            .into_iter()
            .any(|(call, ..)| matches!(call, Call::Replies { id: owner, cursor: None } if owner == id))
    };
    assert!(
        !asks(&store),
        "an empty list just fetched is not asked again"
    );

    let mut answered = page("log-latest");
    answered.rows[0].replies_count += 1;
    store.apply(&Call::Page(Log::Latest), Reply::Page(answered));
    assert!(asks(&store), "a reply that came in since is fetched");

    store.apply(&Call::Page(Log::Latest), Reply::Page(page("log-latest")));
    store.no_replies(&id, false);
    store.apply(&Call::Page(Log::Latest), Reply::Page(page("log-latest")));
    assert!(
        !asks(&store),
        "a refresh with the same count keeps the list"
    );
}

#[test]
fn a_commit_fetched_again_without_its_quote_drops_it() {
    let mut store = store();
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/commit-quote.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let quoting = CommitPage::parse(&json).unwrap();
    let id = quoting.commit.commit.id.clone();
    let call = Call::Commit(id.clone());
    store.apply(&call, Reply::Thread(Box::new(quoting.clone())));
    assert!(store.row(&id).unwrap().quoted_commit.is_some());

    let mut unquoted = quoting;
    unquoted.commit.quoted_commit = None;
    store.apply(&call, Reply::Thread(Box::new(unquoted)));

    assert!(
        store.row(&id).unwrap().quoted_commit.is_none(),
        "a quote deleted elsewhere is gone"
    );
}

fn replies_page(prefix: &str, count: usize, next_cursor: Option<&str>) -> RepliesPage {
    let json = std::fs::read_to_string(format!(
        "{}/fixtures/replies-paged.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut page = RepliesPage::parse(&json).unwrap();
    let template = page.replies[0].clone();
    page.replies = (0..count)
        .map(|index| {
            let mut reply = template.clone();
            if let Shown::Live(reply) = &mut reply {
                reply.commit.id = CommitId(format!("{prefix}-{index}"));
            }
            reply
        })
        .collect();
    page.next_cursor = next_cursor.map(str::to_owned);
    page
}

fn reply_ids(store: &Store, id: &CommitId) -> Vec<String> {
    let Some(Branch::Listed(replies)) = store.replies(id) else {
        panic!("the replies are listed");
    };
    replies.ids.iter().map(|id| id.0.clone()).collect()
}

#[test]
fn a_reloaded_reply_list_swaps_in_once_it_is_as_deep_as_the_one_shown() {
    let mut store = store();
    let owner = CommitId("owner".to_owned());
    let first = |cursor: Option<&str>| Call::Replies {
        id: owner.clone(),
        cursor: cursor.map(str::to_owned),
    };
    store.apply(
        &first(None),
        Reply::Replies(replies_page("a", 3, Some("p2"))),
    );
    store.apply(
        &first(Some("p2")),
        Reply::Replies(replies_page("b", 3, Some("p3"))),
    );
    let before = reply_ids(&store, &owner);
    assert_eq!(before.len(), 6);

    let mut fresh = replies_page("a", 3, Some("p2"));
    fresh.replies.remove(1);
    store.apply(&first(None), Reply::Replies(fresh));
    assert_eq!(reply_ids(&store, &owner), before, "nothing moves halfway");
    assert_eq!(store.refilling_replies(&owner), Some("p2"));

    store.apply(
        &first(Some("p2")),
        Reply::Replies(replies_page("b", 3, Some("p3"))),
    );

    assert_eq!(
        reply_ids(&store, &owner),
        before,
        "five rows are not as deep as six"
    );
    store.apply(
        &first(Some("p3")),
        Reply::Replies(replies_page("c", 1, None)),
    );

    assert_eq!(
        reply_ids(&store, &owner),
        ["a-0", "a-2", "b-0", "b-1", "b-2", "c-0"],
        "a reply deleted elsewhere is gone"
    );
    assert_eq!(store.refilling_replies(&owner), None);
}
