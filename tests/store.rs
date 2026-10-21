use devnull::api::{Account, Call, Me, Reply};
use devnull::model::{Log, LogPage};
use devnull::store::{Slot, Store};

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
    store.fail(Log::Trending, "the site is in maintenance".to_owned());
    assert!(matches!(
        store.slot(Log::Trending),
        Slot::Failed(message) if message == "the site is in maintenance"
    ));

    store.apply(
        &Call::Page(Log::Trending),
        Reply::Page(page("log-trending")),
    );
    store.fail(Log::Trending, "the site is in maintenance".to_owned());
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
            },
            unread: 3,
        }),
    );
    assert_eq!(store.unread, 3);
    assert_eq!(store.me.name.as_deref(), Some("Ada"));
}

fn ids(store: &Store, log: Log) -> Vec<String> {
    let Slot::Loaded(listing) = store.slot(log) else {
        panic!("{log:?} is loaded");
    };
    listing.ids.iter().map(|id| id.0.clone()).collect()
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
    let before = ids(&store, Log::Latest);
    assert!(before.contains(&overlap));

    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor,
        },
        Reply::Page(older),
    );

    assert_eq!(ids(&store, Log::Latest), before);
    assert!(store.page(Log::Latest).unwrap().next_cursor.is_none());
}

#[test]
fn an_older_page_for_a_listing_a_refresh_replaced_is_dropped() {
    let mut store = store();
    store.apply(&Call::Page(Log::Latest), Reply::Page(page("log-latest")));
    let before = ids(&store, Log::Latest);
    let mut older = page("log-trending");
    older.log = Log::Latest;

    store.apply(
        &Call::Older {
            log: Log::Latest,
            cursor: "a cursor from before the refresh".to_owned(),
        },
        Reply::Page(older),
    );

    assert_eq!(ids(&store, Log::Latest), before);
}
