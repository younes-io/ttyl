mod common;

use std::time::Instant;

use common::{boot, boot_serving, contains, page, press, profile, runs, screen, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Card, Focus, Log, LogPage, Row, RowPage, Shown};
use ttyl::sched::Ticket;

fn latest(at: usize) -> Row {
    page(Log::Latest).rows[at].clone()
}

fn pin(at: usize, on: bool) -> Call {
    Call::Pin {
        id: latest(at).commit.id,
        on,
    }
}

/// The commit as dev-null.sh answers a pin, with `is_pinned` as it now is.
fn pinned(at: usize, on: bool) -> Reply {
    let row = latest(at);
    let mut commit = row.commit;
    commit.is_pinned = on;
    Reply::Commit(Box::new(Focus {
        card: Card {
            commit,
            media: row.media,
            stars_count: row.stars_count,
            starred: row.starred,
            quoted_commit: row.quoted_commit,
        },
        replies_count: row.replies_count,
    }))
}

fn one_run(effects: &[Effect]) -> Ticket {
    let [Effect::Run(ticket, _)] = effects else {
        panic!("one call goes out: {:#?}", runs(effects));
    };
    *ticket
}

fn reply(
    engine: &mut Engine,
    ticket: Ticket,
    result: Result<Reply, Error>,
    at: Instant,
) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, result), at);
    effects.extend(engine.pump(at));
    effects
}

/// The card header that carries this short hash.
fn header<'a>(rows: &'a [String], short_hash: &str) -> &'a str {
    rows.iter()
        .find(|row| row.contains(short_hash))
        .unwrap_or_else(|| panic!("{short_hash}: {rows:#?}"))
}

#[test]
fn caret_pins_your_commit_and_its_card_says_so() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    assert!(!header(&screen(&mut engine, 100, 40, at), "e4f5a61").contains("pinned"));

    let effects = press(&mut engine, "^", at);
    assert_eq!(runs(&effects), [&pin(0, true)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(header(&rows, "e4f5a61").contains("  pinned"), "{rows:#?}");
    assert!(
        rows[39].starts_with(" pinned to your profile · ^ to unpin"),
        "{}",
        rows[39]
    );
}

#[test]
fn a_new_pin_replaces_the_old_one_everywhere_at_once() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let ticket = one_run(&press(&mut engine, "^", at));
    reply(&mut engine, ticket, Ok(pinned(0, true)), at);

    let effects = press(&mut engine, "j^", at);
    assert_eq!(runs(&effects), [&pin(1, true)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(header(&rows, "e4f5a62").contains("pinned"), "{rows:#?}");
    assert!(!header(&rows, "e4f5a61").contains("pinned"), "{rows:#?}");
    assert!(
        rows[39].starts_with(" pinned to your profile in place of e4f5a61 · ^ to unpin"),
        "{}",
        rows[39]
    );
}

#[test]
fn caret_again_unpins() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let ticket = one_run(&press(&mut engine, "^", at));
    reply(&mut engine, ticket, Ok(pinned(0, true)), at);
    assert_eq!(runs(&press(&mut engine, "^", at)), [&pin(0, false)]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!header(&rows, "e4f5a61").contains("pinned"), "{rows:#?}");
    assert!(
        rows[39].starts_with(" unpinned from your profile"),
        "{}",
        rows[39]
    );
}

#[test]
fn a_refused_pin_comes_off_and_says_why() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let ticket = one_run(&press(&mut engine, "^", at));
    let refused = Error::Api {
        status: 403,
        code: None,
        message: "This action is unauthorized.".to_owned(),
        retry_after: None,
    };
    reply(&mut engine, ticket, Err(refused), at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!header(&rows, "e4f5a61").contains("pinned"), "{rows:#?}");
    assert_eq!(rows[39].trim_end(), " This action is unauthorized.");
}

#[test]
fn only_your_own_commits_pin() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    assert!(press(&mut engine, "^", at).is_empty(), "grace's commit");
}

/// Opens your own profile from your first latest commit, with these of your commits listed.
fn on_your_profile(at: Instant, listed: &[usize]) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let rows = listed.iter().map(|at| latest(*at)).collect();
    open_profile(&mut engine, profile("ada", false), rows, at);
    engine
}

/// Presses `p` on the selected commit and answers with this header and these commits.
fn open_profile(engine: &mut Engine, header: Reply, listed: Vec<Row>, at: Instant) {
    let mut header = Some(header);
    let mut listed = Some(listed);
    for effect in press(engine, "p", at) {
        let Effect::Run(ticket, call) = effect else {
            continue;
        };
        let answer = match call {
            Call::User(_) => header.take().unwrap(),
            Call::Commits { .. } => Reply::Rows(RowPage {
                rows: listed.take().unwrap(),
                next_cursor: None,
            }),
            _ => continue,
        };
        engine.update(Event::Reply(ticket, Ok(answer)), at);
    }
}

#[test]
fn a_refused_pin_leaves_a_later_one_standing() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let first = one_run(&press(&mut engine, "^", at));
    assert!(
        runs(&press(&mut engine, "j^", at)).is_empty(),
        "one at a time"
    );
    let refused = Error::Api {
        status: 403,
        code: None,
        message: "This action is unauthorized.".to_owned(),
        retry_after: None,
    };
    let effects = reply(&mut engine, first, Err(refused), at);
    assert_eq!(runs(&effects), [&pin(1, true)]);
}

/// Every log, with the commit e4f5a62 quotes claiming a pin of its own.
fn quoting_a_pinned_copy(log: Log) -> LogPage {
    let mut page = page(log);
    for row in &mut page.rows {
        if let Some(Shown::Live(quote)) = row.quoted_commit.as_mut()
            && row.commit.short_hash == "e4f5a62"
        {
            quote.commit.is_pinned = true;
        }
    }
    page
}

#[test]
fn a_quoted_copy_never_shows_a_pin() {
    let at = Instant::now();
    let mut engine = boot_serving(at, quoting_a_pinned_copy);
    press(&mut engine, "2j", at);
    tap(&mut engine, KeyCode::Enter, at);
    let rows = screen(&mut engine, 100, 40, at);
    let quote = header(&rows, "Margaret @margaret");
    assert!(!quote.contains("pinned"), "{rows:#?}");
}

/// Every log, with grace's commit 0000201 still marked pinned from before.
fn graces_old_pin(log: Log) -> LogPage {
    let mut page = page(log);
    for row in &mut page.rows {
        row.commit.is_pinned = row.commit.short_hash == "0000201";
    }
    page
}

#[test]
fn a_profile_naming_another_pin_unpins_the_old_one_everywhere() {
    let at = Instant::now();
    let mut engine = boot_serving(at, graces_old_pin);
    press(&mut engine, "2jj", at);
    assert!(header(&screen(&mut engine, 100, 40, at), "0000201").contains("pinned"));

    let Reply::User(mut grace) = profile("grace", false) else {
        unreachable!();
    };
    grace.pinned = Some(latest(5));
    open_profile(&mut engine, Reply::User(grace), Vec::new(), at);
    tap(&mut engine, KeyCode::Esc, at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!header(&rows, "0000201").contains("pinned"), "{rows:#?}");
    assert!(header(&rows, "0000601").contains("pinned"), "{rows:#?}");
}

#[test]
fn your_profile_shows_a_pin_it_has_not_listed() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2j", at);
    let ticket = one_run(&press(&mut engine, "^", at));
    reply(&mut engine, ticket, Ok(pinned(1, true)), at);

    press(&mut engine, "k", at);
    open_profile(
        &mut engine,
        profile("ada", false),
        vec![latest(0), latest(8)],
        at,
    );
    let rows = screen(&mut engine, 100, 40, at);
    let label = rows.iter().position(|row| row.contains("┆ pinned"));
    let [moved, first] = order(&rows, &["e4f5a62", "e4f5a61"])[..] else {
        unreachable!();
    };
    assert!(
        label.is_some_and(|label| label < moved) && moved < first,
        "{rows:#?}"
    );
}

fn order(rows: &[String], hashes: &[&str]) -> Vec<usize> {
    hashes
        .iter()
        .map(|hash| rows.iter().position(|row| row.contains(hash)).unwrap())
        .collect()
}

#[test]
fn your_profile_puts_a_new_pin_first_and_an_unpin_back_by_date() {
    let at = Instant::now();
    let mut engine = on_your_profile(at, &[0, 1, 8]);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(order(&rows, &["e4f5a61", "e4f5a62", "0000901"]).is_sorted());

    press(&mut engine, "j^", at);
    let rows = screen(&mut engine, 100, 40, at);
    let label = rows
        .iter()
        .position(|row| row.contains("┆ pinned"))
        .unwrap();
    let [moved, first, last] = order(&rows, &["e4f5a62", "e4f5a61", "0000901"])[..] else {
        unreachable!();
    };
    assert!(label < moved && moved < first && first < last, "{rows:#?}");
    assert!(
        selected(&rows).contains("e4f5a62"),
        "the selection follows the pin: {rows:#?}"
    );

    press(&mut engine, "^", at);
    let rows = screen(&mut engine, 100, 40, at);
    assert!(!contains(&rows, "┆ pinned"), "{rows:#?}");
    assert!(
        order(&rows, &["e4f5a61", "e4f5a62", "0000901"]).is_sorted(),
        "{rows:#?}"
    );
    assert!(
        selected(&rows).contains("e4f5a62"),
        "the selection follows the commit back down: {rows:#?}"
    );
}

/// The header of the selected card, the one over the first lifted rail.
fn selected(rows: &[String]) -> &str {
    let rail = rows.iter().position(|row| row.starts_with(" ┃")).unwrap();
    &rows[rail - 1]
}
