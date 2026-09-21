#!/usr/bin/env python3
"""Grade ordo's claims per hunk across the pinned corpus with a System One judge.

Walks corpus/manifest.toml the way tests/corpus.rs does, runs ordo-engine on
each commit, and asks the judge three nouls per hunk: is the rationale faithful
to the diff, is it consistent with the commit subject, and, when ordo flagged
noise, is the hunk really noise. Findings get one noul each. Scores are written
per hunk to --out (jsonl) so a sweep can resume, and summarised into --report
bucketed by rationale template, language, repo, enclosing_kind and rule.

The judge is paid per input token (output is free), so the sweep is budgeted
on input: --max-tokens stops the walk once usage crosses it (checked after every request, so it overshoots by at
most one hunk). Commits are taken newest-first per repo, round-robin across
repos, so a partial sweep still covers every language.

usage: TYPESAFE_API_KEY=... scripts/corpus-judge.py --max-tokens 3000000 [--per-repo 60] [--base-url URL]
       scripts/corpus-judge.py --report-only   # re-summarise an existing --out
"""
import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

try:
    from typesafe_sdk import Noul, TypeSafeClient
except ImportError:  # --report-only needs no judge
    Noul = TypeSafeClient = None

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / "target/release/ordo-engine"
QUESTIONS = {
    "faithful": "Does the rationale accurately describe what this diff changes?",
    "matches_commit": "Is the rationale consistent with the commit message?",
    "noise_correct": "Is this diff only formatting, whitespace, generated code or import lines, with no behavioural change?",
}
# Cuts that maximise balanced accuracy against the Opus labels on
# corpus/gold.jsonl with jev-1.13.0 (scripts/judge-gold.py scores): faithful
# 0.67 @0.58, matches_commit 0.98 @0.24, noise_correct 0.78 @0.15, finding
# 0.76 @0.50. Balanced rather than raw accuracy, since raw is won by the base
# rate (faithful 0.74 @0.39 is mostly "say yes"). Re-derive on a model bump.
CUTS = {"faithful": 0.58, "matches_commit": 0.24, "noise_correct": 0.15, "finding": 0.5}


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
    names = [l.split("\t", 2)[2] for l in git(repo, "diff", "--numstat", "--no-renames", parent, sha).splitlines()
             if not l.startswith("-\t-\t")]
    if not names:
        return None, None
    inp = {"changes": [{"path": p, "old": git(repo, "show", f"{parent}:{p}"),
                        "new": git(repo, "show", f"{sha}:{p}")} for p in names]}
    r = subprocess.run([ENGINE, "order", "--json"], input=json.dumps(inp), capture_output=True, text=True)
    return parent, json.loads(r.stdout)


def hunk_diff(repo, parent, sha, path, new_range):
    text = git(repo, "diff", "-U3", parent, sha, "--", path)
    for m in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@.*$", text, re.M):
        start, count = int(m.group(1)), int(m.group(2) or 1)
        if start <= new_range[0] <= start + max(count, 1):
            end = text.find("\n@@ ", m.end())
            return text[m.start():end if end > 0 else None]
    return None


def template(rationale):
    return rationale.split(" ", 1)[0] if rationale else "change"


def walk(corpus, per_repo):
    """(repo name, lang, sha, subject) round-robin across repos, newest first."""
    lists = []
    for name, lang, rev in repos():
        repo = corpus / name
        if not repo.is_dir():
            continue
        listing = git(repo, "rev-list", "--max-count", str(per_repo), "--format=%s", rev).splitlines()
        shas = [(name, lang, listing[i][7:], listing[i + 1]) for i in range(0, len(listing) - 1, 2)
                if listing[i].startswith("commit ")]
        lists.append(shas)
    for i in range(per_repo):
        for shas in lists:
            if i < len(shas):
                yield shas[i]


def summarise(out_path, report_path):
    rows = [json.loads(l) for l in out_path.read_text().splitlines() if l.strip()]
    buckets = {k: defaultdict(lambda: defaultdict(list)) for k in ("template", "lang", "repo", "enclosing_kind")}
    rules = defaultdict(list)
    overall = defaultdict(list)
    for r in rows:
        s = r["scores"]
        for q in QUESTIONS:
            if q in s:
                v = s[q] >= CUTS[q]
                overall[q].append(v)
                for k, d in buckets.items():
                    d[r.get(k) or "-"][q].append(v)
        for name, p in r.get("findings", {}).items():
            rules[name].append(p >= CUTS["finding"])
            overall["finding"].append(p >= CUTS["finding"])
    pct = lambda v: {"n": len(v), "rate": round(100 * sum(v) / len(v)) if v else None}
    report = {
        "hunks": len(rows),
        "cuts": CUTS,
        "overall": {q: pct(v) for q, v in overall.items()},
        "by": {k: {b: {q: pct(v) for q, v in qs.items()} for b, qs in d.items()} for k, d in buckets.items()},
        "rules": {n: pct(v) for n, v in sorted(rules.items(), key=lambda kv: -len(kv[1]))},
    }
    report_path.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n")
    print(f"{len(rows)} hunks -> {report_path}")
    print("overall:", {q: f"{v['rate']}% (n={v['n']})" for q, v in report["overall"].items()})
    print("faithful by template, worst first:")
    for b, qs in sorted(report["by"]["template"].items(), key=lambda kv: kv[1].get("faithful", {}).get("rate") or 0):
        f = qs.get("faithful")
        if f and f["n"] >= 10:
            print(f"  {b:12} {f['rate']:3d}% (n={f['n']})")


def hunk_questions(h):
    """The nouls one hunk is asked: faithful, matches_commit, noise when
    flagged, one per finding."""
    qs = {"faithful": Noul(instructions=QUESTIONS["faithful"]),
          "matches_commit": Noul(instructions=QUESTIONS["matches_commit"])}
    if h.get("noise"):
        qs["noise_correct"] = Noul(instructions=QUESTIONS["noise_correct"])
    for x in h.get("findings", []):
        qs[f"finding:{x['name']}"] = Noul(
            instructions=f"Is this reviewer warning warranted for this code? Warning: {x['message']}")
    return qs


class Sweep:
    """A budgeted walk of the corpus: produces (key, meta, text, questions)
    items, asks the judge from a pool of `workers` threads, appends scores to
    `out`, and stops the walk once the input-token budget is spent."""

    def __init__(self, client, corpus, out_path, per_repo, max_tokens, workers):
        self.client, self.corpus, self.per_repo = client, corpus, per_repo
        self.max_tokens, self.workers = max_tokens, workers
        self.done, spent = set(), 0
        if out_path.exists():
            for l in out_path.read_text().splitlines():
                if l.strip():
                    r = json.loads(l)
                    self.done.add(r["key"])
                    spent += r.get("tokens", 0)
        self.out = out_path.open("a")
        self.lock = threading.Lock()
        # the budget is the whole sweep's, so a resumed run starts from what
        # the earlier runs already spent
        self.spent, self.used, self.n, self.stop = spent, spent, 0, False
        self.t0 = time.time()

    def ask(self, item):
        key, meta, text, qs = item
        if self.stop:
            return
        for attempt in range(5):
            try:
                res = self.client.system_one(text, qs)
                break
            except Exception:
                if attempt == 4:
                    raise
                time.sleep(2 ** attempt)
        got = {k: v.noul for k, v in res.nouls.items()}
        tokens = res.usage.input_tokens  # the billed side; output is free
        self.record(key, meta, got, tokens)

    def record(self, key, meta, got, tokens):
        """One hunk's scores to `out`, under the lock the pool's threads share."""
        with self.lock:
            self._record(key, meta, got, tokens)

    def _record(self, key, meta, got, tokens):
        self.used += tokens
        self.n += 1
        self.out.write(json.dumps({
            **meta, "key": key,
            "scores": {k: got[k] for k in QUESTIONS if k in got},
            "findings": {k.split(":", 1)[1]: v for k, v in got.items() if k.startswith("finding:")},
            "tokens": tokens,
        }) + "\n")
        self.out.flush()
        if self.n % 200 == 0:
            print(f"  {self.n} hunks, {self.used} input tokens, {time.time() - self.t0:.0f}s", file=sys.stderr)
        if self.max_tokens and self.used >= self.max_tokens and not self.stop:
            print(f"token budget reached: {self.used} >= {self.max_tokens}", file=sys.stderr)
            self.stop = True

    def items(self):
        for name, lang, sha, subject in walk(self.corpus, self.per_repo):
            if self.stop:
                return
            repo = self.corpus / name
            parent, o = commit_output(repo, sha)
            if not o:
                continue
            for f in o["files"]:
                if f.get("unsupported") or f.get("degraded"):
                    continue
                for h in f["hunks"]:
                    key = f"{name}:{sha[:8]}:{f['path']}:{h['new_range'][0]}"
                    if key in self.done:
                        continue
                    diff = hunk_diff(repo, parent, sha, f["path"], h["new_range"])
                    if diff is None:
                        continue
                    text = f"commit message: {subject}\nrationale: {h['rationale']}\ndiff:\n{diff[:6000]}"
                    meta = {"repo": name, "lang": lang, "path": f["path"],
                            "template": template(h["rationale"]), "enclosing_kind": h.get("enclosing_kind"),
                            "rationale": h["rationale"], "noise": h.get("noise", False)}
                    yield key, meta, text, hunk_questions(h)

    def run(self):
        workers = self.workers
        # bounded submission so the producer (git + ordo, cheap) never runs far
        # ahead of the budget check
        with ThreadPoolExecutor(max_workers=workers) as pool:
            pending = []
            for item in self.items():
                pending.append(pool.submit(self.ask, item))
                if len(pending) >= workers * 4:
                    for fut in pending[:workers * 2]:
                        fut.result()
                    pending = pending[workers * 2:]
            for fut in pending:
                fut.result()
        self.out.close()
        print(f"judged {self.n} new hunks, {self.used - self.spent} tokens this run, "
              f"{self.used} in total, {time.time() - self.t0:.0f}s")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--max-tokens", type=int, default=0, help="stop once usage crosses this")
    ap.add_argument("--per-repo", type=int, default=60)
    ap.add_argument("--base-url", default=None)
    ap.add_argument("--out", type=Path, default=Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")) / "judged.jsonl")
    ap.add_argument("--report", type=Path, default=ROOT / "corpus/judged.json")
    ap.add_argument("--report-only", action="store_true")
    ap.add_argument("--workers", type=int, default=6, help="jev allows 1200 rpm; 6 workers at ~0.4s each is ~850")
    a = ap.parse_args()
    if a.report_only:
        return summarise(a.out, a.report)

    client = TypeSafeClient(api_key=os.environ["TYPESAFE_API_KEY"], base_url=a.base_url) \
        if a.base_url else TypeSafeClient(api_key=os.environ["TYPESAFE_API_KEY"])
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus"))
    Sweep(client, corpus, a.out, a.per_repo, a.max_tokens, a.workers).run()
    summarise(a.out, a.report)


if __name__ == "__main__":
    main()
