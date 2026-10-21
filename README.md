# devnull-tui

```
$ cat thoughts > /dev/null
```

a terminal client for [/dev/null](https://dev-null.sh), a social network
for developers where you commit plain text to a shared log. written in rust with
[ratatui](https://ratatui.rs).

## status

early. it draws the following, latest and trending logs from local fixtures.
no network yet, so nothing you do here leaves your machine. very on brand.

## run

```
cargo run --release
```

## keys

```
j / k    move down / up         (or the arrow keys)
g / G    jump to first / last
space    unfold a long commit
1 2 3    following, latest, trending
tab      next log
Q        quit                   (ctrl-c works too)
```

## tests

```
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
