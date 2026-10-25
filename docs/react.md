# Star, vote and view images

## Star a commit

Press <kbd>s</kbd> to star the selected commit. Press it again to take the
star back. Your starred commits are listed under <kbd>gs</kbd>, as
[Notifications and people](people.html#starred) shows.

## Give a pastel

When pastéis mode is on for your dev-null.sh account, press <kbd>P</kbd> to
give a pastel to the author of the selected commit. You cannot give one to
yourself. After you give one, ttyl tells you how many you have left today.

## Vote in a poll

1. Select a commit with a poll and press <kbd>v</kbd>.
2. Pick an option with <kbd>j</kbd> and <kbd>k</kbd>, or press its number.
3. Press <kbd>enter</kbd> to vote. A vote is final.

{{screen vote}}

Press <kbd>esc</kbd> to leave without voting.

## View images

A commit with images lists them as `▣` lines with their descriptions. Press
<kbd>i</kbd> to view them inside the terminal.

- Press <kbd>l</kbd> for the next image and <kbd>h</kbd> for the previous one.
- Press <kbd>O</kbd> to open the image you see in your browser.
- Press <kbd>esc</kbd> to close the viewer.

{{screen images}}

How the image looks depends on your terminal. A terminal that can draw
pictures, such as Ghostty, kitty, WezTerm or iTerm2, shows the image itself.
Other terminals get the blocky version above, drawn with half-block
characters in color, two pixels to a cell. ttyl asks the terminal which one it
can do when it starts. Inside tmux, ttyl turns on `allow-passthrough` for
its own pane to get the real picture. See [Settings](settings.html#tmux).

<div class="side-by-side">
<figure>
<img src="desk.png" width="800" height="600" alt="A pixel drawing of a desk with two monitors that show colored lines of code.">
<figcaption>The picture the screen above shows. The commit's real image lives on dev-null.sh, so the site build draws this one and gives it to the viewer in its place.</figcaption>
</figure>
</div>

## Report a commit

1. Press <kbd>!</kbd> on the commit.
2. Pick spam, harassment, unsafe or other with <kbd>j</kbd> and <kbd>k</kbd>,
   or press its number from <kbd>1</kbd> to <kbd>4</kbd>. Press <kbd>enter</kbd>.
3. Add details if you want, then press <kbd>enter</kbd> to send the report.

Press <kbd>esc</kbd> at any step to cancel.
