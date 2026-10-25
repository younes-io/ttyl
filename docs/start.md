# Getting started

## Sign in

You need a [dev-null.sh](https://dev-null.sh) account.

1. Run `ttyl`.
2. Press <kbd>enter</kbd>. Your browser opens dev-null.sh.
3. Approve this terminal in the browser.

{{screen signin}}

If no browser opens, ttyl shows a link. Open it in a browser on the same
computer, because the browser hands the approval back to ttyl there. You sign
in once per machine. On Linux, sign in from a desktop session. See
[Settings](settings.html#linux).

## Find your way around

After you sign in, ttyl opens your following log. The line at the top shows
your unread notifications, such as `● 3`, and your name. The line at the
bottom shows the keys you need most on the current screen.

Press <kbd>?</kbd> to see every key for the screen you are on. Press
<kbd>?</kbd> again or <kbd>esc</kbd> to close it.

{{screen help}}

Press <kbd>Q</kbd> or <kbd>ctrl-c</kbd> to quit.

## Next

- [Read](read.html) the logs and threads.
- [Write](write.html) your first commit.

## Uninstall

Run `brew uninstall ttyl` or `cargo uninstall ttyl`. If you used the install
script, delete `~/.cargo/bin/ttyl`. To also remove the key this terminal
signs in with, delete `~/.config/ttyl`, or `$XDG_CONFIG_HOME/ttyl` if you set
that variable.
