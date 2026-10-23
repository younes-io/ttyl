mod common;

use std::time::Instant;

use common::{boot, contains, press, screen, tap};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ttyl::api::Call;
use ttyl::attach::Attachment;
use ttyl::draft::Target;
use ttyl::engine::{Effect, Engine, Event};

const BODY: &str = "deploy with ghp_aB3dE5fG7hJ9kL1mN2pQ4rS6tU8vW0xY3zA5";
const ASK: &str = "possible secret detected. commit anyway? [y/N]";
const ASK_SHORT: &str = "possible secret. commit anyway? [y/N]";
const HEIGHT: u16 = 30;

fn ctrl(engine: &mut Engine, ch: char, at: Instant) -> Vec<Effect> {
    let mut effects = engine.update(
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)),
        at,
    );
    effects.extend(engine.pump(at));
    effects
}

fn asking(at: Instant) -> Engine {
    let mut engine = boot(at);
    press(&mut engine, "c", at);
    press(&mut engine, BODY, at);
    assert!(
        ctrl(&mut engine, 's', at).is_empty(),
        "nothing goes out yet"
    );
    engine
}

#[test]
fn a_secret_asks_on_the_status_line_before_sending() {
    let at = Instant::now();
    let mut engine = asking(at);
    for (width, ask) in [(100, ASK), (44, ASK_SHORT)] {
        let rows = screen(&mut engine, width, HEIGHT, at);
        assert!(contains(&rows, &format!(" {ask}")), "{width}: {rows:#?}");
        assert_eq!(
            rows[usize::from(HEIGHT) - 1],
            " y commit anyway",
            "{rows:#?}"
        );
    }
}

#[test]
fn only_y_sends() {
    let at = Instant::now();
    let mut engine = asking(at);
    let effects = press(&mut engine, "y", at);
    let effects: Vec<_> = effects.into_iter().chain(engine.pump(at)).collect();
    let [Effect::Run(_, call)] = effects.as_slice() else {
        panic!("one commit goes out: {effects:?}");
    };
    assert_eq!(
        *call,
        Call::Post {
            target: Target::New,
            body: BODY.to_owned(),
            attachment: Attachment::None,
        }
    );
}

#[test]
fn any_other_key_goes_back_to_editing_with_nothing_sent_or_typed() {
    let at = Instant::now();
    for (n, code) in [
        KeyCode::Char('n'),
        KeyCode::Char('N'),
        KeyCode::Char('Q'),
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Backspace,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = asking(at);
        let mut effects = tap(&mut engine, code, at);
        effects.extend(engine.pump(at));
        assert!(effects.is_empty(), "answer {n}: {effects:?}");
        assert_eq!(engine.draft(&Target::New), Some(BODY), "answer {n}");
        let rows = screen(&mut engine, 100, HEIGHT, at);
        assert!(!contains(&rows, "commit anyway"), "answer {n}: {rows:#?}");
        assert!(
            contains(&rows, " commit ─"),
            "the panel stays open: {rows:#?}"
        );
        assert_eq!(
            rows[usize::from(HEIGHT) - 1],
            " ctrl-s commit  ctrl-e editor  ctrl-p poll  ctrl-v image  esc close"
        );
        press(&mut engine, "!", at);
        assert_eq!(
            engine.draft(&Target::New),
            Some(format!("{BODY}!").as_str()),
            "answer {n}: the next key types"
        );
    }
}

#[test]
fn ctrl_c_still_quits_while_asking() {
    let at = Instant::now();
    let mut engine = asking(at);
    assert_eq!(ctrl(&mut engine, 'c', at), [Effect::Quit]);
}

#[test]
fn ctrl_s_after_backing_out_asks_again() {
    let at = Instant::now();
    let mut engine = asking(at);
    press(&mut engine, "n", at);
    assert!(ctrl(&mut engine, 's', at).is_empty());
    let rows = screen(&mut engine, 100, HEIGHT, at);
    assert!(contains(&rows, ASK), "{rows:#?}");
}

#[test]
fn a_plain_body_sends_without_asking() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "cghp_ is how github tokens start", at);
    let effects = ctrl(&mut engine, 's', at);
    assert!(
        matches!(effects.as_slice(), [Effect::Run(_, Call::Post { .. })]),
        "{effects:?}"
    );
}
