"""Writes fixtures/log-latest.json in the shape CommitResource gives a signed log request.

Bodies are split the way app/Services/CodeSpans.php splits them and cut the way CommitBodies::excerpt cuts them,
so body_segments and preview_segments agree with the body. Keys follow CommitResource's order.
"""
import base64
import html
import json
import re
import sys

SITE = "https://dev-null.sh"
PATTERN = re.compile(r"((?<![^\n])`{3,}[^`\r\n]*\r?\n(?:[\s\S]*?\r?\n)?`{3,}[ \t]*(?![^\r\n])|`[^`\r\n]+`)")
PREVIEW_LENGTH = 280
PREVIEW_LINES = 8

PHP_HTML = (
    '<span class="hl-variable">$name</span> = <span class="hl-value">\'devnull\'</span>;\n'
    '<span class="hl-keyword">echo</span> <span class="hl-value">&quot;&lt;b&gt;$name&lt;/b&gt;&quot;</span>;'
)


def split(text):
    return [text] if "`" not in text else PATTERN.split(text)


def is_block(part):
    return part is not None and part.startswith("```")


def block(part):
    lines = part.split("\n")
    language = lines[0].lstrip("`").split()[0] if lines[0].lstrip("`").split() else None
    code = "\n".join(lines[1:-1])
    return {"type": "block", "language": language, "html": PHP_HTML if language == "php" else html.escape(code, quote=True)}


def highlight(text):
    if "`" not in text:
        return [] if text == "" else [{"type": "text", "text": text}]
    parts = split(text)
    segments = []
    for index, part in enumerate(parts):
        if index % 2 == 1:
            segments.append(block(part) if is_block(part) else {"type": "code", "code": part[1:-1]})
            continue
        if index > 0 and is_block(parts[index - 1]) and part.startswith("\n"):
            part = part[1:]
        if index + 1 < len(parts) and is_block(parts[index + 1]) and part.endswith("\n"):
            part = part[:-1]
        if part:
            segments.append({"type": "text", "text": part})
    return segments


def limit(body):
    lines = body.split("\n")
    if len(lines) > PREVIEW_LINES:
        return min(PREVIEW_LENGTH, len("\n".join(lines[:PREVIEW_LINES])))
    return PREVIEW_LENGTH if len(body) > PREVIEW_LENGTH else None


def cut_prose(prose, at, is_first):
    cut = prose[:at]
    if prose[at:at + 1].isspace():
        return cut
    match = re.match(r"\A(.*)\s", cut, re.S)
    if match:
        return match.group(1)
    return cut if is_first else ""


def excerpt(body):
    at = limit(body)
    if at is None:
        return None
    if "`" not in body:
        return cut_prose(body, at, True).rstrip() + "…"
    out = ""
    for index, part in enumerate(split(body)):
        if len(part) <= at:
            out += part
            at -= len(part)
            continue
        is_first = out.strip() == ""
        if index % 2 == 0:
            out += cut_prose(part, at, is_first)
        elif is_first:
            raise SystemExit("cutting into a leading code block is not needed by these fixtures")
        break
    out = out.rstrip()
    return out + "\n…" if out.endswith("```") else out + "…"


def short_hash(commit_id):
    return commit_id.replace("-", "")[-7:]


def cursor(created_at, commit_id):
    payload = {"created_at": created_at.replace("T", " ").replace("Z", ""), "id": commit_id, "_pointsToNextItems": True}
    return base64.urlsafe_b64encode(json.dumps(payload, separators=(",", ":")).encode()).decode().rstrip("=")


def user(username, name, number, lane, joined, avatar=None, bio=None, location=None):
    return {
        "id": f"{number:02x}000000-0000-4000-8000-0000000000{number:02x}",
        "username": username,
        "name": name,
        "avatar_url": avatar,
        "bio": bio,
        "company": None,
        "location": location,
        "website": None,
        "github_url": f"https://github.com/{username}",
        "html_url": f"{SITE}/@{username}",
        "feed_url": f"{SITE}/@{username}/feed.atom",
        "created_at": joined,
        "lane": lane,
    }


ADA = user("ada", "Ada", 0x0A, 0, "2026-09-01T10:00:00Z", "https://avatars.githubusercontent.com/u/1?v=4", "compilers, mostly", "London")
GRACE = user("grace", "Grace", 0x0B, "#7048e8", "2026-09-02T10:00:00Z")
LINUS = user("linus", "Linus", 0x0C, 3, "2026-10-21T07:00:00Z")
MARGARET = user("margaret", "Margaret", 0x0D, 5, "2026-09-03T10:00:00Z")
KEN = user("ken", "Ken", 0x0E, 1, "2026-09-04T10:00:00Z")


def mentions(body):
    return [
        {"username": m.group(1), "offset": m.start(), "length": len(m.group(0))}
        for m in re.finditer(r"(?<![\w@])@([A-Za-z0-9-]+)", body)
    ]


def signed(commit_id, author, body, created_at, parent_id=None, status="approved", quoted_commit_id=None):
    """The keys every signed commit has, in CommitResource's order, before the counted and loaded ones."""
    preview = excerpt(body)
    return {
        "id": commit_id,
        "short_hash": short_hash(commit_id),
        "body": body,
        "parent_id": parent_id,
        "html_url": f"{SITE}/@{author['username']}/commits/{commit_id}",
        "created_at": created_at,
        "author": author,
        "body_segments": highlight(body),
        "preview_segments": None if preview is None else highlight(preview),
        "mentions": mentions(body),
        "status": status,
        "is_pinned": False,
        "pasteis_count": 0,
        "deleted_at": None,
        "quoted_commit_id": quoted_commit_id,
    }


ORDER = [
    "id", "short_hash", "body", "parent_id", "html_url", "created_at", "stars_count", "replies_count",
    "link_preview", "images", "author", "body_segments", "preview_segments", "mentions", "status", "is_pinned",
    "starred", "pasteis_count", "deleted_at", "quoted_commit_id", "quoted_commit", "poll", "reply_preview",
]


def ordered(fields):
    return {key: fields[key] for key in ORDER if key in fields}


def quote(commit, link_preview=None, images=(), poll=None):
    return ordered({**commit, "link_preview": link_preview, "images": list(images), "poll": poll})


def card(commit, stars, starred, replies, link_preview=None, images=(), poll=None):
    return {**commit, "stars_count": stars, "starred": starred, "replies_count": replies,
            "link_preview": link_preview, "images": list(images), "poll": poll}


def reply(commit, stars=0, starred=False, replies=0, **media):
    return ordered(card(commit, stars, starred, replies, **media))


def tombstone(commit_id, parent_id, deleted_at, replies_count=None):
    out = {"id": commit_id, "parent_id": parent_id, "deleted_at": deleted_at}
    if replies_count is not None:
        out["replies_count"] = replies_count
    return out


def row(commit, stars, starred, replies, preview=(), more=None, quoted=None, **media):
    return ordered({
        **card(commit, stars, starred, replies, **media),
        "quoted_commit": quoted,
        "reply_preview": {"data": list(preview), "next_cursor": more},
    })


def image(number, width, height, description):
    image_id = f"7a000000-0000-4000-8000-0000000007{number:02x}"
    return {"id": image_id, "width": width, "height": height, "description": description, "url": f"{SITE}/commit-images/{image_id}"}


SCHEDULER_ID = "4e5f6071-0000-4000-8000-000000000401"
SCHEDULER = signed(SCHEDULER_ID, MARGARET, (
    "notes from rewriting our scheduler.\n\n"
    "1. the queue was never the bottleneck.\n"
    "2. the lock around the queue was.\n"
    "3. we held it across an fsync.\n"
    "4. nobody noticed for two years because the disks were fast.\n"
    "5. then we moved to network storage.\n"
    "6. p50 went from 2ms to 90ms overnight.\n"
    "7. the fix was four lines.\n"
    "8. finding it took three weeks.\n"
    "9. write down what your locks protect.\n"
    "10. and measure before you guess."
), "2026-10-20T22:40:00Z")

CURSOR_ID = "9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61"
FOOTGUN_ID = "2c3d4e5f-0000-4000-8000-000000000201"
DOTFILES_ID = "5f607182-0000-4000-8000-000000000501"
LAST_ID = DOTFILES_ID
LAST_AT = "2026-10-20T17:03:00Z"

rows = [
    row(
        signed(CURSOR_ID, ADA, "switched the feed to `cursorPaginate(20)`. offsets fall over once the table grows.", "2026-10-21T09:42:00Z"),
        12, True, 3,
        preview=[reply(signed("1b2c3d4e-0000-4000-8000-000000000101", GRACE, "@ada did you keep an index on created_at?",
                              "2026-10-21T09:50:00Z", parent_id=CURSOR_ID), stars=1)],
        more=cursor("2026-10-21T09:50:00Z", "1b2c3d4e-0000-4000-8000-000000000101"),
    ),
    row(
        signed("9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a62", ADA, "and the p99 dropped from 480ms to 31ms. same lesson as this:",
               "2026-10-21T09:31:00Z", status="pending", quoted_commit_id=SCHEDULER_ID),
        0, False, 0,
        quoted=quote(SCHEDULER),
    ),
    row(
        signed(FOOTGUN_ID, GRACE, "today's footgun:\n\n```php\n$name = 'devnull';\necho \"<b>$name</b>\";\n```\nescape your output.",
               "2026-10-21T08:05:00Z"),
        27, False, 5,
        preview=[tombstone("1b2c3d4e-0000-4000-8000-000000000202", FOOTGUN_ID, "2026-10-21T08:30:00Z", replies_count=0)],
        more=cursor("2026-10-21T08:20:00Z", "1b2c3d4e-0000-4000-8000-000000000202"),
    ),
    row(signed("3d4e5f60-0000-4000-8000-000000000301", LINUS, "first commit here. hello /dev/null.", "2026-10-21T07:12:00Z"), 0, False, 0),
    row(
        SCHEDULER, 88, True, 14,
        preview=[reply(signed("1b2c3d4e-0000-4000-8000-000000000401", ADA, "@margaret the fsync one gets everybody once.",
                              "2026-10-21T06:58:00Z", parent_id=SCHEDULER_ID), stars=6, starred=False)],
        more=cursor("2026-10-21T06:58:00Z", "1b2c3d4e-0000-4000-8000-000000000401"),
    ),
    row(
        signed("6a708193-0000-4000-8000-000000000601", GRACE, "the case against OFFSET, with numbers: https://use-the-index-luke.com/no-offset",
               "2026-10-20T21:10:00Z"),
        9, False, 0,
        link_preview={
            "url": "https://use-the-index-luke.com/no-offset",
            "title": "We need tool support for keyset pagination",
            "description": "Offset pagination gets slower the further you page. Keyset pagination does not.",
            "image_url": None,
            "site_name": "Use The Index, Luke",
            "is_pending": False,
            "embed": None,
        },
    ),
    row(
        signed("6b8192a4-0000-4000-8000-000000000701", KEN, "new desk, same bugs.", "2026-10-20T20:30:00Z"),
        15, False, 0,
        images=[image(1, 1600, 1200, "a desk with two monitors"), image(2, 1200, 1600, None)],
    ),
    row(
        signed("6c92a3b5-0000-4000-8000-000000000801", MARGARET, "settling this for the team. tabs or spaces?", "2026-10-20T19:45:00Z"),
        4, False, 0,
        poll={
            "id": "8a000000-0000-4000-8000-000000000801",
            "closes_at": "2026-10-23T19:45:00.000000Z",
            "is_closed": False,
            "has_voted": False,
            "total_votes": None,
            "options": [
                {"id": "8b000000-0000-4000-8000-000000000801", "label": "tabs", "votes_count": None, "voted": False, "percentage": None},
                {"id": "8b000000-0000-4000-8000-000000000802", "label": "spaces", "votes_count": None, "voted": False, "percentage": None},
            ],
        },
    ),
    row(
        signed("6da3b4c6-0000-4000-8000-000000000901", ADA, "testing whether links to my newsletter get through.", "2026-10-20T18:20:00Z", status="rejected"),
        0, False, 0,
    ),
    row(
        signed("6eb4c5d7-0000-4000-8000-000000000a01", LINUS, "this aged well.", "2026-10-20T17:40:00Z",
               quoted_commit_id="6fc5d6e8-0000-4000-8000-000000000b01"),
        2, False, 0,
        quoted=tombstone("6fc5d6e8-0000-4000-8000-000000000b01", None, "2026-10-20T17:50:00Z"),
    ),
    row(
        signed(DOTFILES_ID, KEN, "my whole dotfiles install, still:\n\n```\nx\ny\n```", LAST_AT),
        3, False, 1,
        preview=[reply(signed("1b2c3d4e-0000-4000-8000-000000000501", GRACE, "honest.", "2026-10-20T17:30:00Z", parent_id=DOTFILES_ID))],
    ),
]

next_cursor = cursor(LAST_AT, LAST_ID)
page = {
    "data": rows,
    "links": {"first": None, "last": None, "prev": None, "next": f"{SITE}/api/v1/log/latest?cursor={next_cursor}"},
    "meta": {"path": f"{SITE}/api/v1/log/latest", "per_page": 20, "next_cursor": next_cursor, "prev_cursor": None},
    "log": "latest",
    "is_ranked": False,
    "suggestions": [],
}

with open(sys.argv[1], "w") as out:
    json.dump(page, out, indent=2, ensure_ascii=False)
    out.write("\n")
