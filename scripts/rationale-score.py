#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scipy", "scikit-learn", "lightgbm"]
# ///
"""Rationale scoreboard: run the current ordo-engine over the hunks in
$ORDO_CORPUS/judged.faithful.jsonl and count what survives. A label survives
while the engine still writes the rationale (and details) the judge saw, and
then it is still right or still a wrong explanation. A label whose rationale
the engine has since changed is dropped, as the trainer drops it, and
reported by its old and new rationale: unfaithful drops are fixes, faithful
drops may be regressions worth a look. No model calls.

Writes corpus/rationale-scoreboard.json and prints the table, with the change
against the report it replaces. Labels are matched to hunks by hunch-train.py's
own `match_labeled`, so the two never disagree on what survives.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/rationale-score.py [--workers N] [--out FILE] [--top N]

Build the engine first (cargo build --release --bin ordo-engine).
"""
import argparse
import importlib.util
import json
import os
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("hunch_train", ROOT / "scripts/hunch-train.py")
ht = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ht)

verb = lambda text: text[0].split()[0] if text[0] else ""


def tally(rows, key):
    """{group: {n, unfaithful, rate}} over (label, group) rows."""
    c = defaultdict(lambda: [0, 0])
    for r in rows:
        g = c[key(r)]
        g[0] += 1
        g[1] += not r["faithful"]
    return {k: {"n": n, "unfaithful": u, "rate": round(u / n, 4)}
            for k, (n, u) in sorted(c.items(), key=lambda kv: (-kv[1][1], -kv[1][0], kv[0]))}


def score(corpus, workers):
    labels = [json.loads(l) for l in (corpus / "judged.faithful.jsonl").read_text().splitlines() if l.strip()]
    why = {r["key"]: r.get("why", "") for r in labels}
    records = list(ht.extra_records(corpus / "judged.faithful.jsonl", corpus / "skip-sample.jsonl", "faithful"))
    keys, _, texts, _, _, by_overlap = ht.match_labeled(corpus, records, workers)
    live, dropped, unmatched = [], [], []
    for r, k in zip(records, keys):
        row = {"key": r["key"], "repo": r["repo"], "faithful": r["labels"]["faithful"], "old": r["text"]}
        if k is None:
            unmatched.append(row)
        elif ht.is_stale(r, texts[k]):
            dropped.append({**row, "new": texts[k], "why": why[r["key"]]})
        else:
            live.append(row)
    for r in live:
        r["verb"] = verb(r["old"])
    for r in dropped:
        r["verb"], r["new_verb"] = verb(r["old"]), verb(r["new"])
    unfaithful = [r for r in dropped if not r["faithful"]]
    faithful = [r for r in dropped if r["faithful"]]
    return {
        "labels": len(records), "unmatched": len(unmatched), "matched_by_overlap": by_overlap,
        "surviving": {"n": len(live), "unfaithful": sum(not r["faithful"] for r in live),
                      "by_verb": tally(live, lambda r: r["verb"]), "by_repo": tally(live, lambda r: r["repo"])},
        "dropped": {"n": len(dropped), "unfaithful": len(unfaithful), "faithful": len(faithful),
                    "by_verb": tally(dropped, lambda r: r["verb"]),
                    "verb_changes": dict(Counter(f"{r['verb']} -> {r['new_verb']}" for r in dropped
                                                  if r["verb"] != r["new_verb"]).most_common()),
                    "faithful_rows": [{k: r[k] for k in ("key", "old", "new", "why")} for r in faithful],
                    "unfaithful_rows": [{k: r[k] for k in ("key", "old", "new", "why")} for r in unfaithful]},
        "unmatched_keys": [r["key"] for r in unmatched],
        "still_unfaithful": [{"key": r["key"], "rationale": r["old"][0], "details": r["old"][1], "why": why[r["key"]]}
                             for r in live if not r["faithful"]],
    }


def table(title, groups, top):
    print(f"\n{title}\n{'':2}{'group':<24}{'n':>6}{'wrong':>7}{'rate':>8}")
    for k, v in list(groups.items())[:top]:
        print(f"  {k:<24}{v['n']:>6}{v['unfaithful']:>7}{v['rate']:>8.1%}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    ap.add_argument("--out", type=Path, default=ROOT / "corpus/rationale-scoreboard.json")
    ap.add_argument("--top", type=int, default=12)
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
    before = json.loads(a.out.read_text()) if a.out.exists() else None
    rep = score(corpus, a.workers)
    a.out.write_text(json.dumps(rep, indent=1, ensure_ascii=False) + "\n")
    s, d = rep["surviving"], rep["dropped"]
    delta = lambda new, old: f" ({new - old:+d})" if before else ""
    ob, od = (before["surviving"], before["dropped"]) if before else ({}, {})
    print(f"{rep['labels']} labels, {rep['unmatched']} unmatched, {rep['matched_by_overlap']} matched by overlap")
    print(f"surviving {s['n']}{delta(s['n'], ob.get('n', 0))}: "
          f"{s['unfaithful']} still unfaithful{delta(s['unfaithful'], ob.get('unfaithful', 0))}, "
          f"{s['n'] - s['unfaithful']} faithful")
    print(f"dropped, rationale changed {d['n']}{delta(d['n'], od.get('n', 0))}: "
          f"{d['unfaithful']} unfaithful (fixed){delta(d['unfaithful'], od.get('unfaithful', 0))}, "
          f"{d['faithful']} faithful (check){delta(d['faithful'], od.get('faithful', 0))}")
    table("still unfaithful by rationale verb", s["by_verb"], a.top)
    table("still unfaithful by repo", s["by_repo"], a.top)
    print("\ntop rationale verb changes among drops:")
    for k, n in list(d["verb_changes"].items())[:a.top]:
        print(f"  {k:<30}{n:>5}")
    print(f"\nwrote {a.out}")


if __name__ == "__main__":
    main()
