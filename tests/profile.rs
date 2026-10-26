mod common;

use std::time::Instant;

use common::{boot, contains, help_entry, page, press, profile, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Circle, Level, Log, PeoplePage, Person, Profile, RowPage, Streaks};
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
    on_profile_with(at, |_| {})
}

/// Opens someone else's profile, answering its header as `edit` leaves it.
fn on_profile_with(at: Instant, edit: impl FnOnce(&mut Profile)) -> Engine {
    let mut engine = boot(at);
    let effects = press(&mut engine, &keys(), at);
    let user = ticket(&effects, |call| matches!(call, Call::User(_)));
    let commits = ticket(&effects, |call| matches!(call, Call::Commits { .. }));
    let Reply::User(mut header) = profile(&author(), true) else {
        unreachable!("profile builds a user reply");
    };
    edit(&mut header);
    engine.update(Event::Reply(user, Ok(Reply::User(header))), at);
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

fn opened(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Open(url) => Some(url.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn their_pinned_commit_comes_first_and_opens_like_any_other() {
    let at = Instant::now();
    let pinned = page(Log::Latest).rows[5].clone();
    let id = pinned.commit.id.clone();
    let mut engine = on_profile_with(at, |header| header.pinned = Some(pinned));
    let rows = screen(&mut engine, 100, 40, at);
    let label = rows.iter().position(|row| row.contains("┆ pinned"));
    let day = rows
        .iter()
        .position(|row| row.contains("┆ ") && !row.contains("pinned"));
    assert!(label.is_some() && label < day, "{rows:#?}");
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Commit(id)),
        "{:#?}",
        runs(&effects)
    );
}

#[test]
fn link_keys_open_where_they_are_and_show_beside_each_link() {
    let at = Instant::now();
    let mut engine = on_profile_with(at, |header| {
        header.person.website = Some("ada.dev".to_owned());
    });
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "ada.dev gw"), "{rows:#?}");
    assert!(
        contains(&rows, &format!("github.com/{} gh", author())),
        "{rows:#?}"
    );
    assert_eq!(opened(&press(&mut engine, "gw", at)), ["https://ada.dev"]);
    let github = format!("https://github.com/{}", author());
    assert_eq!(opened(&press(&mut engine, "gh", at)), [github.as_str()]);
    let page = format!("https://dev-null.sh/{}", author());
    assert_eq!(opened(&press(&mut engine, "gd", at)), [page.as_str()]);
}

#[test]
fn a_link_they_lack_is_dimmed_in_help_and_does_nothing() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    assert_eq!(
        help_entry(&mut engine, at, "open their website"),
        Some(false)
    );
    assert_eq!(help_entry(&mut engine, at, "open their GitHub"), Some(true));
    assert!(opened(&press(&mut engine, "gw", at)).is_empty());
}

/// Gives them a year of commits, streaks and a rank, as the API sends them.
fn standing(profile: &mut Profile) {
    let first = chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
    let buckets: Vec<_> = (0..365)
        .map(|day| {
            serde_json::json!({
                "date": (first + chrono::Days::new(day)).to_string(),
                "count": day % 5,
            })
        })
        .collect();
    let activity = serde_json::json!({ "scale": "year", "buckets": buckets });
    profile.activity = Some(serde_json::from_value(activity).unwrap());
    profile.streaks = Some(Streaks {
        longest: 12,
        current: 3,
    });
    profile.level = Some(Level {
        level: 4,
        rank: "daemon".into(),
        xp: 1200,
        next_level_xp: Some(2000),
        level_progress: 40,
    });
}

#[test]
fn their_rank_activity_and_streaks_show_under_their_name() {
    let at = Instant::now();
    let mut engine = on_profile_with(at, standing);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "Lv. 4  daemon"), "{rows:#?}");
    assert!(contains(&rows, "commits in the last year"), "{rows:#?}");
    assert!(contains(&rows, "longest streak 12 days"), "{rows:#?}");
    assert!(contains(&rows, "● current 3"), "{rows:#?}");
    assert!(contains(&rows, "████"), "{rows:#?}");
    assert!(!contains(&rows, " xp"), "only you see your xp {rows:#?}");
}

#[test]
fn a_short_terminal_drops_the_graph_and_keeps_the_commits() {
    let at = Instant::now();
    let mut engine = on_profile_with(at, standing);
    let rows = screen(&mut engine, 100, 24, at);
    assert!(!contains(&rows, "████"), "{rows:#?}");
    assert!(contains(&rows, "Lv. 4  daemon"), "{rows:#?}");
    assert!(contains(&rows, "├─ /commits ── replies"), "{rows:#?}");
}

#[test]
fn a_profile_without_standing_shows_none() {
    let at = Instant::now();
    let mut engine = on_profile(at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "Lv."), "{rows:#?}");
    assert!(!contains(&rows, "streak"), "{rows:#?}");
}

#[test]
fn your_own_profile_shows_how_far_you_are_from_the_next_rank() {
    let at = Instant::now();
    let mut engine = boot(at);
    let mine = page(Log::Latest)
        .rows
        .iter()
        .position(|row| row.commit.author.username == "ada")
        .expect("the fixture has a commit by @ada");
    let effects = press(&mut engine, &format!("2{}p", "j".repeat(mine)), at);
    let user = ticket(&effects, |call| matches!(call, Call::User(_)));
    let Reply::User(mut header) = profile("ada", false) else {
        unreachable!("profile builds a user reply");
    };
    standing(&mut header);
    engine.update(Event::Reply(user, Ok(Reply::User(header))), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(contains(&rows, "▰▰▰▰▱▱▱▱▱▱  1,200/2,000 xp"), "{rows:#?}");
}
