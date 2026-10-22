use std::env;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::thread;

use base64ct::{Base64, Encoding};

pub fn osc52(text: &str, tmux: bool) -> String {
    let set = format!("\x1b]52;c;{}\x07", Base64::encode_string(text.as_bytes()));
    if tmux {
        // tmux takes the bare sequence when set-clipboard is on and the wrapped one when
        // allow-passthrough is on. Both are off by default, so send both.
        format!("{set}\x1bPtmux;{}\x1b\\", set.replace('\x1b', "\x1b\x1b"))
    } else {
        set
    }
}

pub fn copy(text: &str) {
    let mut out = io::stdout();
    let _ = out.write_all(osc52(text, env::var_os("TMUX").is_some()).as_bytes());
    let _ = out.flush();
    if !spawn("wl-copy", &[], Some(text)) {
        spawn("xclip", &["-selection", "clipboard"], Some(text));
    }
}

pub fn open(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    spawn(opener, &[url], None);
}

fn spawn(program: &str, args: &[&str], input: Option<&str>) -> bool {
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    thread::spawn(move || child.wait());
    true
}
