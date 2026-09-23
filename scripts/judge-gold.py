#!/usr/bin/env python3
"""Score a System One judge against the labeled gold set.

Asks the judge the same questions the labels answer (faithful, matches_commit,
noise_correct, one per finding) as nouls, then reports agreement at a 0.5 cut
and a threshold-free ranking score (AUC) per question, so a judge with a yes
bias still shows whether its ordering carries signal.

usage: scripts/judge-gold.py [--base-url JEFF_URL] [--gold corpus/gold.jsonl] [--out judged.jsonl]
"""
import argparse
import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path

from typesafe_sdk import Noul, TypeSafeClient

from judgelib import JEFF_URL, QUESTIONS, noul_instructions, state_text

ROOT = Path(__file__).resolve().parent.parent



def auc(pairs):
    """Rank-based AUC over (score, label) pairs; None when one class is missing."""
    pos = [s for s, l in pairs if l]
    neg = [s for s, l in pairs if not l]
    if not pos or not neg:
        return None
    wins = sum((p > n) + 0.5 * (p == n) for p in pos for n in neg)
    return wins / (len(pos) * len(neg))


def fmt(x):
    return "   n/a" if x is None else f"{x:6.2f}"


def questions_for(r):
    """The nouls a labeled row answers: one per labeled question, one per finding."""
    return {k: Noul(instructions=v) for k, v in noul_instructions(r).items()}


def judge(client, text, qs):
    """One System One call, retried on 429/529 and transport errors with backoff."""
    for attempt in range(5):
        try:
            return client.system_one(text, qs)
        except Exception:
            if attempt == 4:
                raise
            time.sleep(2 ** attempt)


def report(judge, n_req, secs, scored, per_rule):
    print(f"judge={judge}  hunks={n_req}  {secs:.0f}s")
    print(f"{'question':16} {'n':>5} {'agree@.5':>9} {'AUC':>6}  {'mean(yes)':>9} {'mean(no)':>8}")
    for k in ("faithful", "matches_commit", "noise_correct", "findings"):
        p = scored.get(k)
        if not p:
            continue
        agree = sum((s >= 0.5) == l for s, l in p) / len(p)
        yes = [s for s, l in p if l]
        no = [s for s, l in p if not l]
        print(f"{k:16} {len(p):5d} {agree:9.2f} {fmt(auc(p))}  "
              f"{sum(yes) / len(yes) if yes else float('nan'):9.2f} {sum(no) / len(no) if no else float('nan'):8.2f}")
    if per_rule:
        print("per rule (n, agree@.5, AUC):")
        for name, p in sorted(per_rule.items(), key=lambda kv: -len(kv[1])):
            agree = sum((s >= 0.5) == l for s, l in p) / len(p)
            print(f"  {name:20} {len(p):3d} {agree:5.2f} {fmt(auc(p))}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", default=JEFF_URL, help="the jeff server")
    ap.add_argument("--gold", type=Path, default=ROOT / "corpus/gold.jsonl")
    ap.add_argument("--out", type=Path, default=None, help="per-hunk judge scores, jsonl")
    ap.add_argument("--limit", type=int, default=0)
    a = ap.parse_args()

    # jeff accepts any key unless started with JEFF_API_KEYS
    client = TypeSafeClient(api_key=os.environ.get("JEFF_API_KEY", "local"), base_url=a.base_url)
    rows = [json.loads(l) for l in a.gold.read_text().splitlines() if l.strip()]
    if a.limit:
        rows = rows[:a.limit]

    scored = defaultdict(list)     # question -> [(score, label)]
    per_rule = defaultdict(list)   # finding name -> [(score, label)]
    out = a.out.open("w") if a.out else None
    t0 = time.time()
    n_req = 0
    for i, r in enumerate(rows):
        L = r["labels"]
        qs = questions_for(r)
        if not qs:
            continue
        text = state_text(r)
        res = judge(client, text, qs)
        n_req += 1
        got = {k: v.noul for k, v in res.nouls.items()}
        for k in QUESTIONS:
            if k in got:
                scored[k].append((got[k], L[k]))
        for name, lab in L["findings"].items():
            k = f"finding:{name}"
            if k in got and lab is not None:
                scored["findings"].append((got[k], lab))
                per_rule[name].append((got[k], lab))
        if out:
            out.write(json.dumps({"key": r["key"], "scores": got}) + "\n")
        if (i + 1) % 25 == 0:
            print(f"  {i + 1}/{len(rows)} ({time.time() - t0:.0f}s)", file=sys.stderr)
    report(a.base_url, n_req, time.time() - t0, scored, per_rule)


if __name__ == "__main__":
    main()
