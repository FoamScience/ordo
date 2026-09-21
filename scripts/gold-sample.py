#!/usr/bin/env python3
"""Draw the gold set: ~200 corpus hunks, stratified so every rationale template,
every language, noise hunks and hunks carrying findings all appear.

Writes corpus/gold.jsonl with the labels left null; scripts/gold-label.py fills
them in. Deterministic for a given corpus and seed, so a re-draw after a
manifest bump produces the same picks where the commits still exist.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/gold-sample.py [--n 200] [--per-repo 60]
"""
import argparse
import json
import os
import random
import re
import subprocess
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / "target/release/ordo-engine"
OUT = ROOT / "corpus/gold.jsonl"


def repos():
    text = (ROOT / "corpus/manifest.toml").read_text()
    field = lambda b, k: (m.group(1) if (m := re.search(rf'^{k}\s*=\s*"?([^"\n]+)"?', b, re.M)) else None)
    for b in text.split("[[repo]]")[1:]:
        yield field(b, "name"), field(b, "lang"), field(b, "rev")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True,
                          errors="replace").stdout


def commit_output(repo, sha):
    parent = git(repo, "rev-parse", "--verify", "-q", f"{sha}^").strip()
    if not parent:
        return None, None
    # every text file goes in; the engine flags what it has no grammar for and
    # the caller drops those, so the supported-extension list lives in one place
    names = [l.split("\t", 2)[2] for l in git(repo, "diff", "--numstat", "--no-renames", parent, sha).splitlines()
             if not l.startswith("-\t-\t")]
    if not names:
        return None, None
    inp = {"changes": [{"path": p, "old": git(repo, "show", f"{parent}:{p}"),
                        "new": git(repo, "show", f"{sha}:{p}")} for p in names]}
    r = subprocess.run([ENGINE, "order", "--json"], input=json.dumps(inp),
                       capture_output=True, text=True)
    return parent, json.loads(r.stdout)


def hunk_diff(repo, parent, sha, path, new_range):
    """The -U3 hunk of `git diff` that contains the hunk's first new line, or
    None when git and ordo disagree about where the change is — such a hunk
    cannot be labeled against the right text and is left out of the pool."""
    text = git(repo, "diff", "-U3", parent, sha, "--", path)
    for m in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@.*$", text, re.M):
        start, count = int(m.group(1)), int(m.group(2) or 1)
        if start <= new_range[0] <= start + max(count, 1):
            end = text.find("\n@@ ", m.end())
            return text[m.start():end if end > 0 else None]
    return None


def template(rationale):
    return rationale.split(" ", 1)[0] if rationale else "change"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=200)
    ap.add_argument("--per-repo", type=int, default=60, help="commits swept per repo")
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus"))

    pool, unlocated = [], 0
    for name, lang, rev in repos():
        repo = corpus / name
        if not repo.is_dir():
            continue
        listing = git(repo, "rev-list", "--max-count", str(a.per_repo), "--format=%s", rev).splitlines()
        shas = [(listing[i][7:], listing[i + 1]) for i in range(0, len(listing) - 1, 2)
                if listing[i].startswith("commit ")]
        for sha, subject in shas:
            parent, out = commit_output(repo, sha)
            if not out:
                continue
            for f in out["files"]:
                if f.get("unsupported") or f.get("degraded"):
                    continue
                for h in f["hunks"]:
                    diff = hunk_diff(repo, parent, sha, f["path"], h["new_range"])
                    if diff is None:
                        unlocated += 1
                        continue
                    pool.append({
                        "key": f"{name}:{sha[:8]}:{f['path']}:{h['new_range'][0]}",
                        "repo": name, "lang": lang, "sha": sha, "subject": subject,
                        "path": f["path"], "template": template(h["rationale"]),
                        "rationale": h["rationale"], "details": h.get("details", []),
                        "noise": h.get("noise", False), "comment": h.get("comment", False),
                        "findings": [{"name": x["name"], "message": x["message"], "level": x["level"]}
                                     for x in h.get("findings", [])],
                        "diff": diff,
                    })
        print(f"  {name:12} pool={len(pool)}")

    rng = random.Random(a.seed)
    rng.shuffle(pool)
    picked, seen = [], set()

    def take(pred, n):
        for h in pool:
            if len(picked) >= a.n or n <= 0:
                return
            if h["key"] not in seen and pred(h):
                seen.add(h["key"]); picked.append(h); n -= 1

    # the rare strata first, then round-robin over template x language until full
    take(lambda h: h["findings"], a.n // 8)
    take(lambda h: h["noise"], a.n // 10)
    cells = defaultdict(list)
    for h in pool:
        cells[(h["template"], h["lang"])].append(h)
    keys = sorted(cells)
    while len(picked) < a.n and any(cells[k] for k in keys):
        for k in keys:
            while cells[k] and cells[k][-1]["key"] in seen:
                cells[k].pop()
            if cells[k] and len(picked) < a.n:
                h = cells[k].pop(); seen.add(h["key"]); picked.append(h)

    for h in picked:
        h["labels"] = {"faithful": None, "matches_commit": None,
                       **({"noise_correct": None} if h["noise"] else {}),
                       "findings": {x["name"]: None for x in h["findings"]}}
    picked.sort(key=lambda h: h["key"])
    OUT.write_text("".join(json.dumps(h, ensure_ascii=False) + "\n" for h in picked))
    by = defaultdict(int)
    for h in picked:
        by[h["template"]] += 1
    print(f"wrote {len(picked)} hunks to {OUT.relative_to(ROOT)}: "
          + ", ".join(f"{k}={v}" for k, v in sorted(by.items())))
    if unlocated:
        print(f"skipped {unlocated} hunks whose -U3 diff hunk could not be located")


if __name__ == "__main__":
    main()
