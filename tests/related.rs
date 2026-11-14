mod common;

use std::time::Instant;

use common::{answer, boot, contains, page, press, profile, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitId, Log, Row, RowPage};
use ttyl::sched::Ticket;

fn ticket(effects: &[Effect], want: impl Fn(&Call) -> bool) -> Ticket {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(ticket, call) if want(call) => Some(*ticket),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no such call in {:#?}", runs(effects)))
}

fn rows_reply(rows: Vec<Row>) -> Reply {
    Reply::Rows(RowPage {
        rows,
        next_cursor: None,
    })
}

fn land(engine: &mut Engine, ticket: Ticket, rows: Vec<Row>, at: Instant) {
    engine.update(Event::Reply(ticket, Ok(rows_reply(rows))), at);
}

/// Grace's two commits, as the answer of what is like the selected one.
fn alike() -> Vec<Row> {
    let rows: Vec<Row> = (page(Log::Latest).rows.into_iter())
        .filter(|row| row.commit.author.handle == "grace")
        .collect();
    assert_eq!(rows.len(), 2);
    rows
}

fn first_following() -> CommitId {
    page(Log::Following).rows[0].commit.id.clone()
}

/// Presses `gr` and says which commit it asked about.
fn related(engine: &mut Engine, at: Instant) -> (Ticket, CommitId) {
    let effects = press(engine, "gr", at);
    let [Effect::Run(ticket, Call::Related(id))] = effects.as_slice() else {
        panic!("gr asks for related commits: {:#?}", runs(&effects));
    };
    (*ticket, id.clone())
}

#[test]
fn gr_lists_the_commits_like_the_selected_one_titled_by_its_hash() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, id) = related(&mut engine, at);
    assert_eq!(id, first_following());
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains(&format!("related {}", id.short_hash())),
        "{rows:#?}"
    );

    land(&mut engine, ticket, alike(), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "today's footgun"), "{rows:#?}");
    assert!(contains(&rows, "the case against OFFSET"), "{rows:#?}");

    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Commit(alike()[0].commit.id.clone())),
        "enter opens the thread: {:#?}",
        runs(&effects)
    );
}

#[test]
fn nothing_alike_says_so() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = related(&mut engine, at);
    land(&mut engine, ticket, Vec::new(), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, "Nothing like this commit yet."),
        "{rows:#?}"
    );
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[1].contains("/following"), "esc leads back: {rows:#?}");
}

#[test]
fn gr_asks_about_the_commit_selected_in_a_thread() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    answer(&mut engine, effects, at);
    let (_, id) = related(&mut engine, at);
    assert_eq!(id, first_following());
}

#[test]
fn gr_asks_about_the_commit_selected_in_starred_and_search_results() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gs", at);
    let starred = ticket(&effects, |call| matches!(call, Call::Starred { .. }));
    land(&mut engine, starred, alike(), at);
    assert_eq!(related(&mut engine, at).1, alike()[0].commit.id);

    let mut engine = boot(at);
    press(&mut engine, "/offsets", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let search = ticket(&effects, |call| matches!(call, Call::SearchCommits(_)));
    land(&mut engine, search, alike(), at);
    press(&mut engine, "j", at);
    assert_eq!(related(&mut engine, at).1, alike()[1].commit.id);
}

#[test]
fn gr_asks_about_the_commit_selected_on_a_profile() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "2jjp", at);
    let user = ticket(&effects, |call| matches!(call, Call::User(_)));
    let commits = ticket(&effects, |call| matches!(call, Call::Commits { .. }));
    engine.update(Event::Reply(user, Ok(profile("grace", true))), at);
    land(&mut engine, commits, alike(), at);
    assert_eq!(related(&mut engine, at).1, alike()[0].commit.id);
}

#[test]
fn gr_from_related_commits_walks_on_and_esc_walks_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, first) = related(&mut engine, at);
    land(&mut engine, ticket, alike(), at);
    let (_, next) = related(&mut engine, at);
    assert_eq!(next, alike()[0].commit.id);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains(&format!("related {}", next.short_hash())),
        "{rows:#?}"
    );
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains(&format!("related {}", first.short_hash())),
        "{rows:#?}"
    );
}

#[test]
fn related_rows_leave_the_reply_line_of_a_card_listed_elsewhere() {
    let at = Instant::now();
    let mut engine = boot(at);
    let reply = "did you keep an index on created_at?";
    assert!(contains(&screen(&mut engine, 100, 30, at), reply));
    let (ticket, _) = related(&mut engine, at);
    let bare = Row {
        reply_preview: None,
        ..page(Log::Following).rows[0].clone()
    };
    land(&mut engine, ticket, vec![bare], at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        contains(&rows, reply),
        "its stored preview stays: {rows:#?}"
    );
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, reply), "the log keeps it: {rows:#?}");
}

#[test]
fn a_search_from_related_commits_leaves_them_to_come_back_to() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _) = related(&mut engine, at);
    land(&mut engine, ticket, alike(), at);
    let (_, second) = related(&mut engine, at);
    press(&mut engine, "/rust", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert_eq!(runs(&effects), [&Call::SearchCommits("rust".to_owned())]);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(
        rows[0].contains(&format!("related {}", second.short_hash())),
        "esc leads back to the related commits: {rows:#?}"
    );
}
