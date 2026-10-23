mod common;

use common::profile;
use ttyl::api::{Account, Call, Me, Reply};
use ttyl::model::{CommitId, Deletion, Focus, Log, LogPage, Row};
use ttyl::sched::CallKey;
use ttyl::store::{Node, Store};

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn store() -> Store {
    let mut store = Store::new(Account {
        me: Me {
            username: "ada".to_owned(),
            name: None,
            pasteis_mode: false,
            pasteis_remaining: None,
        },
        unread: 0,
    });
    let latest = LogPage::parse(&fixture("log-latest")).unwrap();
    store.apply(&Call::Page(Log::Latest), Reply::Page(latest));
    store
}

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

fn shown(store: &Store) -> (bool, u32) {
    let Some(Node::Live(row)) = store.node(&grace()) else {
        panic!("grace's commit is live");
    };
    let in_log = store
        .page(Log::Latest)
        .unwrap()
        .rows
        .into_iter()
        .find(|row| row.commit.id == grace())
        .unwrap();
    assert_eq!(
        (in_log.starred, in_log.stars_count),
        (row.starred, row.stars_count)
    );
    (row.starred, row.stars_count)
}

fn answer(starred: bool, stars_count: u32) -> Reply {
    let json: serde_json::Value = serde_json::from_str(&fixture("log-latest")).unwrap();
    let mut card = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == GRACE)
        .unwrap()
        .clone();
    card["starred"] = starred.into();
    card["stars_count"] = stars_count.into();
    let focus: Focus = serde_json::from_value(card).unwrap();
    Reply::Commit(Box::new(focus))
}

fn star(on: bool) -> Call {
    Call::Star { id: grace(), on }
}

fn idle(_: &CallKey) -> bool {
    false
}

#[test]
fn a_star_shows_at_once_and_its_answer_does_not_count_it_twice() {
    let mut store = store();
    assert_eq!(shown(&store), (false, 27));

    assert_eq!(store.star(&grace()), Some(true));
    assert_eq!(shown(&store), (true, 28));
    assert_eq!(store.writes(idle), vec![star(true)]);

    store.apply(&star(true), answer(true, 28));
    assert_eq!(shown(&store), (true, 28));
    assert_eq!(store.writes(idle), Vec::new());
}

#[test]
fn presses_while_a_star_is_out_end_where_the_last_press_left_it() {
    let mut store = store();
    store.star(&grace());
    assert_eq!(store.writes(idle), vec![star(true)]);
    let flying = |key: &CallKey| *key == CallKey::Star(grace());

    store.star(&grace());
    store.star(&grace());
    store.star(&grace());
    assert_eq!(shown(&store), (false, 27));
    assert_eq!(store.writes(flying), Vec::new());

    store.apply(&star(true), answer(true, 28));
    assert_eq!(shown(&store), (false, 27));
    assert_eq!(store.writes(idle), vec![star(false)]);

    store.apply(&star(false), answer(false, 27));
    assert_eq!(shown(&store), (false, 27));
    assert_eq!(store.writes(idle), Vec::new());
}

#[test]
fn a_refused_star_goes_back_to_what_the_server_has() {
    let mut store = store();
    store.star(&grace());
    store.writes(idle);
    store.refused(&star(true));
    assert_eq!(shown(&store), (false, 27));
    assert_eq!(store.writes(idle), Vec::new());
}

#[test]
fn a_star_with_no_call_out_is_sent_until_the_server_agrees() {
    let mut store = store();
    store.star(&grace());
    assert_eq!(store.writes(idle), vec![star(true)]);
    assert_eq!(shown(&store), (true, 28));
    assert_eq!(store.writes(idle), vec![star(true)]);
}

#[test]
fn a_settled_star_does_not_undo_a_change_made_elsewhere() {
    let mut store = store();
    store.star(&grace());
    store.writes(idle);
    store.apply(&star(true), answer(true, 28));
    store.writes(idle);

    let mut page = LogPage::parse(&fixture("log-latest")).unwrap();
    let row: &mut Row = page
        .rows
        .iter_mut()
        .find(|row| row.commit.id == grace())
        .unwrap();
    row.starred = false;
    row.stars_count = 30;
    store.apply(&Call::Page(Log::Latest), Reply::Page(page));
    assert_eq!(shown(&store), (false, 30));
    assert_eq!(store.writes(idle), Vec::new());
}

#[test]
fn a_follow_with_no_known_state_asks_the_server_first() {
    let mut store = store();
    assert_eq!(store.follow("grace"), None);
    assert_eq!(store.writes(idle), vec![Call::User("grace".to_owned())]);

    store.apply(&Call::User("grace".to_owned()), profile("grace", false));
    assert_eq!(store.following("grace"), Some(true));
    let follow = Call::Follow {
        username: "grace".to_owned(),
        on: true,
    };
    assert_eq!(store.writes(idle), vec![follow.clone()]);

    store.apply(&follow, Reply::Done);
    assert_eq!(store.writes(idle), Vec::new());
    assert_eq!(store.follow("grace"), Some(false));
}

#[test]
fn a_refused_follow_goes_back_to_what_the_server_has() {
    let mut store = store();
    store.follow("grace");
    store.apply(&Call::User("grace".to_owned()), profile("grace", true));
    assert_eq!(store.following("grace"), Some(false));
    let unfollow = Call::Follow {
        username: "grace".to_owned(),
        on: false,
    };
    assert_eq!(store.writes(idle), vec![unfollow.clone()]);
    store.refused(&unfollow);
    assert_eq!(store.following("grace"), Some(true));
    assert_eq!(store.writes(idle), Vec::new());
}

#[test]
fn a_deleted_commit_shows_as_gone_everywhere_once_the_server_agrees() {
    let mut store = store();
    let deletion = |store: &Store| {
        store
            .page(Log::Latest)
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.commit.id == grace())
            .unwrap()
            .deletion
    };
    let before = store.generation();
    store.start_delete(&grace());
    assert!(store.generation() > before);
    assert_eq!(deletion(&store), Some(Deletion::Running));
    assert!(
        matches!(store.node(&grace()), Some(Node::Live(row)) if row.deletion == Some(Deletion::Running))
    );

    store.apply(&Call::Delete(grace()), Reply::Done);
    assert_eq!(deletion(&store), Some(Deletion::Done));
    assert!(matches!(store.node(&grace()), Some(Node::Gone(_))));
}

#[test]
fn a_refused_delete_leaves_the_commit_as_it_was() {
    let mut store = store();
    store.start_delete(&grace());
    store.refused(&Call::Delete(grace()));
    assert!(matches!(store.node(&grace()), Some(Node::Live(row)) if row.deletion.is_none()));
}
