use devnull::api::{Account, Me, Reply};
use devnull::model::{Entry, Log, LogPage};
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
    page.entries
        .iter()
        .map(|entry| match entry {
            Entry::Commit(commit) => commit.short_hash.clone(),
            Entry::Tombstone(tombstone) => tombstone.id.0.clone(),
        })
        .collect()
}

#[test]
fn a_page_reply_is_read_back_in_order_with_its_suggestions() {
    let mut store = store();
    assert_eq!(*store.slot(Log::Latest), Slot::Loading);
    assert!(store.page(Log::Latest).is_none());

    let latest = page("log-latest");
    let expected = hashes(&latest);
    store.apply(Reply::Page(latest));
    store.apply(Reply::Page(page("log-following")));

    let shown = store.page(Log::Latest).unwrap();
    assert_eq!(hashes(&shown), expected);
    assert_eq!(shown.log, Log::Latest);
    assert!(!shown.is_ranked);

    let following = store.page(Log::Following).unwrap();
    assert!(following.entries.is_empty());
    assert_eq!(following.suggestions.len(), 3);
}

#[test]
fn a_failure_shows_only_where_nothing_loaded() {
    let mut store = store();
    store.fail(Log::Trending, "the site is in maintenance".to_owned());
    assert_eq!(
        *store.slot(Log::Trending),
        Slot::Failed("the site is in maintenance".to_owned())
    );

    store.apply(Reply::Page(page("log-trending")));
    store.fail(Log::Trending, "the site is in maintenance".to_owned());
    assert!(store.page(Log::Trending).is_some());
}

#[test]
fn an_account_reply_moves_the_unread_count() {
    let mut store = store();
    store.apply(Reply::Account(Account {
        me: Me {
            username: "ada".to_owned(),
            name: Some("Ada".to_owned()),
        },
        unread: 3,
    }));
    assert_eq!(store.unread, 3);
    assert_eq!(store.me.name.as_deref(), Some("Ada"));
}
