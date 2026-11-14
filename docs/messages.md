# Messages

dev-null.sh calls a one-to-one conversation a remote. ttyl shows yours as a
list you read like a mail index, and each conversation the way an IRC client
shows a query window.

## Read your conversations

Press <kbd>gm</kbd> from anywhere to list your conversations, one per line:
the person, how many of their messages you haven't read, the first line of
the last message and how long ago it came. Pinned conversations come first,
then the ones with unread messages, then the rest, newest first.

{{screen messages}}

- Press <kbd>enter</kbd> to open the selected conversation.
- Press <kbd>esc</kbd> to go back.

## Talk in a conversation

A conversation reads oldest at the top and newest at the bottom, where the
view stays as new messages arrive. Opening it marks it read on dev-null.sh,
and while it is open, ttyl checks for new messages every few seconds.

{{screen conversation}}

The line at the bottom is where you type, and it always has the keys:

- Press <kbd>enter</kbd> to send. The message shows dimmed until dev-null.sh
  has it.
- Press <kbd>ctrl-e</kbd> to write a longer message in your `$EDITOR`.
- Press <kbd>pgup</kbd> and <kbd>pgdn</kbd> to scroll. Scrolling up past the
  oldest message loads the ones before it. A line at the bottom says when new
  messages came in below.
- Press <kbd>esc</kbd> to go back. What you typed stays until you leave
  ttyl.

A terminal can't show a message's image or play its voice note, so they show
as `[image]` and `[voice note]`. Open the conversation on dev-null.sh to see
or hear them.

## Start a conversation

Press <kbd>M</kbd> on a commit to message its author, or on a person in a
profile or in search results to message them. If they don't take messages
from you yet, the line at the bottom says why instead of taking text.

## Message from the shell

Run `ttyl msg @grace -m "your text"` to send a message without opening ttyl.
To send longer text, pipe it in instead of passing `-m`:

```sh
git diff --stat | ttyl msg @grace
```

Run `ttyl msg` alone to print your conversations, one per line: the handle,
the unread count and the first line of the last message, separated by tabs.

## What ttyl leaves to the web

ttyl reads and sends one-to-one text. Calls, reactions, typing, pins,
archiving and conversations with an org stay on dev-null.sh, and org
conversations don't show in the list.
