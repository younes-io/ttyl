use ttyl::api::{Account, Call, Failure, Me, Reply};
use ttyl::model::{CommitId, Log, LogPage};
use ttyl::store::{Slot, Store};

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
            username: "ada".to_owned(),
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
                username: "ada".to_owned(),
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
fn a_first_page_that_overlaps_lands_on_top_and_keeps_the_older_pages() {
    let mut store = latest_and_an_older_page();
    let (before, _, _) = listed(&store, Log::Latest);
    let mut first = renamed(page("log-latest"), "new");
    first.rows.truncate(2);
    first
        .rows
        .extend(page("log-latest").rows.into_iter().take(9));
    first.next_cursor = Some("after-first".to_owned());

    store.apply(&Call::Page(Log::Latest), Reply::Page(first));

    let (ids, cursor, _) = listed(&store, Log::Latest);
    assert_eq!(ids[..2], ["new-0", "new-1"]);
    assert_eq!(ids[2..], before[..]);
    assert_eq!(cursor.as_deref(), Some("after-old"));
}

#[test]
fn a_first_page_with_no_listed_commit_replaces_the_log() {
    let mut store = latest_and_an_older_page();
    let fresh = renamed(page("log-latest"), "new");
    let expected: Vec<String> = fresh
        .rows
        .iter()
        .map(|row| row.commit.id.0.clone())
        .collect();

    store.apply(&Call::Page(Log::Latest), Reply::Page(fresh));

    let (ids, cursor, _) = listed(&store, Log::Latest);
    assert_eq!(ids, expected);
    assert_eq!(cursor, page("log-latest").next_cursor);
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

    store.apply(&Call::Page(Log::Latest), Reply::Page(page("log-latest")));
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
