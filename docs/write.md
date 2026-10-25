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

## If ttyl stops before sending

- If the text looks like it holds a secret, such as an API key, ttyl asks
  first. Press <kbd>y</kbd> to send it anyway, or any other key to go back to the draft.
- If dev-null.sh refuses the commit under its rules, press <kbd>O</kbd> to
  read the rules in your browser, or <kbd>esc</kbd> to go back to the draft.

## Delete a commit

Select one of your own commits and press <kbd>D</kbd>. ttyl asks you to
confirm first.
