#!/usr/bin/env bash
# Clone/update the corpora listed in corpus/manifest.toml at their pinned revs.
# Never run by the test suite — `tests/corpus.rs` only reads what is already on
# disk and skips when nothing is.
#
# Usage: scripts/corpus-fetch.sh [name ...]     (no args = every repo)
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
manifest="$root/corpus/manifest.toml"
dest="${ORDO_CORPUS:-$HOME/.cache/ordo-corpus}"
want=("$@")

# Depth must exceed every max_commits in the manifest: the sweep needs each
# commit's PARENT blobs, and tests/corpus.rs skips any commit whose parent lies
# beyond the shallow boundary rather than diffing it against the empty tree.
# Each clone is then deepened by date so the oldest swept commit still has the
# hunch history window (91 days, as scripts/corpuslib.py bounds its log) behind it.
depth=600

mkdir -p "$dest"

# Parse the flat [[repo]] blocks — name/url/rev/max_commits only, one field per line.
python3 - "$manifest" "$dest" "$depth" "${want[@]}" <<'PY'
import re, subprocess, sys
from datetime import datetime, timezone
from pathlib import Path

manifest, dest, depth, *want = sys.argv[1:]
blocks = open(manifest).read().split("[[repo]]")[1:]
field = lambda b, k: (m.group(1) if (m := re.search(rf'^{k}\s*=\s*"?([^"\n]+)"?', b, re.M)) else None)
HISTORY_DAYS = 91


def git(path, *args):
    return subprocess.run(["git", "-C", path, *args], capture_output=True, text=True).stdout


def deepen(name, path, rev, commits):
    """Fetches the history window behind the oldest of the `commits` swept from
    `rev`, unless every shallow edge already lies before it."""
    shallow = Path(path) / git(path, "rev-parse", "--git-path", "shallow").strip()
    if not shallow.is_file():
        return
    edges = lambda: [int(t) for t in git(path, "show", "-s", "--format=%ct", *shallow.read_text().split()).split()]
    since = min(map(int, git(path, "log", "--format=%ct", "--max-count", str(commits), rev).split())) \
        - HISTORY_DAYS * 86400
    if all(t <= since for t in edges()):
        return
    cutoff = datetime.fromtimestamp(since - 86400, timezone.utc).strftime("%Y-%m-%d %H:%M:%S +0000")
    print(f"  deepen  {name} since {cutoff[:10]}")
    subprocess.run(["git", "-C", path, "fetch", "--quiet", f"--shallow-since={cutoff}", "origin", rev],
                   check=True)
    if any(t > since for t in edges()):
        print(f"  WARN    {name}: shallow edge still inside the history window")

for b in blocks:
    name, url, rev = field(b, "name"), field(b, "url"), field(b, "rev")
    commits = int(field(b, "max_commits") or depth)
    if not (name and url and rev) or (want and name not in want):
        continue
    path = f"{dest}/{name}"
    try:
        have = subprocess.run(["git", "-C", path, "rev-parse", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
    except Exception:
        have = ""
    if have == rev:
        deepen(name, path, rev, commits)
        print(f"  ok      {name} @ {rev[:8]}")
        continue
    if not have:
        print(f"  clone   {name}")
        subprocess.run(["git", "clone", "--quiet", "--depth", depth,
                        "--single-branch", url, path], check=True)
    else:
        print(f"  fetch   {name} -> {rev[:8]}")
        subprocess.run(["git", "-C", path, "fetch", "--quiet", "--depth", depth,
                        "origin", rev], check=True)
    # A pinned rev may sit behind the shallow boundary of a fresh clone; fetch it
    # explicitly before checking out, and report plainly if it is unreachable.
    if subprocess.run(["git", "-C", path, "cat-file", "-e", f"{rev}^{{commit}}"],
                      capture_output=True).returncode != 0:
        subprocess.run(["git", "-C", path, "fetch", "--quiet", "--depth", depth,
                        "origin", rev], check=False)
    r = subprocess.run(["git", "-C", path, "checkout", "--quiet", rev],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"  FAILED  {name}: cannot check out {rev}\n{r.stderr.strip()}")
    deepen(name, path, rev, commits)
    print(f"  ok      {name} @ {rev[:8]}")
PY

echo
echo "corpora in $dest"
echo "run the corpus tests with:  ORDO_CORPUS=$dest cargo test --test corpus"
