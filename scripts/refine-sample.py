#!/usr/bin/env python3
"""Draw the refine gold set: small hunks with lines on both sides, the hunks
intra-line refinement has work to do on, a few per corpus repo.

Each entry carries ordo's own hunk ranges and its removed/added lines; the
`marked` field is left null for a labeller to fill with the same lines, the
changed text wrapped in ⟦ ⟧ and moved text in ⟪ ⟫. tests/refine_gold.rs
scores the refiner against those marks. Deterministic for a given corpus and
seed.

The draw does not look at the refiner's output: a set picked by what the
refiner already does well would grade it on its own terms. Two thirds of each
repo's picks have a removed and an added line that difflib finds alike (an
edit in place), the rest are any small two-sided hunk.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/refine-sample.py [--per-repo 3] [--commits 40]
       ... --exclude corpus/refine_gold.jsonl --out more.jsonl   # a further draw
"""
import argparse
import difflib
import json
import os
import random
import sys
from pathlib import Path

from corpuslib import ROOT, commit_input, engine_output, git, repos

OUT = ROOT / "corpus/refine_gold.jsonl"
MAX_LINES = 6


def lines(blob):
    # the reviewer reads its sources with str.lines(), which splits on "\n"
    # and drops a trailing empty line; match it so ranges index the same rows
    return blob.split("\n")[:-1] if blob.endswith("\n") else blob.split("\n")


def alike(old, new):
    return any(difflib.SequenceMatcher(None, o.strip(), n.strip()).ratio() >= 0.6
               for o in old for n in new if o.strip() and n.strip())


def pool_for(corpus, name, lang, rev, commits):
    repo = corpus / name
    listing = git(repo, "rev-list", "--max-count", str(commits), rev).split()
    pool = []
    for sha in listing:
        parent, inp = commit_input(repo, sha)
        if parent is None:
            continue
        blobs = {c["path"]: (lines(c["old"]), lines(c["new"])) for c in inp["changes"]}
        for f in engine_output(inp)["files"]:
            if f.get("unsupported") or f.get("degraded"):
                continue
            old, new = blobs[f["path"]]
            for h in f["hunks"]:
                (o0, o1), (n0, n1) = h["old_range"], h["new_range"]
                if not (1 <= o1 - o0 + 1 <= MAX_LINES and 1 <= n1 - n0 + 1 <= MAX_LINES):
                    continue
                rm, add = old[o0 - 1:o1], new[n0 - 1:n1]
                pool.append({
                    "key": f"{name}:{sha[:8]}:{f['path']}:{n0}",
                    "repo": name, "lang": lang, "sha": sha, "path": f["path"],
                    "old_range": [o0, o1], "new_range": [n0, n1],
                    "old": rm, "new": add, "marked": None,
                    "_alike": alike(rm, add),
                })
    return pool


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per-repo", type=int, default=3)
    ap.add_argument("--commits", type=int, default=40)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--exclude", type=Path)
    ap.add_argument("--out", type=Path, default=OUT)
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", "")).expanduser()
    if not corpus.is_dir():
        sys.exit("set ORDO_CORPUS to a directory populated by scripts/corpus-fetch.sh")
    taken = set()
    if a.exclude and a.exclude.exists():
        taken = {json.loads(l)["key"] for l in a.exclude.read_text().splitlines() if l.strip()}
    rng = random.Random(a.seed)
    picked = []
    for name, lang, rev in repos():
        if not (corpus / name).is_dir():
            continue
        pool = [h for h in pool_for(corpus, name, lang, rev, a.commits) if h["key"] not in taken]
        edits = [h for h in pool if h["_alike"]]
        k = min(len(edits), (2 * a.per_repo + 2) // 3)
        chosen = rng.sample(edits, k)
        rest = [h for h in pool if h not in chosen]
        chosen += rng.sample(rest, min(len(rest), a.per_repo - k))
        print(f"  {name:14} pool={len(pool):4} edits={len(edits):4} picked={len(chosen)}")
        picked += chosen
    with a.out.open("w") as f:
        for h in picked:
            h.pop("_alike")
            f.write(json.dumps(h, ensure_ascii=False) + "\n")
    print(f"wrote {len(picked)} to {a.out}")


if __name__ == "__main__":
    main()
