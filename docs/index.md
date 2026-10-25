# A terminal client for /dev/null

ttyl lets you read, reply and post on [dev-null.sh](https://dev-null.sh), a
social network for developers where a post is called a commit. Every
action has a key, so you never reach for the mouse. It runs on macOS and Linux.

{{screen home}}

ttyl draws every screen on this site itself. The site build runs the app
against sample data and copies what it puts on the terminal, cell by cell.

<section class="panel install" id="install" aria-labelledby="install-title">
<h2 id="install-title">Install</h2>

With Homebrew:

```sh
brew install younes-io/tap/ttyl
```

With the install script, which puts `ttyl` in `~/.cargo/bin`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/younes-io/ttyl/releases/latest/download/ttyl-installer.sh | sh
```

With Cargo, if you have a recent stable [Rust](https://rustup.rs):

```sh
cargo install ttyl
```

You can also download a binary from the
[releases page](https://github.com/younes-io/ttyl/releases).

Then run `ttyl`. [Getting started](start.html) covers the first minute.

</section>

## What you can do

- [Read](read.html) the following, latest and trending logs, and open threads.
- [Write](write.html) commits, replies and quotes, with polls, images and a preview.
- [Star, vote and view images](react.html) without leaving the terminal.
- [Read notifications, profiles and your starred commits](people.html).
- Look up [every key](keys.html), or [change settings and fix problems](settings.html).

On any page, press a number from the bar at the bottom to go to that page,
as you switch logs in ttyl.
