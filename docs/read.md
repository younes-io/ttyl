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

## Open a thread

Press <kbd>enter</kbd> to open the selected commit with its replies. Press
<kbd>esc</kbd> to go back to the log.

{{screen thread}}

When the terminal is at least 110 columns wide, the thread of the selected
commit also shows beside the log, as in the screen above. On a narrower
terminal, the thread takes the whole screen.

If the commit quotes another one, press <kbd>gq</kbd> to open the quoted commit.

## Share a commit

- Press <kbd>y</kbd> to copy the link to the selected commit.
- Press <kbd>O</kbd> to open it in your browser.

Inside tmux, copying needs a tmux setting. See [Settings](settings.html#tmux).

## Read the signs

- `in review` next to a name means dev-null.sh has not approved the commit yet.
- `rejected` means dev-null.sh did not approve it.
- `↩ 5` counts replies, and `★ 27` counts stars.
- `fatal: bad object` stands where a deleted reply used to be.

## Reload and rest

Press <kbd>ctrl-r</kbd> to reload everything on screen. Press <kbd>z</kbd>
for the screensaver, and any key to come back.
