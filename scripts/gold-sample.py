#!/usr/bin/env python3
"""Draw the gold set: ~200 corpus hunks, stratified so every rationale template,
every language, noise hunks and hunks carrying findings all appear. Each entry
carries ordo's own hunk as a diff, not git's: the rationale describes exactly
those lines and the label must judge exactly those lines.

Writes corpus/gold.jsonl with the labels left null; scripts/gold-label.py fills
them in. Deterministic for a given corpus and seed, so a re-draw after a
manifest bump produces the same picks where the commits still exist.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/gold-sample.py [--n 200] [--per-repo 60]
       ... --n 2000 --seed 2 --exclude corpus/gold.jsonl --out silver.jsonl   # a training draw
"""
import argparse
import json
import os
import random
import sys
from collections import defaultdict
from pathlib import Path

from corpuslib import ROOT, commit_input, engine_output, git, repos, template

OUT = ROOT / "corpus/gold.jsonl"


def commit_output(repo, sha):
    """(blobs by path, engine output) for one commit, or (None, None)."""
    parent, inp = commit_input(repo, sha)
    if parent is None:
        return None, None
    # split on "\n" alone: str.splitlines() also breaks on form feed and the
    # unicode separators, and a file that contains one (execa's escape-sequence
    # tests carry seven) then numbers its lines differently from the engine
    blobs = {c["path"]: (c["old"].split("\n"), c["new"].split("\n")) for c in inp["changes"]}
    return blobs, engine_output(inp)


def hunk_diff(old, new, old_range, new_range, ctx=3):
    """ordo's own hunk as a unified diff, with context from the blobs. The git
    -U3 hunk is wider than what ordo describes (git merges a blank-line removal
    or a lone import line into its neighbour), so a labeler shown git's hunk
    judges text the rationale never claimed to cover."""
    o0, o1 = old_range
    n0, n1 = new_range
    removed = old[o0 - 1:o1] if o0 <= o1 else []
    added = new[n0 - 1:n1] if n0 <= n1 else []
    # context: the new-side neighbourhood, or the old side for a pure deletion
    if n0 <= n1:
        before = new[max(0, n0 - 1 - ctx):n0 - 1]
        after = new[n1:n1 + ctx]
    else:
        before = old[max(0, o0 - 1 - ctx):o0 - 1]
        after = old[o1:o1 + ctx]
    head = f"@@ -{o0},{max(0, o1 - o0 + 1)} +{n0},{max(0, n1 - n0 + 1)} @@"
    body = [" " + l for l in before] + ["-" + l for l in removed] + ["+" + l for l in added] + [" " + l for l in after]
    return "\n".join([head] + body)


def hunk_entry(name, lang, sha, subject, f, h, old, new):
    return {
        "key": f"{name}:{sha[:8]}:{f['path']}:{h['new_range'][0]}",
        "repo": name, "lang": lang, "sha": sha, "subject": subject,
        "path": f["path"], "template": template(h["rationale"]),
        "rationale": h["rationale"], "details": h.get("details", []),
        "noise": h.get("noise", False), "comment": h.get("comment", False),
        "findings": [{"name": x["name"], "message": x["message"], "level": x["level"]}
                     for x in h.get("findings", [])],
        "diff": hunk_diff(old, new, h["old_range"], h["new_range"]),
    }


def draw_pool(corpus, per_repo, taken):
    """Every hunk ordo emits on the newest `per_repo` commits of each corpus
    repo, minus the keys in `taken`."""
    pool = []
    for name, lang, rev in repos():
        repo = corpus / name
        if not repo.is_dir():
            continue
        listing = git(repo, "rev-list", "--max-count", str(per_repo), "--format=%s", rev).splitlines()
        shas = [(listing[i][7:], listing[i + 1]) for i in range(0, len(listing) - 1, 2)
                if listing[i].startswith("commit ")]
        for sha, subject in shas:
            blobs, out = commit_output(repo, sha)
            if not out:
                continue
            for f in out["files"]:
                if f.get("unsupported") or f.get("degraded"):
                    continue
                old, new = blobs[f["path"]]
                pool.extend(hunk_entry(name, lang, sha, subject, f, h, old, new) for h in f["hunks"])
        print(f"  {name:12} pool={len(pool)}")
    return [h for h in pool if h["key"] not in taken]


def draw_keys(corpus, keys):
    """The hunks named by `keys` (repo:sha:path:new_start), one engine run per
    commit. A targeted draw: the stratified pool answers "what does ordo emit",
    this answers "show me exactly these", which is what chasing a bucket of the
    judged sweep back to labels needs."""
    want = defaultdict(set)
    for k in keys:
        repo, sha, rest = k.split(":", 2)
        path, _, line = rest.rpartition(":")
        want[(repo, sha)].add((path, int(line)))
    lang_of = {name: lang for name, lang, _ in repos()}
    picked = []
    for (name, short), members in sorted(want.items()):
        if name not in lang_of:
            print(f"  {name}: not in corpus/manifest.toml", file=sys.stderr)
            continue
        repo = corpus / name
        full = git(repo, "rev-parse", "--verify", "-q", short).strip()
        if not full:
            print(f"  {name}:{short} not in the clone", file=sys.stderr)
            continue
        subject = git(repo, "show", "-s", "--format=%s", full).strip()
        blobs, out = commit_output(repo, full)
        if not out:
            continue
        found = set()
        for f in out["files"]:
            if f.get("unsupported") or f.get("degraded"):
                continue
            old_lines, new_lines = blobs[f["path"]]
            for h in f["hunks"]:
                at = (f["path"], h["new_range"][0])
                if at in members:
                    found.add(at)
                    picked.append(hunk_entry(name, lang_of[name], full, subject, f,
                                             h, old_lines, new_lines))
        for path, line in sorted(members - found):
            print(f"  {name}:{short}:{path}:{line} matches no hunk", file=sys.stderr)
    return picked


def stratify(pool, n, seed):
    """`n` hunks: the rare strata first (findings, noise), then round-robin
    over template x language until full."""
    pool = list(pool)
    rng = random.Random(seed)
    rng.shuffle(pool)
    picked, seen = [], set()

    def take(pred, want):
        for h in pool:
            if len(picked) >= n or want <= 0:
                return
            if h["key"] not in seen and pred(h):
                seen.add(h["key"]); picked.append(h); want -= 1

    take(lambda h: h["findings"], n // 8)
    take(lambda h: h["noise"], n // 10)
    cells = defaultdict(list)
    for h in pool:
        cells[(h["template"], h["lang"])].append(h)
    keys = sorted(cells)
    while len(picked) < n and any(cells[k] for k in keys):
        for k in keys:
            while cells[k] and cells[k][-1]["key"] in seen:
                cells[k].pop()
            if cells[k] and len(picked) < n:
                h = cells[k].pop(); seen.add(h["key"]); picked.append(h)
    return picked


def unlabeled_rows(picked):
    """The draw as gold.jsonl rows: every label null, sorted by key."""
    rows = [{**h, "labels": {"faithful": None, "matches_commit": None,
                             **({"noise_correct": None} if h["noise"] else {}),
                             "findings": {x["name"]: None for x in h["findings"]}}}
            for h in picked]
    return sorted(rows, key=lambda h: h["key"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=200)
    ap.add_argument("--per-repo", type=int, default=60, help="commits swept per repo")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", type=Path, default=OUT)
    ap.add_argument("--exclude", type=Path, action="append", default=[],
                    help="jsonl whose keys must not be drawn again (keeps a training draw off the gold set)")
    ap.add_argument("--keys", type=Path,
                    help="draw exactly these keys (one per line) instead of stratifying a pool")
    a = ap.parse_args()
    taken = {json.loads(l)["key"] for f in a.exclude for l in f.read_text().splitlines() if l.strip()}
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus"))
    if a.keys:
        keys = [l.strip() for l in a.keys.read_text().splitlines() if l.strip()]
        excluded = [k for k in keys if k in taken]
        if excluded:
            print(f"  skipping {len(excluded)} keys already in --exclude", file=sys.stderr)
        keys = [k for k in keys if k not in taken]
        picked = draw_keys(corpus, keys)
        print(f"asked for {len(keys)} keys, found {len(picked)}")
    else:
        picked = stratify(draw_pool(corpus, a.per_repo, taken), a.n, a.seed)
    rows = unlabeled_rows(picked)
    a.out.write_text("".join(json.dumps(h, ensure_ascii=False) + "\n" for h in rows))
    by = defaultdict(int)
    for h in rows:
        by[h["template"]] += 1
    print(f"wrote {len(rows)} hunks to {a.out}: "
          + ", ".join(f"{k}={v}" for k, v in sorted(by.items())))


if __name__ == "__main__":
    main()
