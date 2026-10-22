mod common;

use std::time::Instant;

use common::{answer, boot, contains, press, runs, screen, secs, tap};
use devnull::api::{Call, Error};
use devnull::draft::Target;
use devnull::engine::{Effect, Engine, Event};
use devnull::model::CommitId;
use devnull::sched::Ticket;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const GRACE: &str = "2c3d4e5f-0000-4000-8000-000000000201";
const HEIGHT: u16 = 30;
const NO_ANSWER: &str = "fatal: no answer from dev-null.sh. it may have gone through. check your log before sending again.";
const VANISHED: &str = "That commit is no longer available to reply to.";
const SAVED: &str = "Your draft is saved.";
const RULES_KEYS: &str = " O open dev-null.sh  esc back to the draft";

fn grace() -> CommitId {
    CommitId(GRACE.to_owned())
}

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

fn one_post(effects: &[Effect]) -> Ticket {
    let [Effect::Run(ticket, Call::Post { .. })] = effects else {
        panic!("one commit goes out: {effects:?}");
    };
    *ticket
}

/// Opens the composer on `target`, types "hello", and sends it.
fn sent(target: &Target, at: Instant) -> (Engine, Ticket) {
    let mut engine = boot(at);
    let open = match target {
        Target::New => "c",
        Target::Reply(_) => "2jjr",
        Target::Quote(_) => "2jjq",
    };
    press(&mut engine, open, at);
    press(&mut engine, "hello", at);
    let ticket = one_post(&ctrl(&mut engine, 's', at));
    (engine, ticket)
}

fn refuse(engine: &mut Engine, ticket: Ticket, error: Error, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(Event::Reply(ticket, Err(error)), at);
    effects.extend(engine.pump(at));
    effects
}

fn api(status: u16, code: Option<&str>, message: &str) -> Error {
    Error::Api {
        status,
        code: code.map(str::to_owned),
        message: message.to_owned(),
        retry_after: (status == 429).then(|| secs(30)),
    }
}

/// Lets time pass, with the logs refreshing on their timers, and returns every commit sent.
fn posts_over_the_next_hour(engine: &mut Engine, at: Instant) -> Vec<Call> {
    let mut sent = Vec::new();
    for later in [1, 6, 31, 60, 600, 3600] {
        let now = at + secs(later);
        let mut effects = engine.update(Event::Tick, now);
        effects.extend(engine.pump(now));
        sent.extend(
            runs(&effects)
                .into_iter()
                .filter(|call| matches!(call, Call::Post { .. }))
                .cloned(),
        );
    }
    sent
}

fn timed_out() -> Error {
    Error::Http(ureq::Error::Timeout(ureq::Timeout::RecvResponse))
}

#[test]
fn each_refusal_keeps_the_draft_says_why_and_sends_nothing_more() {
    let at = Instant::now();
    for (target, error, shown) in [
        (
            Target::New,
            api(403, Some("timed_out"), "You're timed out until 3pm."),
            "You're timed out until 3pm.",
        ),
        (
            Target::New,
            api(403, Some("banned"), "This account is banned."),
            "This account is banned.",
        ),
        (
            Target::New,
            api(
                429,
                Some("rate_limited"),
                "Too many attempts. Try again in a minute.",
            ),
            "Too many attempts. Try again in a minute.",
        ),
        (
            Target::New,
            api(422, None, "The body field is required."),
            "The body field is required.",
        ),
        (
            Target::Reply(grace()),
            api(
                404,
                Some("commit_unavailable"),
                "This commit isn't available.",
            ),
            VANISHED,
        ),
        (
            Target::Quote(grace()),
            api(
                404,
                Some("commit_unavailable"),
                "This commit isn't available.",
            ),
            VANISHED,
        ),
        (Target::New, timed_out(), NO_ANSWER),
        (Target::New, Error::Unexpected(504), NO_ANSWER),
    ] {
        let (mut engine, ticket) = sent(&target, at);
        let effects = refuse(&mut engine, ticket, error, at);
        assert!(runs(&effects).is_empty(), "{shown}: {effects:?}");
        assert_eq!(engine.draft(&target), Some("hello"), "{shown}");
        let rows = screen(&mut engine, 100, HEIGHT, at);
        assert!(contains(&rows, shown), "{shown}: {rows:#?}");
        assert_eq!(posts_over_the_next_hour(&mut engine, at), [], "{shown}");
        assert_eq!(engine.draft(&target), Some("hello"), "{shown}");
    }
}

#[test]
fn a_commit_with_no_answer_is_not_resent_when_the_connection_comes_back() {
    let at = Instant::now();
    let (mut engine, ticket) = sent(&Target::New, at);
    refuse(&mut engine, ticket, timed_out(), at);
    let effects = tap(&mut engine, KeyCode::Esc, at);
    assert!(runs(&effects).is_empty());
    let mut effects = press(&mut engine, "R", at);
    effects.extend(engine.pump(at));
    assert!(
        !runs(&effects).is_empty(),
        "the log reloads over the restored connection"
    );
    assert!(
        !runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Post { .. })),
        "{effects:?}"
    );
    answer(&mut engine, effects, at);
    assert_eq!(posts_over_the_next_hour(&mut engine, at), []);
    assert_eq!(engine.draft(&Target::New), Some("hello"));
}

#[test]
fn the_no_answer_copy_wraps_on_a_narrow_panel() {
    let at = Instant::now();
    let (mut engine, ticket) = sent(&Target::New, at);
    refuse(&mut engine, ticket, timed_out(), at);
    let rows = screen(&mut engine, 44, HEIGHT, at);
    let text: String = rows
        .iter()
        .skip_while(|row| !row.starts_with(" commit ─"))
        .skip(1)
        .take(usize::from(HEIGHT))
        .filter(|row| !row.contains("ctrl-s"))
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains(NO_ANSWER), "{rows:#?}");
}

fn rules(at: Instant) -> Engine {
    let (mut engine, ticket) = sent(&Target::New, at);
    let error = api(
        403,
        Some("commit_rules_required"),
        "Read the commit rules on dev-null.sh before your first commit.",
    );
    assert!(runs(&refuse(&mut engine, ticket, error, at)).is_empty());
    engine
}

#[test]
fn a_first_commit_shows_the_rules_panel_at_100_and_44_columns() {
    let at = Instant::now();
    let mut engine = rules(at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(
        contains(&rows, " Your first commit has to come from the web,"),
        "{rows:#?}"
    );
    assert!(
        contains(&rows, " so you can read the commit rules first."),
        "{rows:#?}"
    );
    assert!(contains(&rows, &format!(" {SAVED}")), "{rows:#?}");
    assert!(!contains(&rows, "hello"), "{rows:#?}");
    assert_eq!(rows[usize::from(HEIGHT) - 1], RULES_KEYS);

    let rows = screen(&mut engine, 44, HEIGHT, at);
    let copy: String = rows
        .iter()
        .skip_while(|row| !row.starts_with(" commit ─"))
        .skip(1)
        .take_while(|row| !row.contains(SAVED))
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        copy, "Your first commit has to come from the web, so you can read the commit rules first.",
        "{rows:#?}"
    );
    assert!(
        rows.iter().all(|row| row.chars().count() <= 44),
        "{rows:#?}"
    );
    assert_eq!(rows[usize::from(HEIGHT) - 1], RULES_KEYS);
}

#[test]
fn o_opens_the_site_and_types_nothing() {
    let at = Instant::now();
    let mut engine = rules(at);
    let effects = press(&mut engine, "O", at);
    assert_eq!(effects, [Effect::Open("https://dev-null.sh/".to_owned())]);
    assert!(press(&mut engine, "xyz", at).is_empty());
    assert_eq!(engine.draft(&Target::New), Some("hello"));
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, "Your first commit"), "{rows:#?}");
}

#[test]
fn esc_goes_back_to_the_draft_and_a_paste_waits_until_then() {
    let at = Instant::now();
    let mut engine = rules(at);
    assert!(
        engine
            .update(Event::Paste(" world".to_owned()), at)
            .is_empty()
    );
    assert!(tap(&mut engine, KeyCode::Esc, at).is_empty());
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, " hello"), "{rows:#?}");
    assert!(!contains(&rows, "Your first commit"), "{rows:#?}");
    assert_eq!(
        rows[usize::from(HEIGHT) - 1],
        " ctrl-s commit  ctrl-e editor  esc close"
    );
    press(&mut engine, "!", at);
    assert_eq!(engine.draft(&Target::New), Some("hello!"));
    one_post(&ctrl(&mut engine, 's', at));
}

#[test]
fn ctrl_c_quits_from_the_rules_panel() {
    let at = Instant::now();
    let mut engine = rules(at);
    assert_eq!(ctrl(&mut engine, 'c', at), [Effect::Quit]);
}

#[test]
fn a_refusal_for_a_closed_composer_is_a_toast() {
    let at = Instant::now();
    let (mut engine, ticket) = sent(&Target::New, at);
    tap(&mut engine, KeyCode::Esc, at);
    refuse(&mut engine, ticket, timed_out(), at);
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(
        rows[usize::from(HEIGHT) - 1].contains("fatal: no answer"),
        "{rows:#?}"
    );
    assert_eq!(engine.draft(&Target::New), Some("hello"));
}
