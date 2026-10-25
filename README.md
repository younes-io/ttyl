<div align="center">

# ttyl

**/dev/null, in your terminal.**

A fast, keyboard-first client for [dev-null.sh](https://dev-null.sh), the
social network where developers commit plain text to a shared log.

<sub>An unofficial client. Not affiliated with or endorsed by dev-null.sh.</sub>

[![crates.io](https://img.shields.io/crates/v/ttyl?style=flat-square&color=ff8a4c)](https://crates.io/crates/ttyl)
[![ci](https://img.shields.io/github/actions/workflow/status/younes-io/ttyl/ci.yml?branch=main&style=flat-square&label=ci)](https://github.com/younes-io/ttyl/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/ttyl?style=flat-square&color=6e8bff)](LICENSE)

[**Docs**](https://younes-io.github.io/ttyl/) ·
[Install](#install) ·
[Every key](https://younes-io.github.io/ttyl/keys.html) ·
[Releases](https://github.com/younes-io/ttyl/releases)

<img src="https://younes-io.github.io/ttyl/screen-home.svg" alt="ttyl with the latest log on the left and the open thread of the selected commit on the right" width="100%">

</div>

## Everything, without the mouse

- **Read** the following, latest and trending logs. Open a thread beside
  the log on a wide terminal, or full screen on a narrow one.
- **Write** commits, replies and quotes. Preview the draft as the commit it
  will become, code colors included, or write it in your own editor.
- **Polls and images.** Add a poll or up to three images to a commit. Vote
  in a keystroke, and view images inside the terminal.
- **Stars and pastéis.** Star a commit, or give its author a pastel.
- **Notifications and profiles.** See who replied, quoted, starred,
  mentioned or followed you. Open anyone's profile and follow them.
- **Careful by default.** Drafts survive a closed composer and a quit.
  ttyl asks before it sends text that looks like a secret.

<table>
  <tr>
    <td width="50%"><img src="https://younes-io.github.io/ttyl/screen-vote.svg" alt="Voting in a poll"><br><sub>Vote in a poll</sub></td>
    <td width="50%"><img src="https://younes-io.github.io/ttyl/screen-images.svg" alt="The image viewer"><br><sub>View images in the terminal</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="https://younes-io.github.io/ttyl/screen-preview.svg" alt="Previewing a draft"><br><sub>Preview before you commit</sub></td>
    <td width="50%"><img src="https://younes-io.github.io/ttyl/screen-notifications.svg" alt="Notifications"><br><sub>Catch up on notifications</sub></td>
  </tr>
</table>

Every picture here is drawn by ttyl itself from the current code, so it
always matches the version you install.

## Install

ttyl runs on macOS and Linux.

```sh
brew install younes-io/tap/ttyl
```

<details>
<summary>Other ways to install</summary>

With the install script, which puts `ttyl` in `~/.cargo/bin`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/younes-io/ttyl/releases/latest/download/ttyl-installer.sh | sh
```

With Cargo, if you have a recent stable [Rust](https://rustup.rs):

```sh
cargo install ttyl
```

You can also download a binary from the
[releases page](https://github.com/younes-io/ttyl/releases). On macOS, a
binary downloaded in the browser is blocked until you run
`xattr -d com.apple.quarantine ttyl`.

To remove ttyl, run `brew uninstall ttyl` or `cargo uninstall ttyl`. If you
used the install script, delete `~/.cargo/bin/ttyl`.

</details>

Then run `ttyl`, press `enter`, and approve this terminal in your browser.
Press `?` at any time to see every key for the screen you're on.

[Getting started](https://younes-io.github.io/ttyl/start.html) covers the
first minute.
[Settings and troubleshooting](https://younes-io.github.io/ttyl/settings.html)
covers tmux, Nerd Fonts and images.

## Development

```sh
git clone https://github.com/younes-io/ttyl
cd ttyl
cargo test
cargo run
```

Set `DEVNULL_URL` to point the app at another server, such as a local copy
of dev-null.sh.

`cargo test` also builds the docs site from the Markdown in `docs/` into
`target/site`, including the screens this README shows. To build only the
site, run `cargo test --test site`, then open `target/site/index.html`.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
