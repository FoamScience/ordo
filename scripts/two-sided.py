#!/usr/bin/env python3
"""Hunks with dependencies in both directions — the dependency canvas's wide case.

Most hunks have edges one way or none at all (measured: max one edge per hunk on
real branches), so the canvas usually shows a single card. This finds the ones
that fill both halves, which is what you want when testing or demonstrating it.

    scripts/two-sided.py                  # HEAD, against its parent
    scripts/two-sided.py main...HEAD      # a branch
    scripts/two-sided.py 4abfd84e -C ~/repo/nightshift

Prints, for each hit, the `:goto` that selects its file. `gD` opens the canvas.
"""

import collections
import json
import subprocess
import sys

ENGINE = "target/debug/ordo-engine"


def main() -> int:
    argv = sys.argv[1:]
    repo = "."
    if "-C" in argv:
        i = argv.index("-C")
        repo = argv[i + 1]
        del argv[i : i + 2]
    rev = argv[0] if argv else "HEAD~1..HEAD"
    if ".." not in rev:
        rev = f"{rev}~1..{rev}"

    diff = subprocess.run(
        ["git", "-C", repo, "diff", "-U100000", *rev.split("..", 1)],
        capture_output=True,
        text=True,
    )
    if diff.returncode != 0:
        print(diff.stderr.strip(), file=sys.stderr)
        return 1

    out = subprocess.run(
        [ENGINE, "review", "--full-context"],
        input=diff.stdout,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        print(out.stderr.strip() or f"{ENGINE} failed", file=sys.stderr)
        return 1

    d = json.loads(out.stdout)
    where = {
        h["id"]: (f["path"], h["new_range"][0])
        for f in d["files"]
        for h in f["hunks"]
    }
    # an edge runs definition -> use: a hunk that is a `from` is needed by
    # something, one that is a `to` needs something
    needed_by: collections.Counter = collections.Counter()
    needs: collections.Counter = collections.Counter()
    for e in d["edges"]:
        needed_by[e["from"]] += 1
        needs[e["to"]] += 1

    hits = [
        (where[h][0], where[h][1], needs[h], needed_by[h])
        for h in set(needs) | set(needed_by)
        if needs[h] and needed_by[h] and h in where
    ]
    hits.sort(key=lambda r: -min(r[2], r[3]))

    print(f"{rev}: {len(d['edges'])} edges, {len(hits)} two-sided hunk(s)")
    for path, line, n, m in hits:
        print(f"  {path}:{line}  {n} needs, {m} needed by   ->  :goto {path}")
    if not hits:
        print("  (none — every hunk's dependencies run one way)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
