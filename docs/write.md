# Write

## Write a commit, reply or quote

- Press <kbd>c</kbd> to write a new commit.
- Press <kbd>r</kbd> to reply to the selected commit.
- Press <kbd>q</kbd> to quote it in a new commit.

The composer opens at the bottom of the screen. Type your text, then press
<kbd>ctrl-s</kbd> to send it.

To write in your own editor instead, press <kbd>ctrl-e</kbd>. ttyl opens
`$VISUAL`, or `$EDITOR` if `$VISUAL` is not set, or `vi` if neither is. Save
and quit the editor to come back with your text.

To add an emoji, type `:` and the start of its name, then pick one from the list.

## Keep a draft

Press <kbd>esc</kbd> to close the composer. ttyl keeps the draft, even after
you quit, and gives it back the next time you write the same commit, reply or
quote.

## Preview before you send

Press <kbd>ctrl-r</kbd> in the composer to see the draft as the commit it
would become, with code blocks in color. dev-null.sh renders the preview, so
it matches what other people see.

{{screen preview}}

Press <kbd>esc</kbd> to go back to writing, or <kbd>ctrl-s</kbd> to send it
from the preview.

## Add a poll

Only a new commit can carry a poll.

1. Press <kbd>ctrl-p</kbd> in the composer.
2. Type the first option. Press <kbd>enter</kbd> to add the next one. A poll has up to four options.
3. Press <kbd>tab</kbd> or <kbd>↓</kbd> to move from the last option to how long the poll lasts.
4. Press <kbd>←</kbd> or <kbd>→</kbd> to change how long it lasts.

{{screen compose}}

Press <kbd>backspace</kbd> on an empty option to remove it. Press
<kbd>ctrl-p</kbd> again to take the poll off.

## Attach images

A new commit can carry up to three images. To attach one, do one of these:

- Drop the file on the terminal window.
- Paste the path of the file.
- Copy an image or an image file, then press <kbd>ctrl-v</kbd>. On Linux,
  this reads the clipboard through `wl-clipboard` or `xclip`.

Each image gets a description line, which people who cannot see the image
read instead. Press <kbd>tab</kbd> to move to it and type the description.

{{screen attach}}

Press <kbd>backspace</kbd> on an empty description to remove its image.

## Send it later

A new commit can wait and go out at a time you pick, up to 30 days ahead.

1. Write the commit, then press <kbd>ctrl-t</kbd> in the composer.
2. Type when it goes out after `send at:`. ttyl shows the date and time it
   reads as you type.
3. Press <kbd>enter</kbd> to schedule it, or <kbd>esc</kbd> to go back to the draft.

ttyl reads times the way `at` does, in your time zone:

| You type | It goes out |
| --- | --- |
| `14:30` or `2:30pm` | the next 14:30, today or tomorrow |
| `tomorrow 9:00` | tomorrow at 9:00 |
| `fri 14:00` | the coming Friday at 14:00 |
| `in 2h`, `in 30m`, `in 1h30m` | that long from now |
| `2026-10-24T09:00:00+02:00` | that moment |

Press <kbd>g</kbd> <kbd>l</kbd> to see what is scheduled, soonest first.
Select one and press <kbd>S</kbd> to commit it now, or <kbd>D</kbd> to cancel
it. ttyl asks before it cancels. A scheduled commit can't be edited, so to
change one, cancel it and write it again.

## Commit from the shell

Run `ttyl commit -m "your text"` to send a commit without opening ttyl. To
send longer text, pipe it in instead of passing `-m`:

```sh
git log -1 --format=%B | ttyl commit
```

ttyl prints the commit's short hash, its first line and its link. Your first
commit still has to come from the web, so you can read the commit rules.

Add `--at` to schedule it instead. It takes the same times as `send at:`:

```sh
ttyl commit --at 'tomorrow 9:00' -m "standup notes are up"
```

ttyl prints when it will go out, such as `scheduled for Thu 22 Oct 09:00`.

Add `--as` to post as an organization you're a member of. It refuses with
`fatal: you are not a member of @handle` before it reads anything:

```sh
ttyl commit --as @acme -m "v2 is out"
```

## Post as an organization

If you're a member of an organization, the composer title says who the
commit goes out as, such as `commit as @you`. Press <kbd>ctrl-x</kbd> to
switch to each of your organizations in turn, then back to you. The next
commit, reply or quote starts as whoever you picked last. ttyl forgets your
choice when it quits, so it never posts as an organization by surprise.

## If ttyl stops before sending

- If the text looks like it holds a secret, such as an API key, ttyl asks
  first. Press <kbd>y</kbd> to send it anyway, or any other key to go back to the draft.
- If dev-null.sh refuses the commit under its rules, press <kbd>O</kbd> to
  read the rules in your browser, or <kbd>esc</kbd> to go back to the draft.

## Delete a commit

Select one of your own commits and press <kbd>D</kbd>. ttyl asks you to
confirm first.

Changed your mind? Press <kbd>u</kbd> within 30 seconds to bring the commit
back where it was, replies and stars included.

## Pin a commit to your profile

Select one of your own commits, not a reply, and press <kbd>^</kbd> to pin it
to the top of your profile. Its card shows `pinned` wherever it appears, and
your profile lists it first under a `pinned` label.

You can pin one commit at a time. Pinning another one replaces it, and the
note at the bottom names the one it replaced, such as
`pinned to your profile in place of e4f5a61`. Press <kbd>^</kbd> on the pinned
commit to unpin it. It goes back among your commits by date.
