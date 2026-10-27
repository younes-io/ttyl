mod common;

use std::time::Instant;

use common::{answer_pages, boot, buffer, ms, press, reload, secs};
use ratatui::style::Color;
use ttyl::theme;

#[test]
fn with_motion_off_a_new_selection_and_a_toast_land_at_once() {
    // SAFETY: this binary's only test sets it before anything reads the environment.
    unsafe { std::env::set_var("TTYL_MOTION", "off") };
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2", at);
    let tint = |engine: &mut _, at| {
        let buffer = buffer(engine, 80, 30, at);
        (1..29)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|cell| buffer[cell].bg)
            .find(|bg| ![Color::Reset, theme::BACKGROUND, theme::MUTED].contains(bg))
            .expect("a card is selected")
    };
    tint(&mut engine, at);
    press(&mut engine, "j", at + secs(1));
    assert_eq!(
        tint(&mut engine, at + secs(1)),
        tint(&mut engine, at + secs(2)),
        "the card is fully tinted on its first frame"
    );

    let effects = reload(&mut engine, at + secs(3));
    answer_pages(&mut engine, &effects, at + secs(3));
    let queued = engine.pump(at + secs(3));
    answer_pages(&mut engine, &queued, at + secs(3));
    let ink = |engine: &mut _, at| buffer(engine, 80, 20, at)[(1, 19)].fg;
    assert_eq!(ink(&mut engine, at + secs(3)), theme::MUTED_FOREGROUND);
    assert_eq!(
        ink(&mut engine, at + secs(7) - ms(40)),
        theme::MUTED_FOREGROUND,
        "the toast is whole until it goes"
    );
}
