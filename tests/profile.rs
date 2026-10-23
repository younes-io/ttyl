mod common;

use std::time::Instant;

use common::{boot, contains, page, press, profile, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Circle, Log, PeoplePage, Person, RowPage};
use ttyl::sched::Ticket;

/// The first latest commit by someone other than the signed-in @ada.
fn at_other() -> usize {
    page(Log::Latest)
        .rows
        .iter()
        .position(|row| row.commit.author.username != "ada")
        .expect("the fixture has someone else's commit")
}

fn author() -> String {
    page(Log::Latest).rows[at_other()]
        .commit
        .author
        .username
        .clone()
}

fn keys() -> String {
    format!("2{}p", "j".repeat(at_other()))
}

fn ticket(effects: &[Effect], want: impl Fn(&Call) -> bool) -> Ticket {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Run(ticket, call) if want(call) => Some(*ticket),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no such call in {:#?}", runs(effects)))
}

fn person(username: &str) -> Person {
    let Reply::User(profile) = profile(username, false) else {
        unreachable!("profile builds a user reply");
    };
    Person {
        is_followed: Some(false),
        ..profile.person
    }
}

/// Opens someone else's profile and answers its header and commits.
fn on_profile(at: Instant) -> Engine {
    let mut engine = boot(at);
    let effects = press(&mut engine, &keys(), at);
    let user = ticket(&effects, |call| matches!(call, Call::User(_)));
    let commits = ticket(&effects, |call| matches!(call, Call::Commits { .. }));
    engine.update(Event::Reply(user, Ok(profile(&author(), true))), at);
    let rows = page(Log::Latest).rows[..3].to_vec();
    let reply = Reply::Rows(RowPage {
        rows,
        next_cursor: None,
    });
    engine.update(Event::Reply(commits, Ok(reply)), at);
    engine
}

#[test]
fn p_opens_the_selected_authors_profile() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, &keys(), at);
    let calls = runs(&effects);
    assert!(calls.contains(&&Call::User(author())), "{calls:#?}");
    assert!(
        calls.contains(&&Call::Commits {
            username: author(),
            replies: false,
            cursor: None,
        }),
        "{calls:#?}"
    );
    let mut engine = on_profile(at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[4].contains(&author()), "{rows:#?}");
    assert!(rows[4].contains(" following "), "{rows:#?}");
    assert!(contains(&rows, "Joined January 2025"), "{rows:#?}");
    assert!(contains(&rows, "├─ /commits ── replies"), "{rows:#?}");
}

#[test]
fn enter_on_a_profile_commit_opens_its_thread() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    let id = page(Log::Latest).rows[0].commit.id.clone();
    assert!(
        runs(&effects).contains(&&Call::Commit(id)),
        "{:#?}",
        runs(&effects)
    );
}

#[test]
fn tab_walks_the_tabs_and_loads_each_once() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    let replies = tap(&mut engine, KeyCode::Tab, at);
    assert!(
        runs(&replies).contains(&&Call::Commits {
            username: author(),
            replies: true,
            cursor: None,
        }),
        "{:#?}",
        runs(&replies)
    );
    let followers = tap(&mut engine, KeyCode::Tab, at);
    let asked = ticket(&followers, |call| {
        matches!(
            call,
            Call::People {
                circle: Circle::Followers,
                cursor: None,
                ..
            }
        )
    });
    let reply = Reply::People(PeoplePage {
        people: vec![person("linus")],
        next_cursor: None,
    });
    engine.update(Event::Reply(asked, Ok(reply)), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "@linus"), "{rows:#?}");
    assert!(rows[29].contains("f follow"), "{rows:#?}");
    tap(&mut engine, KeyCode::BackTab, at);
    let back = tap(&mut engine, KeyCode::Tab, at);
    assert!(runs(&back).is_empty(), "{:#?}", runs(&back));
}

#[test]
fn f_follows_a_person_in_the_list() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    tap(&mut engine, KeyCode::Tab, at);
    let followers = tap(&mut engine, KeyCode::Tab, at);
    let asked = ticket(&followers, |call| matches!(call, Call::People { .. }));
    let reply = Reply::People(PeoplePage {
        people: vec![person("linus")],
        next_cursor: None,
    });
    engine.update(Event::Reply(asked, Ok(reply)), at);
    press(&mut engine, "f", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[29].contains("following @linus"), "{rows:#?}");
}

#[test]
fn esc_leaves_the_profile_for_the_log() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[1].contains("/latest"), "{rows:#?}");
}

#[test]
fn space_expands_a_folded_commit_and_folds_it_back() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    press(&mut engine, "jj", at);
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "space to expand"
    ));
    press(&mut engine, " ", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, "space to expand"), "{rows:#?}");
    press(&mut engine, " ", at);
    assert!(contains(
        &screen(&mut engine, 100, 30, at),
        "space to expand"
    ));
}
