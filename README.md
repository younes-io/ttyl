# devnull-tui

A terminal client for [/dev/null](https://dev-null.sh), the social network
where developers commit plain text to a shared log. You can read, reply,
and post without touching the mouse.

```
 /dev/null   1 following  2 latest  3 trending                      ● 3  @ada

 ┆ Today
 │
 ● Ada @ada · 18m · ↩ 3  ★ 12                                         e4f5a61
 ┃ switched the feed to cursorPaginate(20). offsets fall over once the table
 ┃ grows.
 ┃   ● Grace @grace · 10m  @ada did you keep an index on created_at?
 │
 ● Grace @grace · 1h · ↩ 5  ★ 27                                      0000201
 │ today's footgun:
 │
 │  $name = 'devnull';
 │  echo "<b>$name</b>";
 │ +2 lines · space to expand
 │
 j/k move  enter open  c commit  ? help  Q quit
```

## Install

You need a recent stable [Rust](https://rustup.rs). Then run:

```
cargo install --git https://github.com/younes-io/devnull-tui
devnull
```

To remove it, run `cargo uninstall devnull`.

## Sign in

You need a dev-null.sh account. Press `enter` on the first screen, and your
browser opens dev-null.sh so you can approve this terminal. If no browser
opens, the screen shows a link to visit instead. You sign in once per
machine.

Sign-in uses `xdg-mime`, so it works on a Linux desktop.

## Using it

There are three logs. Press `1`, `2` or `3` to switch between them.

- **Following** shows your commits and the people you follow.
- **Latest** shows everyone, newest first.
- **Trending** shows what is popular right now.

When new commits arrive, a note appears at the top. Press `R` to load them.

Press `enter` to open a commit and read its replies, and `esc` to go back.
On a terminal at least 110 columns wide, the thread also appears beside the
log.

The `● 3` at the top right counts your unread notifications. Press `gn` to
read them. Opening them marks them all as read, as the website does.

Press `c` to write a commit, `r` to reply, or `q` to quote. Press `ctrl-s`
to send it, or `ctrl-e` to write in your own editor. If you close the
composer with `esc`, your draft is kept, even after you quit. If the text
looks like it contains a secret, such as an API key, you are asked before
it is sent.

## Keys

The bottom line shows the keys you need most. Press `?` for all of them.

| Key | Action |
| --- | --- |
| `j` `k` | Move down or up |
| `gg` `G` | Go to the top or bottom |
| `space` | Show or hide the rest of a long commit |
| `enter` | Open |
| `esc` | Go back |
| `1` `2` `3` `tab` | Switch log |
| `R` | Reload |
| `gn` | Notifications |
| `c` | Write a commit |
| `r` `q` | Reply or quote |
| `s` | Star |
| `f` | Follow the author |
| `y` | Copy the link |
| `O` | Open in the browser |
| `P` | Give a pastel, if pastéis mode is on |
| `D` | Delete your own commit |
| `!` | Report |
| `z` | Screensaver |
| `?` | Help |
| `Q` | Quit |

## Good to know

- `in review` next to a name means the commit is waiting for approval.
- Any terminal size works. Colors look best with truecolor support.
- Inside tmux, copying needs tmux's `set-clipboard` or `allow-passthrough`
  option.

## Development

```
git clone https://github.com/younes-io/devnull-tui
cd devnull-tui
cargo test
cargo run
```

Set `DEVNULL_URL` to point the app at another server, such as a local copy
of dev-null.sh.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

This client is not affiliated with
[dev-null.sh](https://github.com/nunomaduro/dev-null.sh).
