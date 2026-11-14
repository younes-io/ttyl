# Settings and troubleshooting

ttyl has no configuration file. It reads a few environment variables and
uses what your terminal can do. Your dev-null.sh account's settings live on
dev-null.sh, and ttyl reads and changes them in place.

## Account settings {#account}

Press <kbd>g,</kbd> from anywhere to open your settings: one list, under the
headings account, region, remotes, muted, blocked and devices.

- <kbd>enter</kbd> on a switch flips it, and on a choice such as your color
  moves it to the next value. The row says `saving` until dev-null.sh
  confirms. If dev-null.sh refuses, the row goes back and the bottom row says
  why.
- <kbd>enter</kbd> on the timezone or the translation language types it on the
  bottom row, the way `send at:` does. As you type, the matches show after it,
  the closest first: IANA names such as `Asia/Tokyo` for the timezone, and
  codes, English and native names for the language. <kbd>tab</kbd> moves
  through them, <kbd>ctrl-u</kbd> clears the line, <kbd>enter</kbd> saves and
  <kbd>esc</kbd> keeps what was there. An empty line unsets it.
- On someone you mute, <kbd>m</kbd> unmutes them. On someone you block,
  <kbd>B</kbd> unblocks them at once. The row stays, struck through, so the
  same key undoes it.
- Each device shows its name, the app, when it was last used and its
  fingerprint, and the terminal you are on says `this device`.
  <kbd>D</kbd> revokes one after you answer <kbd>y</kbd>. Revoking this
  device signs ttyl out, and the question says so first.

From the shell, `ttyl config` reads and writes the same settings the way
`git config` does:

```sh
ttyl config                        # every setting that is set, as key=value
ttyl config region.timezone        # one value; exits 1 when it is unset
ttyl config account.color teal     # sets it
ttyl config --unset region.language
```

A bad key or value prints a `fatal:` line and exits 1. `ttyl --help` lists
the keys and what each one takes. `ttyl muted` and `ttyl blocked` print
handles one per line. `ttyl devices` prints one device per line, tab
separated: id, last used, fingerprint and name. `ttyl devices revoke <id>`
revokes one, and only by its id.

Deleting your account, GitHub sync, live and push notifications stay on the
web.

## Environment variables

| Variable | What it does |
| --- | --- |
| `TTYL_ICONS` | Set to `plain` if icons look like boxes or question marks. ttyl then uses plain Unicode symbols instead of Nerd Font icons. You can also press <kbd>i</kbd> on the sign-in screen, and ttyl remembers the choice. |
| `TTYL_MOTION` | Set to `off` to have the selection and messages appear at once, with no fade. Stars and sparks still show, then settle. |
| `VISUAL`, `EDITOR` | The editor that <kbd>ctrl-e</kbd> opens in the composer. ttyl tries `$VISUAL`, then `$EDITOR`, then `vi`. |
| `DEVNULL_URL` | The server ttyl talks to. The default is `https://dev-null.sh`. Set it to use a local copy of dev-null.sh while you develop. |
| `LC_ALL`, `LC_MESSAGES`, `LANG` | The language <kbd>t</kbd> translates commits into, when you picked none in your dev-null.sh settings. ttyl takes the first that names a language, so `fr_FR.UTF-8` gives French. The default is English. |
| `XDG_CONFIG_HOME` | Where ttyl keeps the key this terminal signs in with, in `ttyl/device_ed25519`. The default is `~/.config`. |
| `XDG_STATE_HOME` | Where ttyl keeps your drafts, in `ttyl/drafts`, and the icons you picked, in `ttyl/icons`. The default is `~/.local/state`. |

For example, to start ttyl with plain icons:

```sh
TTYL_ICONS=plain ttyl
```

## Terminal

Any terminal size works. Colors look best in a terminal with truecolor
support, which most current terminals have.

ttyl shows Nerd Font icons by default. If your terminal font is not a
[Nerd Font](https://www.nerdfonts.com), press <kbd>i</kbd> on the sign-in screen
or set `TTYL_ICONS=plain`.

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
