# Settings and troubleshooting

ttyl has no configuration file. It reads a few environment variables and
uses what your terminal can do.

## Environment variables

| Variable | What it does |
| --- | --- |
| `TTYL_ICONS` | Set to `plain` if icons look like boxes or question marks. ttyl then uses plain Unicode symbols instead of Nerd Font icons. |
| `VISUAL`, `EDITOR` | The editor that <kbd>ctrl-e</kbd> opens in the composer. ttyl tries `$VISUAL`, then `$EDITOR`, then `vi`. |
| `DEVNULL_URL` | The server ttyl talks to. The default is `https://dev-null.sh`. Set it to use a local copy of dev-null.sh while you develop. |
| `XDG_CONFIG_HOME` | Where ttyl keeps the key this terminal signs in with, in `ttyl/device_ed25519`. The default is `~/.config`. |
| `XDG_STATE_HOME` | Where ttyl keeps your drafts, in `ttyl/drafts`. The default is `~/.local/state`. |

For example, to start ttyl with plain icons:

```sh
TTYL_ICONS=plain ttyl
```

## Terminal

Any terminal size works. Colors look best in a terminal with truecolor
support, which most current terminals have.

ttyl shows Nerd Font icons by default. If your terminal font is not a
[Nerd Font](https://www.nerdfonts.com), set `TTYL_ICONS=plain`.

Images show as real pictures in a terminal that can draw them, such as
Ghostty, kitty, WezTerm or iTerm2. Other terminals show them with colored
half-block characters. See [View images](react.html#view-images).

## tmux {#tmux}

tmux blocks two things ttyl needs by default. ttyl turns on
`allow-passthrough` for its own pane when `TERM_PROGRAM` is `tmux` or `TERM`
starts with `tmux`. If it can't tell it runs in tmux, or to turn both on for
every pane, add these lines to `~/.tmux.conf`, then run
`tmux source-file ~/.tmux.conf`:

```sh
set -g allow-passthrough on
set -g set-clipboard on
```

- `allow-passthrough` lets images through as real pictures, and lets
  <kbd>y</kbd> copy links.
- `set-clipboard` also lets <kbd>y</kbd> copy links.

## Linux {#linux}

- Install `wl-clipboard` on Wayland or `xclip` on X11. ttyl uses them to
  copy links and to paste images with <kbd>ctrl-v</kbd>.
- <kbd>O</kbd> and sign-in open your browser with `xdg-open`.
- After you approve the terminal, the browser hands the approval back to ttyl
  through a `devnull:` link handler that ttyl registers with `xdg-mime`. The
  desktop cannot launch a binary whose path has a space or a quote, so keep
  ttyl in a path without them.
- Sign in from a desktop session. The browser must run on the same computer
  as ttyl to hand the approval back.

## macOS

A binary downloaded in the browser is blocked until you remove the
quarantine flag. In the folder with the binary, run:

```sh
xattr -d com.apple.quarantine ttyl
```

Installs with Homebrew, the install script or cargo are not blocked.

<kbd>y</kbd> copies links and <kbd>ctrl-v</kbd> pastes images through the
macOS clipboard, with nothing to install. <kbd>O</kbd> and sign-in open your
browser with `open`.

## Something else

[Open an issue](https://github.com/younes-io/ttyl/issues) with your
terminal, your operating system and what you see.
