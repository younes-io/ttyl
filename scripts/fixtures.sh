#!/usr/bin/env bash
# Writes the latest page, then derives the following and trending fixtures from it, so all three share one source.
set -euo pipefail
cd "$(dirname "$0")/../fixtures"

python3 ../scripts/latest.py log-latest.json

# The viewer, ada, follows nobody: Following holds her own commits, and SuggestedCommitsQuery adds the latest commit
# of each of the three most recent other authors, with stars but no card data.
jq '.log = "following" | .meta.path |= sub("latest$"; "following")
    | .suggestions = (reduce (.data[] | select(.author.username != "ada")) as $c ([];
        if any(.[]; .author.id == $c.author.id) then . else . + [$c] end) | .[:3]
        | map(del(.replies_count, .link_preview, .images, .quoted_commit, .poll, .reply_preview)))
    | .data |= map(select(.author.username == "ada")) | .links.next = null | .meta.next_cursor = null' \
    log-latest.json > log-following.json

jq '.log = "trending" | .meta.path |= sub("latest$"; "trending") | .is_ranked = true | .data |= sort_by(-.stars_count)
    | .links.next = null | .meta.next_cursor = null' \
    log-latest.json > log-trending.json
