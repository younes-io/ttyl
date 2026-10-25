<div align="center">

# ttyl

<p><b><picture><source media="(prefers-color-scheme: dark)" srcset="https://younes-io.github.io/ttyl/dev-null-wordmark-dark.svg"><img src="https://younes-io.github.io/ttyl/dev-null-wordmark-light.svg" alt="/dev/null" height="18"></picture>, in your terminal.</b></p>

A TUI for [dev-null.sh](https://dev-null.sh), a social network for
developers where a post is called a commit.

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

## everything, without the mouse

- **Read** the following, latest and trending logs. Open a thread beside
  the log on a wide terminal, or full screen on a narrow one.
- **Write** commits, replies and quotes. Preview the draft as the commit it
  will become, code colors included, or write it in your own editor.
- **Attach** a poll or up to three images to a commit. Vote in a
  keystroke, and view images inside the terminal.
- **Star** a commit, or give its author a pastel.
- **Catch up** on who replied, quoted, starred, mentioned or followed you.
  Open anyone's profile and follow them.
- **Keep** your drafts through a closed composer and a quit. ttyl also asks
  before it sends text that looks like a secret.

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

ttyl draws every picture here from the code on `main`. They are real
screens, not mockups.

## install

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

## how it's made

I build ttyl with AI coding agents. I decide how each screen should work,
check what the agents write, and try every feature by hand on my Omarchy
Linux machine and my Mac.

## development

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

## license

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
