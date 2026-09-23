#!/usr/bin/env python3
"""What ordo says about one corpus hunk, by key (repo:sha:path:line), for
chasing a labeled miss back to the engine. Keys are matched as substrings, so
a path or a sha alone lists every hunk of that commit or file.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/corpus-hunk.py KEY... [--diff] [--json]
"""
import argparse
import json
import os
import sys
from pathlib import Path

from corpuslib import commit_input, engine_output


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("keys", nargs="+")
    ap.add_argument("--diff", action="store_true", help="print the hunk's diff too")
    ap.add_argument("--json", action="store_true", help="the hunk's raw engine record")
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", "~/.cache/ordo-corpus")).expanduser()
    cache = {}
    for key in a.keys:
        parts = key.split(":", 2)
        if len(parts) < 2:
            print(f"{key}: expected repo:sha[:path[:line]]", file=sys.stderr)
            continue
        repo, sha = parts[0], parts[1]
        rest = parts[2] if len(parts) > 2 else ""
        path, _, line = rest.rpartition(":")
        if not line.isdigit():
            path, line = rest, ""
        if (repo, sha) not in cache:
            parent, out = None, None
            d = corpus / repo
            if d.is_dir():
                parent, inp = commit_input(str(d), sha)
                out = engine_output(inp) if parent else None
            cache[(repo, sha)] = (parent, out, inp if parent else None)
        parent, out, inp = cache[(repo, sha)]
        if out is None:
            print(f"{key}: no such commit in {corpus / repo}", file=sys.stderr)
            continue
        # the blobs are only split when a diff is actually printed: a commit
        # can be megabytes, and a plain lookup never reads them
        blobs = {}
        if a.diff:
            # split on "\n" alone: str.splitlines() also breaks on form feed and
            # the unicode separators, and a file that contains one (execa's
            # escape-sequence tests carry seven) then numbers its lines
            # differently from the engine
            blobs = {c["path"]: (c["old"].split("\n"), c["new"].split("\n")) for c in inp["changes"]}
        for f in out["files"]:
            if path and path not in f["path"]:
                continue
            for h in f["hunks"]:
                if line and str(h["new_range"][0]) != line and str(h["old_range"][0]) != line:
                    continue
                print(f"{repo}:{sha[:8]}:{f['path']}:{h['new_range'][0]}  [{h['id']}]  "
                      f"old {h['old_range']} new {h['new_range']}  category={h.get('category')} "
                      f"noise={h.get('noise', False)} enclosing={h.get('enclosing')!r} kind={h.get('enclosing_kind')}")
                print(f"  rationale: {h.get('rationale')}")
                for k in ("details", "notes", "defines", "uses", "imports", "symbols"):
                    if h.get(k):
                        print(f"  {k}: {h[k]}")
                for fd in h.get("findings", []):
                    print(f"  finding {fd['name']} [{fd['level']}]: {fd['message'][:100]}")
                if a.diff:
                    old, new = blobs[f["path"]]
                    o0, o1 = h["old_range"]
                    n0, n1 = h["new_range"]
                    for l in old[o0 - 1:o1] if o0 <= o1 else []:
                        print(f"  - {l}")
                    for l in new[n0 - 1:n1] if n0 <= n1 else []:
                        print(f"  + {l}")
                if a.json:
                    print("  " + json.dumps(h))


if __name__ == "__main__":
    main()
