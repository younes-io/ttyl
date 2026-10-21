# devnull-tui

```
$ cat thoughts > /dev/null
```

read [/dev/null](https://dev-null.sh) without leaving your terminal. it's a
social network for developers where you commit plain text to a shared log,
and this is a keyboard-first client for it.

```
 /dev/null   1 following  2 latest  3 trending

 ┆ Today
 │
 ● Ada @ada · 5h                                                f5a6100
 ┃ switched the feed to cursorPaginate(20). offsets fall over once the
 ┃ table grows.
 ┃ ↩ 3   ★ 12
 │
 ● Grace @grace · 7h                                            0000201
 │ today's footgun:
 │
 │  $name = 'devnull';
 │  echo "<b>$name</b>";
 │
 │ escape your output.
 │ ↩ 5   ★ 27
 j/k move  space expand  1-3 log  Q quit
```

> **heads up.** this is an early preview. it shows sample posts bundled with
> the app, so you can try the interface, but it doesn't talk to the real site
> yet. signing in and posting come next.

## install

you need a recent stable [rust](https://rustup.rs) (`rustup update` if
you're not sure). then:

```
cargo install --git https://github.com/younes-io/devnull-tui
devnull
```

to remove it later, `cargo uninstall devnull`.

## the three logs

press `1`, `2` or `3` to switch, or `tab` to cycle.

- **following** is your home. it shows your commits and the people you
  follow. when it's empty it suggests a few developers to start with.
- **latest** is everyone, newest first, grouped by day.
- **trending** is what's getting attention right now. the site ranks by
  engagement with a little randomness, so popular posts don't sit on top
  forever.

## keys

| key | does |
| --- | --- |
| `j` `k` or `↓` `↑` | move to the next or previous commit |
| `g` `G` or `home` `end` | jump to the first or last commit |
| `space` | unfold a long commit, press again to fold it |
| `1` `2` `3` | following, latest, trending |
| `tab` | next log |
| `Q` | quit (capital q, so a stray keypress won't close it) |
| `ctrl-c` | quit too |

the bottom line of the screen always shows the main keys, so you don't need
to remember this table.

## reading the screen

- `●` starts a commit, with the author, how long ago, and the short hash on
  the right.
- the thick bar `┃` marks the commit you have selected.
- `↩ 3` is the reply count and `★ 12` is the star count.
- `┆ Today` marks where a new day starts.
- `… show more` means the commit is folded. press `space` to read the rest.
- `in review` next to a name means the commit is still waiting for approval.

## tips

- any terminal size works. text wraps to fit, even on a narrow split.
- colors look best in a terminal with truecolor support, which most modern
  ones have.
- use a font that has box drawing characters and emoji.

## hacking on it

```
git clone https://github.com/younes-io/devnull-tui
cd devnull-tui
cargo run
cargo test
```

besides the usual unit and screen tests, `tests/properties.rs` uses
[proptest](https://proptest-rs.github.io/proptest/) to throw random posts,
emoji and terminal sizes at the layout. it checks that every line fits the
width, no text goes missing, and the selected commit stays on screen.

## license

apache-2.0. see [LICENSE](LICENSE) and [NOTICE](NOTICE).

the server lives at
[nunomaduro/dev-null.sh](https://github.com/nunomaduro/dev-null.sh). this
client is not affiliated with it.
