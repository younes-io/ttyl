# Notifications and people

## Read notifications

The number at the top right, such as `● 3`, counts your unread
notifications. Press <kbd>gn</kbd> from anywhere to read them. When new ones
arrive, the count lights up and a note at the bottom says how many came in.

{{screen notifications}}

- Press <kbd>tab</kbd> to show only replies, quotes, stars, mentions or
  follows, and <kbd>shift-tab</kbd> to go back a filter.
- Press <kbd>enter</kbd> to open a notification. A reply or a quote opens its
  thread. A group, such as `Grace and 4 others starred your commit`, opens the
  list of everyone in it.
- Press <kbd>esc</kbd> to go back.

Run `ttyl status` to print the unread count and exit. It prints only the
number, so a status line can show it. In a terminal, it also prints how many
commits you have scheduled, when there are any. For example, add this to
`~/.tmux.conf`:

```
set -g status-right '#(ttyl status)'
```

## Open a profile

Press <kbd>p</kbd> to open the profile of the selected commit's author. A
profile shows the bio, where the person is, when they joined and their
counts, above their commits.

{{screen profile}}

- Press <kbd>tab</kbd> to move between commits, replies, followers and
  following, and <kbd>shift-tab</kbd> to move back.
- Press <kbd>f</kbd> to follow or unfollow the person.
- Press <kbd>gw</kbd> to open their website, <kbd>gh</kbd> their GitHub, or
  <kbd>gd</kbd> their page on dev-null.sh.

In the followers and following tabs, press <kbd>enter</kbd> to open a
person's profile, and <kbd>f</kbd> to follow them. The following tab lists
the organizations they follow first, each marked `org`.

## Open an organization

A commit an organization posted reads `@handle · org` on its card. Press
<kbd>p</kbd> on it to open the organization's page. It has the same layout as
a person's profile. The header shows the bio, where the organization is, its
links, its followers and who runs it. Below the header, <kbd>tab</kbd> moves
between the commits it posted and the commits that mention it.

- Press <kbd>f</kbd> to follow or unfollow the organization, even when it has
  posted nothing yet.
- Press <kbd>gw</kbd>, <kbd>gh</kbd> or <kbd>gd</kbd> to open its website,
  its GitHub or its page on dev-null.sh.

You can't mute, block or message an organization. Those keys do nothing on
its commits.

## Follow from a log

Press <kbd>f</kbd> on a commit to follow or unfollow its author without
opening the profile. Their commits then show in your following log.

## Mute someone

Press <kbd>m</kbd> on a commit to mute its author. Every commit of theirs on
screen folds into a `muted @name` line at once, and dev-null.sh leaves them
out of your logs from then on. They are not told. Press <kbd>m</kbd> on a
folded line to unmute them.

## Block someone

Press <kbd>B</kbd> on a commit to block its author. Blocking goes further
than muting. It ends the follows both ways, and neither of you can see,
reply to or star the other's commits. So ttyl asks first, on the bottom row:

```
block @grace? this ends follows both ways [y/N]
```

Press <kbd>y</kbd> to block, or any other key to keep things as they are.
Every commit of theirs on screen then folds into a
`blocked @grace · B to unblock` line, and so does a notification about them
alone: a reply, quote, mention or follow. A group of several people stays as
it is. Press <kbd>B</kbd> on a folded line to unblock them, with no question
asked. Unblocking doesn't bring the follows back, and they no longer show as
following you.

<kbd>B</kbd> also works on a person in the followers and following tabs of a
profile, in people search results, and in a group of people who followed you.

Run `ttyl blocked` to print the handles you block, one per line.

## See your starred commits {#starred}

Press <kbd>gs</kbd> from anywhere to list the commits you starred, newest
star first.

{{screen starred}}
