#!/usr/bin/env python3
"""Draw the census repo list: the most-starred GitHub repositories per
language, under a size cap, each pinned to its default branch's head. Writes
corpus/census.toml for scripts/census.py.

A breadth complement to corpus/manifest.toml, which holds twelve repos read in
depth; the corpus repos are left out here. Needs an authenticated `gh`.

Usage: scripts/census-list.py [--per-lang 5] [--max-mb 60]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

from corpuslib import ROOT

OUT = ROOT / "corpus/census.toml"
LANGS = ["python", "rust", "c", "c++", "go", "java", "javascript", "typescript", "lua", "shell"]


def gh(path):
    r = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"gh api {path}: {r.stderr.strip()}")
    return json.loads(r.stdout)


def corpus_urls():
    text = (ROOT / "corpus/manifest.toml").read_text()
    return {l.split('"')[1].removesuffix(".git").lower() for l in text.splitlines() if l.startswith("url")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--per-lang", type=int, default=5)
    ap.add_argument("--max-mb", type=int, default=60)
    ap.add_argument("--out", type=Path, default=OUT)
    a = ap.parse_args()
    skip = corpus_urls()
    rows = []
    for lang in LANGS:
        q = f"language:{lang.replace('+', '%2B')}+stars:>1000+archived:false+size:<{a.max_mb * 1000}"
        items = gh(f"search/repositories?q={q}&sort=stars&per_page=50")["items"]
        picked = 0
        for it in items:
            url = it["html_url"]
            # a list of links is markdown, not the language it is filed under
            if url.lower() in skip or "awesome" in it["name"].lower() or it.get("fork"):
                continue
            sha = gh(f"repos/{it['full_name']}/commits/{it['default_branch']}")["sha"]
            rows.append((it["full_name"].replace("/", "__"), url, lang, sha))
            picked += 1
            if picked == a.per_lang:
                break
        print(f"  {lang:11} {picked}")
    with a.out.open("w") as f:
        f.write("# Census repos, drawn by scripts/census-list.py; read by scripts/census.py.\n")
        for name, url, lang, sha in rows:
            f.write(f'\n[[repo]]\nname = "{name}"\nurl = "{url}"\nlang = "{lang}"\nrev = "{sha}"\n')
    print(f"wrote {len(rows)} to {a.out}")


if __name__ == "__main__":
    main()
