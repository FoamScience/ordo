#!/usr/bin/env python3
"""Turn an `ordo-engine review` result into one GitHub pull request review:
an inline comment on each hunk, numbered in ordo's reading order, each linking
to the next, so GitHub's own diff view is read in ordo's order.

usage: .github/ordo/pr-review.py ORDO_JSON PR_DIFF --repo OWNER/NAME --pr N --commit SHA
       .github/ordo/pr-review.py --self-test

PR_DIFF is the diff GitHub shows (`git diff BASE...HEAD`, default context):
a review comment must sit on a line of it, so a hunk whose lines it does not
show is listed in the review body instead. Prints the payload for
POST /repos/{repo}/pulls/{pr}/reviews.
"""
import argparse
import hashlib
import json
import re

MARKER = "<!-- ordo-review-inline -->"
MAX_COMMENTS = 50

HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def commentable(diff: str) -> dict:
    """{path: {"LEFT": old lines, "RIGHT": new lines}} a review comment may sit on."""
    out, path, old, new, left, right = {}, None, 0, 0, 0, 0
    for line in diff.splitlines():
        if left or right:
            # inside a hunk the header's counts decide, so a removed `-- x`
            # line is content, not a `--- ` file header
            tag = line[:1] or " "
            if tag in "- ":
                out[path]["LEFT"].add(old)
                old, left = old + 1, left - 1
            if tag in "+ ":
                out[path]["RIGHT"].add(new)
                new, right = new + 1, right - 1
        elif line.startswith("--- "):
            minus = line[6:] if line.startswith("--- a/") else None
        elif line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else minus
            out.setdefault(path, {"LEFT": set(), "RIGHT": set()})
        elif m := HUNK.match(line):
            old, left, new, right = int(m[1]), int(m[2] or 1), int(m[3]), int(m[4] or 1)
    return out


def anchor(hunk: dict, lines: dict) -> tuple[str, int] | None:
    """The first line of the hunk GitHub lets a comment sit on, new side first."""
    for side, key in (("RIGHT", "new_range"), ("LEFT", "old_range")):
        lo, hi = hunk[key]
        for n in range(lo, hi + 1):
            if n in lines.get(side, ()):
                return side, n
    return None


def link(repo: str, pr: int, path: str, side: str, line: int) -> str:
    h = hashlib.sha256(path.encode()).hexdigest()
    return f"https://github.com/{repo}/pull/{pr}/files#diff-{h}{side[0]}{line}"


def payload(result: dict, diff: str, repo: str, pr: int, commit: str) -> dict:
    hunks = {(f["path"], h["id"]): {**h, "path": f["path"]} for f in result["files"] for h in f["hunks"]}
    rows = [h for o in result["order"] if (h := hunks.get((o["path"], o["hunk"]))) and not h.get("noise")]
    lines = commentable(diff)
    placed, missed = [], []
    for h in rows:
        at = anchor(h, lines.get(h["path"], {}))
        (placed if at and len(placed) < MAX_COMMENTS else missed).append((h, at))

    def where(h, at):
        return f"[`{h['path']}` L{at[1]}]({link(repo, pr, h['path'], *at)})"

    comments = []
    for i, (h, at) in enumerate(placed):
        body = [f"{MARKER}**ordo {i + 1}/{len(placed)}** · {h.get('rationale') or h['category']}"]
        body += [f"- **{f['level']}** {f['name']}: {f['message']}" for f in h.get("findings", [])]
        if i + 1 < len(placed):
            body.append(f"\nnext: {where(*placed[i + 1])}")
        comments.append({"path": h["path"], "side": at[0], "line": at[1], "body": "\n".join(body)})

    body = [f"{MARKER}**ordo** reading order: {len(placed)} of {len(rows)} hunks commented, numbered in order."]
    if placed:
        body.append(f"Start at {where(*placed[0])}.")
    if missed:
        body.append("\nNot commented (outside GitHub's diff view, or past the cap):")
        body += [f"- `{h['path']}` L{h['new_range'][0]}: {h.get('rationale') or h['category']}" for h, _ in missed]
    if result.get("notes"):
        body += ["\nNotes:"] + [f"- {n}" for n in result["notes"]]
    return {"commit_id": commit, "event": "COMMENT", "body": "\n".join(body), "comments": comments}


def self_test():
    diff = """diff --git a/m.py b/m.py
--- a/m.py
+++ b/m.py
@@ -1,3 +1,3 @@
-def a():
+def a(k):
     return 1

@@ -10,2 +10,1 @@ def c():
     x = 1
-    y = 2
@@ -20 +19 @@
--- lua comment
+++ lua comment
diff --git a/gone.py b/gone.py
--- a/gone.py
+++ /dev/null
@@ -1 +0,0 @@
-x = 1
"""
    c = commentable(diff)
    assert c["m.py"] == {"LEFT": {1, 2, 3, 10, 11, 20}, "RIGHT": {1, 2, 3, 10, 19}}
    assert c["gone.py"] == {"LEFT": {1}, "RIGHT": set()}
    assert anchor({"new_range": [1, 1], "old_range": [1, 1]}, c["m.py"]) == ("RIGHT", 1)
    assert anchor({"new_range": [11, 10], "old_range": [11, 11]}, c["m.py"]) == ("LEFT", 11)
    assert anchor({"new_range": [50, 52], "old_range": [50, 52]}, c["m.py"]) is None
    result = {"notes": ["n"], "order": [{"path": "m.py", "hunk": "h1"}, {"path": "m.py", "hunk": "h0"},
                                        {"path": "m.py", "hunk": "h2"}, {"path": "m.py", "hunk": "h3"}],
              "files": [{"path": "m.py", "hunks": [
                  {"id": "h0", "new_range": [1, 1], "old_range": [1, 1], "category": "definition",
                   "rationale": "changes signature of a",
                   "findings": [{"level": "note", "name": "r", "message": "m"}]},
                  {"id": "h1", "new_range": [11, 10], "old_range": [11, 11], "category": "other"},
                  {"id": "h2", "new_range": [50, 50], "old_range": [50, 50], "category": "other"},
                  {"id": "h3", "new_range": [2, 2], "old_range": [2, 2], "category": "other", "noise": True}]}]}
    p = payload(result, diff, "o/r", 7, "abc")
    assert [(x["side"], x["line"]) for x in p["comments"]] == [("LEFT", 11), ("RIGHT", 1)]
    assert p["comments"][0]["body"].startswith(MARKER + "**ordo 1/2** · other")
    h = hashlib.sha256(b"m.py").hexdigest()
    assert f"next: [`m.py` L1](https://github.com/o/r/pull/7/files#diff-{h}R1)" in p["comments"][0]["body"]
    assert "- **note** r: m" in p["comments"][1]["body"] and "next:" not in p["comments"][1]["body"]
    assert "2 of 3 hunks" in p["body"] and "`m.py` L50: other" in p["body"] and "- n" in p["body"]
    assert p["commit_id"] == "abc" and p["event"] == "COMMENT"
    print("ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("result", nargs="?")
    ap.add_argument("diff", nargs="?")
    ap.add_argument("--repo")
    ap.add_argument("--pr", type=int)
    ap.add_argument("--commit")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not (a.result and a.diff and a.repo and a.pr and a.commit):
        ap.error("ORDO_JSON, PR_DIFF, --repo, --pr and --commit are required")
    with open(a.result) as r, open(a.diff) as d:
        print(json.dumps(payload(json.load(r), d.read(), a.repo, a.pr, a.commit)))


if __name__ == "__main__":
    main()
