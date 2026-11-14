# Messages

dev-null.sh calls a conversation a remote, and a group conversation an org.
ttyl shows yours as a list you read like a mail index, a conversation with
one person the way an IRC client shows a query window, and a group the way
it shows a channel.

## Read your conversations

Press <kbd>gm</kbd> from anywhere to list your conversations, one per line:
the person or the group, how many messages you haven't read, the first line
of the last message and how long ago it came. Pinned conversations come
first, then the ones with unread messages, then the rest, newest first.

{{screen messages}}

- Press <kbd>enter</kbd> to open the selected conversation.
- Press <kbd>esc</kbd> to go back.

## Folders

The list sits in one of three folders, named along its top the way mutt
names a mailbox:

- **open** holds the conversations you're in.
- **requests** holds messages from people you haven't written back to yet,
  with how many wait. It shows only while some do.
- **stashed** holds the conversations you put away.

Press <kbd>tab</kbd> for the next folder and <kbd>shift-tab</kbd> for the one
before.

## Keep your conversations

These keys act on the selected conversation at once and say what they did on
the bottom line. Press one again to undo it.

- Press <kbd>^</kbd> to pin it to the top, or to unpin it.
- Press <kbd>s</kbd> to stash it. A stashed conversation comes back to open
  as soon as anyone writes in it. In stashed, <kbd>s</kbd> puts it back.
- Press <kbd>m</kbd> to mute it. The bottom line asks how long, and one key
  answers: <kbd>h</kbd> for an hour, <kbd>t</kbd> till tomorrow,
  <kbd>w</kbd> for a week or <kbd>f</kbd> forever. <kbd>esc</kbd> leaves it
  as it was. On a muted conversation, <kbd>m</kbd> unmutes it.

In requests, two keys answer the selected one:

- Press <kbd>a</kbd> to merge it. It moves to open and you can write back.
- Press <kbd>D</kbd> to drop it. ttyl asks first, since it can't be undone,
  and the person isn't told.

## Find a conversation

Press <kbd>/</kbd> and type a name or a handle, then <kbd>enter</kbd>. The
folder shows only the conversations that match, and under them, the people
who match that you could write to. A person you can't write to shows why,
dimmed. Press <kbd>enter</kbd> on a person to open a conversation with them.

The find stays while you change folders. Press <kbd>esc</kbd> to show
everything again, or <kbd>/</kbd> then <kbd>enter</kbd> on nothing.

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

## Talk in a group

A group shows in the list as `#` and its name, like an IRC channel, with how
many members it has. Its last message names who wrote it. A group without a
name goes by its members' handles.

A group opens the same way a conversation does, and the same keys send,
scroll and go back. Each message shows its author's nick, yours included. A
line starting `-!-` says who added or removed someone, who left and who
renamed the group.

{{screen group}}

## Start a conversation

Press <kbd>M</kbd> on a commit to message its author, or on a person in a
profile or in search results to message them. In your conversations,
<kbd>/</kbd> finds anyone by name. If they don't take messages from you yet,
the line at the bottom says why instead of taking text.

## Message from the shell

Run `ttyl msg @grace -m "your text"` to send a message without opening ttyl.
To send longer text, pipe it in instead of passing `-m`:

```sh
git diff --stat | ttyl msg @grace
```

Run `ttyl msg` alone to print your conversations, one per line: the handle,
or `#` and the group's name, the unread count and the first line of the last
message, separated by tabs.

To send to a group, name it as `ttyl msg` prints it. Quote it, since the shell
reads `#` as the start of a comment:

```sh
ttyl msg '#unix-room' -m "reviewing now"
```

## What ttyl leaves to the web

ttyl reads and sends text. Calls, reactions, typing, mentions, replies and
running a group (adding people, renaming it) stay on dev-null.sh.
<kbd>M</kbd> and <kbd>/</kbd> start a conversation with one person; start a
group on the web.
