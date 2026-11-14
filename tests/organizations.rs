mod common;

use std::time::Instant;

use common::{boot_as, contains, log_of, press, runs, screen, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use ttyl::api::{Account, Call, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{
    Lane, Log, LogPage, OrganizationList, OrganizationPage, PeoplePage, RowPage, Who,
};
use ttyl::sched::Ticket;

fn acme() -> serde_json::Value {
    json!({
        "type": "organization",
        "id": "o-acme",
        "handle": "acme",
        "name": "Acme",
        "avatar_url": null,
        "bio": "We ship small tools.",
        "location": "Lisbon",
        "website": "acme.dev",
        "github_url": "https://github.com/acme",
        "html_url": "https://dev-null.sh/acme",
        "lane": null,
    })
}

/// Every log as one commit by @acme, which has no lane of its own.
fn by_acme(_: Log) -> LogPage {
    log_of(&[json!({ "author": acme() })])
}

/// @ada, who is a member of @acme and @zeta.
fn member() -> Account {
    serde_json::from_value(json!({
        "data": {
            "handle": "ada",
            "name": "Ada",
            "organizations": [
                { "type": "organization", "id": "o-acme", "role": "admin", "handle": "acme" },
                { "type": "organization", "id": "o-zeta", "role": "member", "handle": "zeta" },
            ],
        },
        "unread_notifications_count": 0,
    }))
    .unwrap()
}

fn header(is_following: bool) -> Reply {
    let mut data = acme();
    data["followers_count"] = json!(1204);
    data["admins"] = json!([{
        "id": "u-grace",
        "handle": "grace",
        "name": "Grace",
        "avatar_url": null,
        "html_url": "https://dev-null.sh/grace",
        "lane": 3,
    }]);
    let json = json!({
        "data": data,
        "is_following": is_following,
        "can_claim": false,
        "settings_url": null,
    });
    Reply::Organization(Box::new(
        OrganizationPage::parse(&json.to_string()).unwrap(),
    ))
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

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

/// The footer, without the spinner a write in flight adds.
fn footer(engine: &mut Engine, at: Instant) -> String {
    screen(engine, 100, 30, at)[29]
        .trim_end_matches(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
        .trim_end()
        .to_owned()
}

/// Opens @acme's page from its commit in latest, answering its header and an empty commit list.
fn on_acme(at: Instant, is_following: bool) -> Engine {
    let mut engine = boot_as(at, member(), by_acme);
    let effects = press(&mut engine, "2p", at);
    let page = ticket(&effects, |call| matches!(call, Call::Organization(_)));
    let commits = ticket(&effects, |call| {
        matches!(call, Call::OrganizationCommits { .. })
    });
    engine.update(Event::Reply(page, Ok(header(is_following))), at);
    let empty = Reply::Rows(RowPage {
        rows: Vec::new(),
        next_cursor: None,
    });
    engine.update(Event::Reply(commits, Ok(empty)), at);
    engine
}

#[test]
fn an_organization_without_a_lane_reads_in_the_sites_blue() {
    let page = by_acme(Log::Latest);
    let author = &page.rows[0].commit.author;
    assert_eq!(author.lane, Lane::Site);
    assert_eq!(author.who(), Who::Organization("acme".to_owned()));
}

#[test]
fn an_organizations_commit_says_so_on_its_card() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    press(&mut engine, "2", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "Acme @acme · org ·"), "{rows:#?}");
}

#[test]
fn p_on_an_organizations_commit_opens_its_page() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    let effects = press(&mut engine, "2p", at);
    let calls = runs(&effects);
    assert!(
        calls.contains(&&Call::Organization("acme".to_owned())),
        "{calls:#?}"
    );
    assert!(
        calls.contains(&&Call::OrganizationCommits {
            handle: "acme".to_owned(),
            mentions: false,
            cursor: None,
        }),
        "{calls:#?}"
    );
    assert!(
        !calls.iter().any(|call| matches!(call, Call::User(_))),
        "an organization has no person to ask for: {calls:#?}"
    );

    let mut engine = on_acme(at, false);
    let rows = screen(&mut engine, 100, 30, at);
    for shown in [
        "Acme  @acme",
        "We ship small tools.",
        "Lisbon",
        "admins @grace",
        "acme.dev",
        "1,204  followers",
        "─ /commits ── mentions",
        "@acme has no commits yet.",
    ] {
        assert!(contains(&rows, shown), "{shown}: {rows:#?}");
    }
    assert!(
        !contains(&rows, "followers ──"),
        "no people tabs: {rows:#?}"
    );
    assert!(rows[3].ends_with(" org"), "the org chip: {rows:#?}");
}

#[test]
fn tab_walks_commits_and_mentions_only() {
    let at = Instant::now();
    let mut engine = on_acme(at, false);
    let mentions = tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(
        runs(&mentions),
        [&Call::OrganizationCommits {
            handle: "acme".to_owned(),
            mentions: true,
            cursor: None,
        }]
    );
    let asked = ticket(&mentions, |_| true);
    let rows = by_acme(Log::Latest).rows;
    engine.update(
        Event::Reply(
            asked,
            Ok(Reply::Rows(RowPage {
                rows,
                next_cursor: None,
            })),
        ),
        at,
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "─ commits ── /mentions"), "{rows:#?}");

    let back = tap(&mut engine, KeyCode::Tab, at);
    assert!(runs(&back).is_empty(), "{:#?}", runs(&back));
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, "─ /commits ── mentions"), "{rows:#?}");
}

#[test]
fn f_on_an_organizations_page_follows_it_and_again_unfollows() {
    let at = Instant::now();
    let mut engine = on_acme(at, false);
    let effects = press(&mut engine, "f", at);
    let acme = Who::Organization("acme".to_owned());
    assert_eq!(
        runs(&effects),
        [&Call::Follow {
            who: acme.clone(),
            on: true,
        }]
    );
    assert_eq!(footer(&mut engine, at), " following @acme · f to undo");
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[3].ends_with(" org   following"), "{rows:#?}");
    let done = ticket(&effects, |_| true);
    engine.update(Event::Reply(done, Ok(Reply::Done)), at);

    let effects = press(&mut engine, "f", at);
    assert_eq!(
        runs(&effects),
        [&Call::Follow {
            who: acme,
            on: false
        }]
    );
    assert_eq!(footer(&mut engine, at), " unfollowed @acme · f to undo");
}

#[test]
fn f_on_an_organizations_commit_follows_the_organization() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    let effects = press(&mut engine, "2f", at);
    assert_eq!(
        runs(&effects),
        [&Call::Organization("acme".to_owned())],
        "its follow state comes from its page"
    );
    let asked = ticket(&effects, |_| true);
    let mut effects = engine.update(Event::Reply(asked, Ok(header(false))), at);
    effects.extend(engine.pump(at));
    assert_eq!(
        runs(&effects),
        [&Call::Follow {
            who: Who::Organization("acme".to_owned()),
            on: true,
        }]
    );
}

#[test]
fn an_organizations_author_cannot_be_muted_blocked_or_messaged() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    for keys in ["2m", "2B", "2M"] {
        let effects = press(&mut engine, keys, at);
        assert!(runs(&effects).is_empty(), "{keys}: {:#?}", runs(&effects));
    }
}

#[test]
fn following_lists_the_organizations_they_follow_above_the_people() {
    let at = Instant::now();
    let mut engine = on_person_following(at);
    let rows = screen(&mut engine, 100, 30, at);
    let acme = rows.iter().position(|row| row.contains("Acme @acme · org"));
    let linus = rows.iter().position(|row| row.contains("@linus"));
    assert!(acme.is_some() && acme < linus, "{rows:#?}");
    assert!(
        contains(&rows, "Zeta @zeta · org"),
        "the second page: {rows:#?}"
    );
    // The selection stays on @linus as the organizations come in above, so gg goes to them.
    press(&mut engine, "gg", at);
    let effects = tap(&mut engine, KeyCode::Enter, at);
    assert!(
        runs(&effects).contains(&&Call::Organization("acme".to_owned())),
        "{:#?}",
        runs(&effects)
    );
}

/// Opens a person's profile from a commit of theirs, then their following tab, answering both
/// of its lists: the people and, on two pages fetched back to back, the organizations.
fn on_person_following(at: Instant) -> Engine {
    let mut engine = boot_as(at, member(), common::page);
    let other = common::page(Log::Latest)
        .rows
        .iter()
        .position(|row| row.commit.author.handle != "ada")
        .unwrap();
    let effects = press(&mut engine, &format!("2{}p", "j".repeat(other)), at);
    let user = ticket(&effects, |call| matches!(call, Call::User(_)));
    let handle = common::page(Log::Latest).rows[other]
        .commit
        .author
        .handle
        .clone();
    engine.update(Event::Reply(user, Ok(common::profile(&handle, false))), at);
    let commits = ticket(&effects, |call| matches!(call, Call::Commits { .. }));
    let empty = Reply::Rows(RowPage {
        rows: Vec::new(),
        next_cursor: None,
    });
    engine.update(Event::Reply(commits, Ok(empty)), at);
    // Following is the last tab, a backtab away from commits.
    let following = tap(&mut engine, KeyCode::BackTab, at);
    let calls = runs(&following);
    assert!(
        calls.contains(&&Call::FollowedOrganizations {
            handle: handle.clone(),
            cursor: None,
        }),
        "{calls:#?}"
    );
    let people = ticket(&following, |call| matches!(call, Call::People { .. }));
    let organizations = ticket(&following, |call| {
        matches!(call, Call::FollowedOrganizations { .. })
    });
    let Reply::User(linus) = common::profile("linus", false) else {
        unreachable!("profile builds a user reply");
    };
    engine.update(
        Event::Reply(
            people,
            Ok(Reply::People(PeoplePage {
                people: vec![linus.person],
                next_cursor: None,
            })),
        ),
        at,
    );
    let first = json!({ "data": [acme()], "meta": { "next_cursor": "o2" } });
    let mut more = engine.update(
        Event::Reply(
            organizations,
            Ok(Reply::Organizations(
                OrganizationList::parse(&first.to_string()).unwrap(),
            )),
        ),
        at,
    );
    more.extend(engine.pump(at));
    assert_eq!(
        runs(&more),
        [&Call::FollowedOrganizations {
            handle,
            cursor: Some("o2".to_owned()),
        }],
        "the next page of organizations comes at once"
    );
    let mut zeta = acme();
    zeta["handle"] = json!("zeta");
    zeta["name"] = json!("Zeta");
    let second = json!({ "data": [zeta], "meta": { "next_cursor": null } });
    let asked = ticket(&more, |_| true);
    engine.update(
        Event::Reply(
            asked,
            Ok(Reply::Organizations(
                OrganizationList::parse(&second.to_string()).unwrap(),
            )),
        ),
        at,
    );
    engine
}

#[test]
fn ctrl_x_cycles_whom_the_commit_goes_out_as() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    press(&mut engine, "c", at);
    let title = |engine: &mut Engine| {
        let rows = screen(engine, 100, 30, at);
        rows.iter()
            .find_map(|row| {
                let start = row.find(" commit as @")?;
                Some(row[start..].split(" ─").next().unwrap().trim().to_owned())
            })
            .unwrap_or_else(|| panic!("{rows:#?}"))
    };
    assert_eq!(title(&mut engine), "commit as @ada");
    ctrl(&mut engine, 'x', at);
    assert_eq!(title(&mut engine), "commit as @acme");
    ctrl(&mut engine, 'x', at);
    assert_eq!(title(&mut engine), "commit as @zeta");
    ctrl(&mut engine, 'x', at);
    assert_eq!(title(&mut engine), "commit as @ada");
}

#[test]
fn a_commit_written_as_an_organization_carries_it() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    press(&mut engine, "c", at);
    ctrl(&mut engine, 'x', at);
    press(&mut engine, "shipped", at);
    let effects = ctrl(&mut engine, 's', at);
    let posted = runs(&effects)
        .into_iter()
        .find(|call| matches!(call, Call::Post { .. }))
        .unwrap_or_else(|| panic!("{:#?}", runs(&effects)));
    let Call::Post { author, body, .. } = posted else {
        unreachable!("found a post");
    };
    assert_eq!(
        (author.as_deref(), body.as_str()),
        (Some("acme"), "shipped")
    );
}

#[test]
fn the_next_commit_starts_as_whom_the_last_was_written_as() {
    let at = Instant::now();
    let mut engine = boot_as(at, member(), by_acme);
    press(&mut engine, "c", at);
    ctrl(&mut engine, 'x', at);
    tap(&mut engine, KeyCode::Esc, at);
    press(&mut engine, "2r", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, " Reply to @acme as @acme "), "{rows:#?}");
}

#[test]
fn ctrl_x_does_nothing_for_someone_in_no_organization() {
    let at = Instant::now();
    let mut engine = common::boot(at);
    press(&mut engine, "c", at);
    ctrl(&mut engine, 'x', at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!contains(&rows, " as @"), "{rows:#?}");
    assert!(contains(&rows, " commit "), "{rows:#?}");
}
