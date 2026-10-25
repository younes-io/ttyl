# ttyl

A terminal client for [/dev/null](https://dev-null.sh), the social network
where developers commit plain text to a shared log. You can read, reply,
and post without touching the mouse.

This is an unofficial client. It is not affiliated with or endorsed by
[dev-null.sh](https://github.com/nunomaduro/dev-null.sh).

See what it looks like and how to use it at
**[younes-io.github.io/ttyl](https://younes-io.github.io/ttyl/)**. ttyl
draws every screen on that site itself, so the pictures match the current
version.

## Install

ttyl runs on macOS and Linux. Pick one of these.

With Homebrew:

```
brew install younes-io/tap/ttyl
```

With the install script, which puts `ttyl` in `~/.cargo/bin`:

```
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/younes-io/ttyl/releases/latest/download/ttyl-installer.sh | sh
```

With Cargo, if you have a recent stable [Rust](https://rustup.rs):

```
cargo install ttyl
```

You can also download a binary from the
[releases page](https://github.com/younes-io/ttyl/releases). On macOS, a
binary downloaded in the browser is blocked until you run
`xattr -d com.apple.quarantine ttyl`.

Then run `ttyl`, press `enter`, and approve this terminal in your browser.
Press `?` in the app for every key on the current screen. To check the
install, run `ttyl --version`.

To remove it, run `brew uninstall ttyl` or `cargo uninstall ttyl`. If you
used the install script, delete `~/.cargo/bin/ttyl`.

## Learn more

- [Getting started](https://younes-io.github.io/ttyl/start.html)
- [Every key](https://younes-io.github.io/ttyl/keys.html)
- [Settings and troubleshooting](https://younes-io.github.io/ttyl/settings.html),
  including tmux, Nerd Fonts and images

## Development

```
git clone https://github.com/younes-io/ttyl
cd ttyl
cargo test
cargo run
```

Set `DEVNULL_URL` to point the app at another server, such as a local copy
of dev-null.sh.

`cargo test` also builds the docs site from the Markdown in `docs/` into
`target/site`. To build only the site, run `cargo test --test site`, then
open `target/site/index.html`.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
