#!/usr/bin/env python3
"""Per-hunk attention labels from real code reviews. For each PR or Gerrit change
with human comments, every commit a human commented on (its snapshot) runs through
the engine with features. Each human comment maps to the hunk holding its line, and
each hunk gets its thread count and whether a later commit of the same PR or change
touched its new lines.

Inputs: $ORDO_CORPUS/reviews.sqlite (pr-mine.py), reviews-gerrit.sqlite (gerrit-mine.py).
Clones go to $ORDO_CORPUS/reviews-repos. Output: attention.review.jsonl (one row per
hunk with features), attention.review.progress.jsonl (resume state, one row per
snapshot), attention.review.report.json.

Usage: tspn 8 scripts/review-label.py
"""
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from corpuslib import engine_output, git, range_input, with_features

CORPUS = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
CLONES = CORPUS / "reviews-repos"
OUT = CORPUS / "attention.review.jsonl"
PROGRESS = CORPUS / "attention.review.progress.jsonl"
REPORT = CORPUS / "attention.review.report.json"
MAX_HUNKS = 200
UNIT_JOBS, REPO_JOBS, BATCH = 8, 4, 50
TRI = {True: "true", False: "false", None: "null"}


def quiet(args):
    return subprocess.run(args, capture_output=True, text=True)


def present(d, shas):
    if not shas:
        return set()
    out = subprocess.run(["git", "-C", str(d), "cat-file", "--batch-check=%(objectname) %(objecttype)"],
                         input="\n".join(sorted(shas)) + "\n", capture_output=True, text=True).stdout
    return {l.split()[0] for l in out.splitlines() if l.endswith(" commit")}


def root(reply, key):
    seen = set()
    while (up := reply.get(key)) is not None and key not in seen:
        seen.add(key)
        key = (*key[:2], up)
    return key[2]


def github(extra):
    db = sqlite3.connect(CORPUS / "reviews.sqlite")
    prs = {(r, n): (b, h) for r, n, b, h in db.execute("select repo, number, base_sha, head_sha from pr where kept = 1")}
    rows = db.execute("select c.repo, c.number, c.id, c.in_reply_to, c.bot, c.path, c.side, c.original_line, "
                      "c.original_commit_id from comment c join pr p on p.repo = c.repo and p.number = c.number "
                      "where p.kept = 1").fetchall()
    reply = {(r, n, i): p for r, n, i, p, *_ in rows}
    groups = defaultdict(list)
    for r, n, i, parent, bot, path, side, line, sha in rows:
        if bot:
            continue
        if sha is None:
            extra[r]["human"] += 1
            extra[r]["unmapped"]["no_snapshot"] += 1
            continue
        groups[(r, n, sha)].append((path, side == "RIGHT", line, root(reply, (r, n, i))))
    units = []
    for (r, n, sha), cs in sorted(groups.items()):
        base, head = prs[(r, n)]
        units.append({"source": "github", "label": r, "pr": n, "snap": sha, "dir": CLONES / r.replace("/", "__"),
                      "url": f"https://github.com/{r}.git", "base": base, "merge": True, "final": head,
                      "refs": {f"refs/review/pr/{n}": f"refs/pull/{n}/head"}, "comments": cs})
    return units


def gerrit(extra):
    db = sqlite3.connect(CORPUS / "reviews-gerrit.sqlite")
    proj = {(h, n): p for h, n, p in db.execute("select host, number, project from change where kept = 1")}
    revs = {(h, n, ps): (sha, parents, ref)
            for h, n, ps, sha, parents, ref in db.execute("select host, number, patch_set, sha, parents, ref from revision")
            if (h, n) in proj}
    latest = {}
    for h, n, ps in revs:
        latest[(h, n)] = max(latest.get((h, n), ps), ps)
    rows = db.execute("select c.host, c.number, c.id, c.in_reply_to, c.bot, c.path, c.side, c.patch_set, c.line "
                      "from comment c join change ch on ch.host = c.host and ch.number = c.number "
                      "where ch.kept = 1").fetchall()
    reply = {(h, n, i): p for h, n, i, p, *_ in rows}
    groups = defaultdict(list)
    for h, n, i, parent, bot, path, side, ps, line in rows:
        if bot:
            continue
        label = f"{h}/{proj[(h, n)]}"
        if (h, n, ps) not in revs:
            extra[label]["human"] += 1
            extra[label]["unmapped"]["no_snapshot"] += 1
            continue
        groups[(h, n, ps)].append((path, side != "PARENT", line, root(reply, (h, n, i))))
    units = []
    for (h, n, ps), cs in sorted(groups.items()):
        sha, parents, ref = revs[(h, n, ps)]
        last = latest[(h, n)]
        p = proj[(h, n)]
        units.append({"source": "gerrit", "label": f"{h}/{p}", "pr": n, "snap": sha,
                      "dir": CLONES / f"{h}__{p.replace('/', '__')}", "url": f"https://{h}/{p}",
                      "base": (parents.split() or [None])[0], "merge": False, "final": revs[(h, n, last)][0],
                      "refs": {f"refs/review/cl/{n}/{ps}": ref, f"refs/review/cl/{n}/{last}": revs[(h, n, last)][2]},
                      "comments": cs})
    return units


def fetch_refs(d, pairs):
    args = [f"+{src}:{dst}" for dst, src in pairs]
    if quiet(["git", "-C", str(d), "fetch", "-q", "--filter=blob:none", "origin", *args]).returncode == 0:
        return
    for a in args:
        quiet(["git", "-C", str(d), "fetch", "-q", "--filter=blob:none", "origin", a])


def fetch(d, spec):
    """Clones `d` if missing, fetches the review refs it lacks and the commits it
    needs by sha. False when the clone fails."""
    if not d.exists():
        part = d.with_name(d.name + ".part")
        shutil.rmtree(part, ignore_errors=True)
        if quiet(["git", "clone", "-q", "--filter=blob:none", "--no-checkout", spec["url"], str(part)]).returncode:
            return False
        part.rename(d)
    have = set(git(d, "for-each-ref", "--format=%(refname)").split())
    want = [(dst, spec["refs"][dst]) for dst in sorted(spec["refs"]) if dst not in have]
    for i in range(0, len(want), BATCH):
        fetch_refs(d, want[i:i + BATCH])
    missing = sorted(spec["shas"] - present(d, spec["shas"]))
    for i in range(0, len(missing), BATCH):
        quiet(["git", "-C", str(d), "fetch", "-q", "--filter=blob:none", "origin", *missing[i:i + BATCH]])
    return True


def changed_lines(diff):
    """path -> [(old start, old count)] of what a `git diff -U0` removes or inserts after."""
    out, path, header = defaultdict(list), None, False
    for l in diff.splitlines():
        if l.startswith("diff --git "):
            header, path = True, None
        elif header and l.startswith("--- "):
            path = l[6:] if l.startswith("--- a/") else None
        elif l.startswith("+++ "):
            header = False
        elif l.startswith("@@") and path:
            m = re.match(r"@@ -(\d+)(?:,(\d+))? ", l)
            out[path].append((int(m[1]), 1 if m[2] is None else int(m[2])))
    return out


def touched(segs, lines):
    """Whether `segs` removes or replaces any of `lines`; pure insertions next
    to them do not count."""
    return any(o <= l <= o + n - 1 for o, n in segs if n for l in lines)


def find(hunks, path, new, line):
    span = "new_range" if new else "old_range"
    for i, (p, h) in enumerate(hunks):
        a, b = h[span]
        if p == path and a <= line <= b:
            return i
    return None


def uid(u):
    return f"{u['source']}:{u['label']}:{u['pr']}:{u['snap']}"


def run_unit(u):
    d, snap, cs = str(u["dir"]), u["snap"], u["comments"]
    stat = {"unit": uid(u), "source": u["source"], "repo": u["label"], "pr": u["pr"], "snapshot": snap,
            "status": "ok", "human": len(cs), "hunks": 0, "labeled": 0, "commented": 0,
            "unmapped": Counter(), "later_fix": Counter()}

    def fail(status):
        stat["status"] = status
        stat["unmapped"][status] += len(cs)
        return [], stat

    if not u["in_clone"]:
        return fail("no_clone")
    if git(d, "cat-file", "-t", snap).strip() != "commit":
        return fail("no_commit")
    base = u["base"]
    if u["merge"] and base:
        base = git(d, "merge-base", base, snap).strip()
    if not base or git(d, "cat-file", "-t", base).strip() != "commit":
        return fail("no_base")
    inp = range_input(d, base, snap)
    if inp is None:
        return fail("no_text")
    try:
        out = engine_output(with_features(d, snap, inp))
    except json.JSONDecodeError:
        return fail("engine_error")
    hunks = [(f["path"], h) for f in out["files"] for h in f["hunks"]]
    file_of = [fi for fi, f in enumerate(out["files"]) for _ in f["hunks"]]
    read = {(o["path"], o["hunk"]): i for i, o in enumerate(out["order"])}
    stat["hunks"] = len(hunks)
    if len(hunks) > MAX_HUNKS:
        return fail("skipped_large")

    keys, seen = [], set()
    for i, (p, h) in enumerate(hunks):
        k = f"{u['label']}:{snap[:8]}:{p}:{h['new_range'][0]}"
        keys.append(k if k not in seen else f"{k}~{i}")
        seen.add(keys[-1])

    roots, lines, unmapped = defaultdict(set), defaultdict(set), stat["unmapped"]
    for path, new, line, thread in cs:
        if line is None:
            unmapped["no_line"] += 1
            continue
        i = find(hunks, path, new, line)
        if i is None:
            unmapped["no_hunk"] += 1
        elif hunks[i][1].get("features") is None:
            unmapped["no_features"] += 1
        else:
            roots[i].add(thread)
            if new:
                lines[i].add(line)

    final = u["final"]
    final_ok = bool(final) and git(d, "cat-file", "-t", final).strip() == "commit"
    changed = {}
    if final_ok and final != snap and roots:
        changed = changed_lines(git(d, "-c", "core.quotePath=false", "diff", "-U0", "--no-renames", "--no-color",
                                    "--no-ext-diff", snap, final))

    def later(i):
        """Whether a later revision rewrote a commented line; None when no
        comment sits on the new side or the final revision is unknown."""
        if not final_ok or not lines[i]:
            return None
        return touched(changed.get(hunks[i][0], []), lines[i])

    rows = []
    for i, (p, h) in enumerate(hunks):
        if h.get("features") is None:
            continue
        t = len(roots.get(i, ()))
        rows.append({"key": keys[i], "source": u["source"], "repo": u["label"], "pr": u["pr"], "snapshot": snap,
                     "threads": t, "attention": t > 0, "later_fix": later(i) if t else None,
                     "new_range": h["new_range"], "comment_lines": sorted(lines.get(i, ())),
                     "file": file_of[i], "read": read[p, h["id"]],
                     "features": h["features"]})
    stat["labeled"] = len(rows)
    stat["commented"] = sum(r["threads"] > 0 for r in rows)
    stat["later_fix"] = Counter(TRI[r["later_fix"]] for r in rows if r["threads"])
    return rows, stat


def row_unit(r):
    return f"{r['source']}:{r['repo']}:{r['pr']}:{r['snapshot']}"


def resume():
    """Units already in the progress file; output rows of any other unit (a
    run cut off mid-unit) are dropped."""
    done = {json.loads(l)["unit"] for l in PROGRESS.read_text().splitlines() if l.strip()} if PROGRESS.exists() else set()
    if OUT.exists():
        keep = [l for l in OUT.read_text().splitlines() if l.strip() and row_unit(json.loads(l)) in done]
        OUT.write_text("".join(l + "\n" for l in keep))
    return done


def report(extra):
    stats = [json.loads(l) for l in PROGRESS.read_text().splitlines() if l.strip()]
    repos = {}

    def slot(label):
        return repos.setdefault(label, {"snapshots": 0, "statuses": Counter(), "hunks": 0, "labeled": 0,
                                        "commented": 0, "human": 0, "unmapped": Counter(), "later_fix": Counter()})
    for s in stats:
        r = slot(s["repo"])
        r["snapshots"] += 1
        r["statuses"][s["status"]] += 1
        r["human"] += s["human"]
        r["unmapped"].update(s["unmapped"])
        r["later_fix"].update(s["later_fix"])
        if s["status"] == "ok":
            r["hunks"] += s["hunks"]
            r["labeled"] += s["labeled"]
            r["commented"] += s["commented"]
    for label, e in extra.items():
        r = slot(label)
        r["human"] += e["human"]
        r["unmapped"].update(e["unmapped"])
    for r in repos.values():
        r["unmapped_pct"] = round(100 * sum(r["unmapped"].values()) / r["human"], 1) if r["human"] else None
    total = {k: sum(r[k] for r in repos.values()) for k in ("snapshots", "hunks", "labeled", "commented", "human")}
    total["unmapped"] = sum(sum(r["unmapped"].values()) for r in repos.values())
    total["later_fix"] = sum((r["later_fix"] for r in repos.values()), Counter())
    REPORT.write_text(json.dumps({"repos": repos, "total": total}, indent=1, sort_keys=True))
    print(f"{'repo':40} {'snap':>5} {'hunks':>7} {'commented':>9} {'human':>6} {'unmap%':>7} {'large':>5} "
          f"{'lf t/f/n':>16}", file=sys.stderr)
    for label, r in sorted(repos.items()):
        lf = r["later_fix"]
        print(f"{label[:40]:40} {r['snapshots']:5} {r['hunks']:7} {r['commented']:9} {r['human']:6} "
              f"{str(r['unmapped_pct']):>7} {r['statuses']['skipped_large']:5} "
              f"{lf['true']}/{lf['false']}/{lf['null']:>5}", file=sys.stderr)
    print(f"total {total}", file=sys.stderr)


def main():
    CLONES.mkdir(parents=True, exist_ok=True)
    extra = defaultdict(lambda: {"human": 0, "unmapped": Counter()})
    units = github(extra) + gerrit(extra)
    specs = {}
    for u in units:
        s = specs.setdefault(str(u["dir"]), {"url": u["url"], "refs": {}, "shas": set()})
        s["refs"].update(u["refs"])
        s["shas"] |= {u["snap"], u["final"]} | ({u["base"]} if u["base"] else set())
    with ThreadPoolExecutor(REPO_JOBS) as ex:
        ok = {d for d, good in zip(specs, ex.map(lambda kv: fetch(Path(kv[0]), kv[1]), specs.items())) if good}
    for u in units:
        u["in_clone"] = str(u["dir"]) in ok
    done = resume()
    todo = [u for u in units if uid(u) not in done]
    print(f"{len(units)} snapshots, {len(todo)} to run, {len(done)} done", file=sys.stderr)
    with OUT.open("a") as out, PROGRESS.open("a") as prog, ThreadPoolExecutor(UNIT_JOBS) as ex:
        futs = [ex.submit(run_unit, u) for u in todo]
        for n, f in enumerate(as_completed(futs), 1):
            rows, stat = f.result()
            for r in rows:
                out.write(json.dumps(r, ensure_ascii=False) + "\n")
            out.flush()
            prog.write(json.dumps(stat) + "\n")
            prog.flush()
            if n % 100 == 0:
                print(f"  {n}/{len(todo)}", file=sys.stderr)
    report(extra)


if __name__ == "__main__":
    main()
