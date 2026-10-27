mod common;

use std::time::Instant;

use common::{contains, press, screen};
use ttyl::api::{Call, Error};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::theme;

fn signed_out(at: Instant) -> Engine {
    let mut engine = Engine::new("ttyl".to_owned(), at);
    let effects = engine.pump(at);
    let [Effect::Run(me, Call::Me)] = effects.as_slice() else {
        panic!("the app asks who it is first");
    };
    let refused = Error::Api {
        status: 401,
        code: Some("device_not_approved".to_owned()),
        message: "device_not_approved from the server".to_owned(),
        retry_after: None,
    };
    engine.update(Event::Reply(*me, Err(refused)), at);
    engine
}

#[test]
fn the_welcome_screen_shows_the_icons_and_i_switches_them_for_good() {
    let at = Instant::now();
    let mut engine = signed_out(at);
    let rows = screen(&mut engine, 100, 20, at);
    assert!(contains(&rows, theme::icons().star), "{rows:#?}");
    assert!(
        contains(&rows, "If these look like boxes, press i"),
        "{rows:#?}"
    );

    assert_eq!(press(&mut engine, "i", at), [Effect::Icons(true)]);
    let rows = screen(&mut engine, 100, 20, at);
    assert!(
        contains(&rows, "★  ↩"),
        "the plain symbols take their place: {rows:#?}"
    );
    assert!(
        contains(&rows, "Press i to try Nerd Font icons."),
        "{rows:#?}"
    );

    assert_eq!(press(&mut engine, "i", at), [Effect::Icons(false)]);
    assert!(!theme::plain_icons(), "i goes back to Nerd Font icons");
}
