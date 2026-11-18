#!/usr/bin/env python3
"""Two attention labels per gold and silver hunk, kept in keyed files of their
own (corpus/attention.gold.jsonl, $ORDO_CORPUS/attention.silver.jsonl) so the
labeled sets themselves are never rewritten here.

`later_fix` is SZZ-style: true when a non-merge commit whose subject matches
FIX_SUBJECT, committed within WINDOW_DAYS after the hunk's commit, deletes or
modifies a line `git blame` traces to one of the hunk's added lines (blame
follows renames). Null when the hunk adds no line, its commit is not on the
clone's remote branch, or the window reaches past what the clone knows (the
time of its last fetch) with no fix found yet. `fixed_by` lists the fixes.

`attention` (1-3) is an Opus rating against ATTENTION_PROMPT, collected from
shuffled chunks that show the hunk as skip-sample.py does (key, repo, lang,
sha, path, subject, diff) and nothing of ordo's output or other labels. One
extra chunk repeats CONTROL hunks to measure the labeller's self-agreement.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/attention-label.py szz
       scripts/attention-label.py chunks DIR [--size 50] [--control 50] [--seed 150]
       scripts/attention-label.py collect DIR --model MODEL
       scripts/attention-label.py report
"""
import argparse
import json
import os
import random
import re
import subprocess
import sys
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from corpuslib import ROOT, git

CORPUS = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
SETS = {"gold": (ROOT / "corpus/gold.jsonl", ROOT / "corpus/attention.gold.jsonl"),
        "silver": (CORPUS / "silver.jsonl", CORPUS / "attention.silver.jsonl")}
ATTENTION_PROMPT = CORPUS / "attention.prompt.txt"
SHOWN = ("key", "repo", "lang", "sha", "path", "subject", "diff")
FIX_SUBJECT = re.compile(r"(?i)\b(fix|bug|regress|revert)")
WINDOW_DAYS = 90
DAY = 86400


def rows_of(path):
    return [json.loads(l) for l in Path(path).read_text().splitlines() if l.strip()] if Path(path).exists() else []


def merge(out, fields):
    """Adds `fields` (key -> {field: value}) to the keyed file `out`, keeping
    whatever else its rows hold."""
    have = {r["key"]: r for r in rows_of(out)}
    for k, f in fields.items():
        have.setdefault(k, {"key": k}).update(f)
    out.write_text("".join(json.dumps(have[k], ensure_ascii=False) + "\n" for k in sorted(have)))


def added_range(diff):
    _, _, n0, nc = map(int, re.match(r"@@ -(\d+),(\d+) \+(\d+),(\d+) @@", diff).groups())
    return range(n0, n0 + nc)


def deleted_ranges(diff):
    """old path -> [(first, last)] of the lines a `git diff -U0` removes."""
    out, path = defaultdict(list), None
    for l in diff.splitlines():
        if l.startswith("--- "):
            path = l[6:] if l.startswith("--- a/") else None
        elif l.startswith("@@") and path:
            o0, oc = re.match(r"@@ -(\d+)(?:,(\d+))?", l).groups()
            oc = 1 if oc is None else int(oc)
            if oc:
                out[path].append((int(o0), int(o0) + oc - 1))
    return out


def blamed(repo, fix, paths, since):
    """(origin sha, origin path, origin line) of every line `fix` removes from
    a file whose name is in `paths`."""
    diff = git(repo, "diff", "-M", "-U0", "--no-color", "--no-ext-diff", f"{fix}^", fix)
    found = set()
    for path, ranges in deleted_ranges(diff).items():
        if path not in paths:
            continue
        args = [a for lo, hi in ranges for a in ("-L", f"{lo},{hi}")]
        out = git(repo, "blame", "--porcelain", f"--since={since}", *args, f"{fix}^", "--", path)
        cur = None
        for l in out.splitlines():
            if m := re.match(r"^([0-9a-f]{40}) (\d+) \d+", l):
                cur = (m.group(1), int(m.group(2)))
            elif l.startswith("filename ") and cur:
                found.add((cur[0], l[9:], cur[1]))
    return found


def renamed_names(repo, tip, since, paths):
    """`paths` plus every later name a rename gives them up to `tip`."""
    log = git(repo, "log", "--no-merges", "-M", "--diff-filter=R", "--name-status", "--format=",
              f"--since={since}", "--reverse", tip)
    names = set(paths)
    for l in log.splitlines():
        parts = l.split("\t")
        if len(parts) == 3 and parts[1] in names:
            names.add(parts[2])
    return names


def szz_repo(name, hunks):
    repo = str(CORPUS / name)
    tip = git(repo, "symbolic-ref", "refs/remotes/origin/HEAD").strip()
    assert tip, f"{name}: no origin/HEAD to look forward from"
    fetched = Path(repo) / git(repo, "rev-parse", "--git-path", "FETCH_HEAD").strip()
    horizon = int(fetched.stat().st_mtime)
    shas = sorted({h["sha"] for h in hunks})
    at = dict(zip(shas, map(int, git(repo, "show", "-s", "--format=%ct", *shas).split())))
    on_tip = {s for s in shas if ancestor(repo, s, tip)}
    lo, hi = min(at.values()), max(at.values()) + WINDOW_DAYS * DAY
    since = f"@{lo - DAY}"
    fixes = []
    for l in git(repo, "log", "--no-merges", "--format=%H%x09%ct%x09%s", f"--since={since}", tip).splitlines():
        h, t, subject = l.split("\t", 2)
        if lo < int(t) <= hi and FIX_SUBJECT.search(subject):
            fixes.append((h, int(t)))
    paths = renamed_names(repo, tip, since, {h["path"] for h in hunks})
    hits = {f: blamed(repo, f, paths, since) for f, _ in fixes}
    out = {}
    for h in hunks:
        t, lines = at[h["sha"]], added_range(h["diff"])
        by = [f for f, ft in fixes if t < ft <= t + WINDOW_DAYS * DAY and
              any(sha == h["sha"] and path == h["path"] and n in lines for sha, path, n in hits[f])]
        if not lines or h["sha"] not in on_tip:
            v = None
        elif by:
            v = True
        else:
            v = None if t + WINDOW_DAYS * DAY > horizon else False
        out[h["key"]] = {"later_fix": v, "fixed_by": by}
    print(f"  {name}: {len(hunks)} hunks, {len(fixes)} fix commits, "
          f"{Counter(map(str, (o['later_fix'] for o in out.values())))}", file=sys.stderr)
    return out


def ancestor(repo, sha, tip):
    return subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", sha, tip]).returncode == 0


def szz(a):
    for which, (src, out) in SETS.items():
        by = defaultdict(list)
        for r in rows_of(src):
            by[r["repo"]].append(r)
        with ThreadPoolExecutor(a.jobs) as ex:
            parts = list(ex.map(lambda kv: szz_repo(*kv), sorted(by.items())))
        merge(out, {k: v for p in parts for k, v in p.items()})
        print(f"{which}: later_fix for {sum(map(len, parts))} hunks into {out}")


def chunks(a):
    """Gold and silver shuffled together, so a chunk is not fifty hunks of one
    commit, plus a last chunk repeating `--control` of them for a separate
    labeller."""
    rows = [{f: r[f] for f in SHOWN} for src, _ in SETS.values() for r in rows_of(src)]
    rng = random.Random(a.seed)
    rng.shuffle(rows)
    a.dir.mkdir(parents=True, exist_ok=True)
    n = 0
    for n, i in enumerate(range(0, len(rows), a.size)):
        (a.dir / f"c{n:03d}.jsonl").write_text(
            "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows[i:i + a.size]))
    control = rng.sample(rows, a.control)
    rng.shuffle(control)
    (a.dir / f"c{n + 1:03d}.control.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in control))
    print(f"{n + 1} chunks of {a.size} and 1 control chunk of {a.control} in {a.dir}")


def collect(a):
    got, control, bad = {}, {}, []
    for f in sorted(a.dir.glob("*.labels.jsonl")):
        src = f.with_name(f.name.replace(".labels", ""))
        want = [r["key"] for r in rows_of(src)]
        lines = rows_of(f)
        keys = [r.get("key") for r in lines]
        if keys != want:
            bad.append(f"{f.name}: keys differ from {src.name}")
            continue
        for r in lines:
            v = r.get("attention")
            if v not in (1, 2, 3) or not r.get("why"):
                bad.append(f"{f.name}: {json.dumps(r)[:120]}")
                continue
            (control if ".control" in f.name else got)[r["key"]] = (v, r["why"])
    everything = {r["key"] for src, _ in SETS.values() for r in rows_of(src)}
    missing = everything - set(got)
    for b in bad:
        print(f"bad {b}", file=sys.stderr)
    if bad or missing:
        sys.exit(f"{len(bad)} bad, {len(missing)} keys missing; nothing written")
    for which, (src, out) in SETS.items():
        merge(out, {r["key"]: {"attention": got[r["key"]][0], "attention_why": got[r["key"]][1],
                               "judge": "opus", "model": a.model} for r in rows_of(src)})
    pairs = [(got[k][0], v) for k, (v, _) in control.items()]
    same = sum(x == y for x, y in pairs)
    print(f"attention for {len(got)} hunks; control self-agreement {same}/{len(pairs)}, "
          f"within one {sum(abs(x - y) <= 1 for x, y in pairs)}/{len(pairs)}, "
          f"weighted kappa {kappa(pairs):.2f}")


def kappa(pairs, k=(1, 2, 3)):
    """Quadratic-weighted Cohen's kappa of two raters' ordinal labels."""
    n = len(pairs)
    if not n:
        return float("nan")
    w = lambda i, j: (i - j) ** 2 / (len(k) - 1) ** 2
    pa, pb = Counter(x for x, _ in pairs), Counter(y for _, y in pairs)
    obs = sum(w(x, y) for x, y in pairs) / n
    exp = sum(w(i, j) * pa[i] * pb[j] for i in k for j in k) / n / n
    return 1 - obs / exp if exp else float("nan")


def rate(xs):
    known = [x for x in xs if x is not None]
    return f"{sum(known)}/{len(known)}" + (f" = {sum(known) / len(known):.1%}" if known else "")


def report(a):
    for which, (src, out) in SETS.items():
        rows = {r["key"]: r for r in rows_of(src)}
        lab = {r["key"]: r for r in rows_of(out)}
        print(f"== {which}: {len(rows)} hunks, {len(lab)} labelled")
        lf = [lab[k].get("later_fix") for k in rows if k in lab]
        print(f"later_fix true {rate(lf)}, unknown {lf.count(None)}/{len(lf)}")
        per = defaultdict(list)
        for k, r in rows.items():
            per[r["repo"]].append(lab.get(k, {}))
        print(f"{'repo':14} {'n':>4} {'att 1/2/3':>12} {'later_fix':>16} {'unknown':>7}")
        for repo, ls in sorted(per.items()):
            c = Counter(x.get("attention") for x in ls)
            lf = [x.get("later_fix") for x in ls]
            print(f"{repo:14} {len(ls):4} {c[1]:4}/{c[2]:3}/{c[3]:3} {rate(lf):>16} {lf.count(None):7}")
        print("later_fix by attention:")
        for v in (1, 2, 3):
            print(f"  {v}: {rate([lab[k].get('later_fix') for k in rows if lab.get(k, {}).get('attention') == v])}")
        print("attention by should_skip:")
        for s in (True, False):
            c = Counter(lab.get(k, {}).get("attention") for k, r in rows.items() if r["labels"].get("should_skip") is s)
            print(f"  should_skip={s}: 1/2/3 = {c[1]}/{c[2]}/{c[3]}")
        print("later_fix by should_skip:")
        for s in (True, False):
            print(f"  should_skip={s}: {rate([lab.get(k, {}).get('later_fix') for k, r in rows.items() if r['labels'].get('should_skip') is s])}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("szz")
    s.add_argument("--jobs", type=int, default=6)
    c = sub.add_parser("chunks")
    c.add_argument("dir", type=Path)
    c.add_argument("--size", type=int, default=50)
    c.add_argument("--control", type=int, default=50)
    c.add_argument("--seed", type=int, default=150)
    k = sub.add_parser("collect")
    k.add_argument("dir", type=Path)
    k.add_argument("--model", required=True)
    sub.add_parser("report")
    a = ap.parse_args()
    {"szz": szz, "chunks": chunks, "collect": collect, "report": report}[a.cmd](a)


if __name__ == "__main__":
    main()
