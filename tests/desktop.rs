use base64ct::{Base64, Encoding};
use devnull::desktop::osc52;

const URL: &str = "https://dev-null.sh/@grace/commits/2c3d4e5f-0000-4000-8000-000000000201";

#[test]
fn the_clipboard_sequence_carries_the_link_and_survives_tmux() {
    let encoded = Base64::encode_string(URL.as_bytes());
    assert_eq!(osc52(URL, false), format!("\x1b]52;c;{encoded}\x07"));
    assert_eq!(
        osc52(URL, true),
        format!("\x1b]52;c;{encoded}\x07\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    );
}
