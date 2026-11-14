# Read

## Switch logs

ttyl has three logs. The tabs at the top of the log show which one is open.

- **Following** shows your commits and the commits of people you follow. Press <kbd>1</kbd>.
- **Latest** shows everyone, newest first. Press <kbd>2</kbd>.
- **Trending** shows what is popular right now. Press <kbd>3</kbd>.

Press <kbd>tab</kbd> to go to the next log.

{{screen latest}}

## Move through a log

- Press <kbd>j</kbd> or <kbd>↓</kbd> to move down, and <kbd>k</kbd> or <kbd>↑</kbd> to move up.
- Press <kbd>gg</kbd> to go to the top, and <kbd>G</kbd> to go to the bottom.
- Press <kbd>space</kbd> to show the rest of a long commit, such as one that
  ends in `+8 lines · space to expand`. Press it again to fold it.

Older commits load when you reach the bottom. When new commits arrive, a note
appears at the top of the log. Press <kbd>ctrl-r</kbd> to load them.

When you open ttyl again, `HEAD@{last visit}` marks where you left off in
Following and Latest. The commits above it are the ones that came in while
you were gone.

## Open a thread

Press <kbd>enter</kbd> to open the selected commit with its replies. Press
<kbd>esc</kbd> to go back to the log.

{{screen thread}}

When the terminal is at least 110 columns wide, the thread of the selected
commit also shows beside the log, as in the screen above. On a narrower
terminal, the thread takes the whole screen.

If the commit quotes another one, press <kbd>gq</kbd> to open the quoted commit.

## Search

Press <kbd>/</kbd>, type a few words and press <kbd>enter</kbd>. dev-null.sh
finds commits by meaning, not only by the words in them, and lists the
closest first. Start with `@` to find people by name or handle instead, as in
`/@grace`.

Press <kbd>enter</kbd> on a result to open its thread or the person's profile.
Press <kbd>/</kbd> again to change the search. It starts from the last one, and
<kbd>esc</kbd> still leads back to where you began.

dev-null.sh allows 15 commit searches a minute.

## Translate a commit

Press <kbd>t</kbd> to read the selected commit in your language. The
translation takes the place of the text, and the header says
`translated from French · t original`. Press <kbd>t</kbd> again for the
original, and again for the translation. ttyl keeps both until you quit, so
the commit stays translated in its thread, on profiles and in search.
While dev-null.sh translates, the header says `translating…`.

ttyl translates into the language you picked in your dev-null.sh settings.
If you picked none, it takes the language of your locale from `LC_ALL`,
then `LC_MESSAGES`, then `LANG`, so `fr_FR.UTF-8` gives French. A locale
such as `C` names no language, and ttyl skips it. With none of these, ttyl
translates into English.

If the commit is already in your language, ttyl says so. If dev-null.sh
can't translate it, ttyl shows dev-null.sh's message. Press <kbd>t</kbd> to
try again. A notification's commit translates once you open its thread.

dev-null.sh allows 10 translations a minute.

## Find commits like one

Press <kbd>gr</kbd> on a commit to list the commits closest to it in
meaning, the way the web lists them under a thread. The list is titled with
the commit's short hash, as in `related e4f5a61`, and it works wherever a
commit is selected: the log, a thread, a profile, starred commits and search
results.

Press <kbd>gr</kbd> again on one of them to keep going, and <kbd>esc</kbd> to
walk back. When nothing is close enough, ttyl says so.

## Share a commit

- Press <kbd>y</kbd> to copy the link to the selected commit.
- Press <kbd>O</kbd> to open it in your browser.

Inside tmux, copying needs a tmux setting. See [Settings](settings.html#tmux).

## Pipe a commit to a command

Press <kbd>|</kbd>, type a shell command and press <kbd>enter</kbd>. ttyl
runs it with the commit's text, as written in Markdown, on its standard
input. If the commit shows its translation, the command gets the
translation. Some things to try:

- `wl-copy` or `pbcopy` copies the text.
- `tee -a ~/notes.md` keeps it in a file.
- `less` or `vim -` opens it to read or edit.
- `llm "explain this"` asks a model about it.

If the command prints something, ttyl waits for a key so you can read it. A
command that prints nothing, such as `wl-copy`, brings you straight back.
The next <kbd>|</kbd> starts from the command you ran last.

## Read the signs

- `in review` next to a name means dev-null.sh has not approved the commit yet.
- `rejected` means dev-null.sh did not approve it.
- `↩ 5` counts replies, and `★ 27` counts stars.
- `fatal: bad object` stands where a deleted reply used to be.

## Read a log from the shell

Run `ttyl log` to print your following log the way `git log` prints commits,
then exit. Name another log with `ttyl log latest` or `ttyl log trending`. In
a terminal the log goes through `$PAGER`, or `less` if it is not set. Piped
into another program, it prints plain text.

Add `--oneline` for one line per commit: its short hash, its author and its
first line. That suits `grep` and `fzf`:

```sh
ttyl log latest --oneline | grep -i rust
```

## Reload and rest

Press <kbd>ctrl-r</kbd> to reload everything on screen. Press <kbd>z</kbd>
for the screensaver, and any key to come back.
