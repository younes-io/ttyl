#!/usr/bin/env bash
# Derives the following and trending fixtures from the hand-written latest page, so all three share one source.
set -euo pipefail
cd "$(dirname "$0")/../fixtures"

# Mirrors SuggestedCommitsQuery: the latest commit of each of the three most recent authors.
jq '.log = "following"
    | .suggestions = (reduce .data[] as $c ([]; if any(.[]; .author.id == $c.author.id) then . else . + [$c] end) | .[:3])
    | .data = [] | .links.next = null | .meta.next_cursor = null' \
    log-latest.json > log-following.json

jq '.log = "trending" | .is_ranked = true | .data |= sort_by(-.stars_count) | .links.next = null | .meta.next_cursor = null' \
    log-latest.json > log-trending.json
