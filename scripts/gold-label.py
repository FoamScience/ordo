#!/usr/bin/env python3
"""Fill the null labels in corpus/gold.jsonl, one hunk at a time.

Shows the diff, the commit subject and ordo's claims; asks y/n per claim.
`s` skips the hunk for now, `q` saves and quits. Progress is written after
every hunk, so a session can stop and resume at any point.

`--human` audits the judge: it asks the same questions about a fixed sample
of gold hunks, about 4 per repo with hunks where the judge rejected one of
ordo's claims drawn first, into `labels_human` beside the judge's `labels`.
The judge's answers and its should_skip reasoning are shown only once a hunk
is answered. The sample is drawn once with a fixed seed and kept in
corpus/human-audit.json.

Usage: scripts/gold-label.py                      # interactive
       scripts/gold-label.py --apply labels.jsonl [target.jsonl]  # merge {key, labels} lines produced elsewhere
       scripts/gold-label.py --human              # label the audit sample
       scripts/gold-label.py --human-report       # human-vs-judge agreement per question
"""
import json
import os
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

GOLD = Path(__file__).resolve().parent.parent / "corpus/gold.jsonl"
AUDIT = GOLD.with_name("human-audit.json")
SKIP_WHY = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")) / "skip.labels.jsonl"
AUDIT_SEED, AUDIT_PER_REPO, AUDIT_REJECTED = 120, 4, 2


def ask(prompt):
    while True:
        r = input(f"  {prompt} [y/n/s/q] ").strip().lower()
        if r in ("y", "n", "s", "q"):
            return r


def apply(path, target=GOLD):
    rows = [json.loads(l) for l in target.read_text().splitlines() if l.strip()]
    by = {r["key"]: r for r in rows}
    n = 0
    for l in Path(path).read_text().splitlines():
        if not l.strip():
            continue
        got = json.loads(l)
        r = by.get(got["key"])
        if r is None:
            print(f"unknown key {got['key']}", file=sys.stderr)
            continue
        for k, v in got["labels"].items():
            if k == "findings":
                r["labels"]["findings"].update({f: v[f] for f in v if f in r["labels"]["findings"]})
            elif k in r["labels"] or k == "should_skip":
                r["labels"][k] = v
        n += 1
    target.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
    print(f"applied {n} label sets to {target}")


def pending(r, field="labels"):
    L = r[field]
    return L.get("should_skip") is None or \
        any(v is None for k, v in L.items() if k != "findings") or \
        any(v is None for v in L["findings"].values())


def show(r, i, total):
    print("=" * 78)
    print(f"[{i + 1}/{total}] {r['key']}  ({r['lang']})")
    print(f"commit:    {r['subject']}")
    print(f"rationale: {r['rationale']}")
    for d in r["details"]:
        print(f"  detail:  {d}")
    if r["noise"]:
        print("noise:     yes (ordo says formatting/generated)")
    for f in r["findings"]:
        print(f"finding:   [{f['level']}] {f['name']}: {f['message']}")
    print("-" * 78)
    print(r["diff"])
    print("-" * 78)


def label(r, L):
    """Ask for every null label of one hunk into `L`; `s` leaves the rest of it
    null. True when the labeler asked to quit."""
    questions = [("faithful", "does the rationale accurately describe this diff?"),
                 ("matches_commit", "is the rationale consistent with the commit message?")]
    if r["noise"]:
        questions.append(("noise_correct", "is this hunk really pure formatting/generated?"))
    questions.append(("should_skip", "can a reviewer skip this hunk without missing anything?"))
    for key, q in questions:
        if L.get(key) is not None:
            continue
        a = ask(q)
        if a in ("q", "s"):
            return a == "q"
        L[key] = a == "y"
    for name in L["findings"]:
        if L["findings"][name] is not None:
            continue
        a = ask(f"is the finding '{name}' warranted here?")
        if a in ("q", "s"):
            return a == "q"
        L["findings"][name] = a == "y"
    return False


def rejected(r):
    """The judge said no to one of ordo's claims about the hunk."""
    L = r["labels"]
    return any(L.get(k) is False for k in ("faithful", "matches_commit", "noise_correct")) or \
        False in L["findings"].values()


def audit_sample(rows):
    """Gold keys for the human audit, drawn once and then read back: per repo,
    up to AUDIT_REJECTED hunks the judge rejected a claim on, the rest of
    AUDIT_PER_REPO from the others, in a seeded shuffle."""
    if AUDIT.exists():
        return json.loads(AUDIT.read_text())["keys"]
    rng = random.Random(AUDIT_SEED)
    by_repo = defaultdict(list)
    for r in rows:
        by_repo[r["repo"]].append(r)
    keys = []
    for repo in sorted(by_repo):
        no = [r["key"] for r in by_repo[repo] if rejected(r)]
        yes = [r["key"] for r in by_repo[repo] if not rejected(r)]
        rng.shuffle(no)
        rng.shuffle(yes)
        k = min(AUDIT_REJECTED, len(no))
        keys += (no[:k] + yes + no[k:])[:AUDIT_PER_REPO]
    rng.shuffle(keys)
    AUDIT.write_text(json.dumps({"seed": AUDIT_SEED, "per_repo": AUDIT_PER_REPO,
                                 "rejected_per_repo": AUDIT_REJECTED, "keys": keys}, indent=1) + "\n")
    print(f"drew {len(keys)} hunks into {AUDIT}")
    return keys


def blank(r):
    L = {"faithful": None, "matches_commit": None}
    if r["noise"]:
        L["noise_correct"] = None
    L["should_skip"] = None
    L["findings"] = {n: None for n in r["labels"]["findings"]}
    return L


def answers(L):
    return {**{k: v for k, v in L.items() if k != "findings"},
            **{f"finding:{n}": v for n, v in L["findings"].items()}}


def reveal(r, why):
    """The judge's answers next to the human's, after the human is done."""
    yn = {True: "y", False: "n", None: "-"}
    human = answers(r["labels_human"])
    for q, j in answers(r["labels"]).items():
        h = human.get(q)
        mark = "" if h is None or h == j else "   <- differs"
        print(f"  judge {q}: {yn[j]}  you: {yn[h]}{mark}")
    if why.get(r["key"]):
        print(f"  judge on should_skip: {why[r['key']]}")


def human(rows):
    by = {r["key"]: r for r in rows}
    sample = [by[k] for k in audit_sample(rows)]
    why = {}
    if SKIP_WHY.exists():
        why = {x["key"]: x.get("why", "") for x in map(json.loads, filter(str.strip, SKIP_WHY.read_text().splitlines()))}
    for r in sample:
        r.setdefault("labels_human", blank(r))
    todo = [r for r in sample if pending(r, "labels_human")]
    print(f"{len(todo)} of {len(sample)} audit hunks unlabeled\n")
    for i, r in enumerate(todo):
        show(r, i, len(todo))
        quit_ = label(r, r["labels_human"])
        if not pending(r, "labels_human"):
            reveal(r, why)
        GOLD.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
        if quit_:
            break
    left = sum(pending(r, "labels_human") for r in sample)
    print(f"\nsaved. {left} audit hunks still unlabeled")


def human_report(rows):
    """Per question: hunks both answered, agreement, and each kind of
    disagreement; findings pooled under `finding`."""
    tally = defaultdict(Counter)
    differ = defaultdict(list)
    for r in rows:
        if "labels_human" not in r:
            continue
        judge = answers(r["labels"])
        for q, h in answers(r["labels_human"]).items():
            j = judge.get(q)
            if h is None or j is None:
                continue
            q = q.split(":")[0]
            tally[q][(j, h)] += 1
            if j != h:
                differ[q].append(r["key"])
    if not tally:
        return print("no human labels yet; run --human first")
    audited = [r for r in rows if "labels_human" in r and any(v is not None for v in answers(r["labels_human"]).values())]
    print(f"{len(audited)} hunks audited, {sum(map(rejected, audited))} drawn from the judge-rejected "
          f"pool (oversampled, so agreement leans on them)\n")
    print(f"{'question':<16}{'n':>4}{'agree':>8}{'judge y/you n':>15}{'judge n/you y':>15}")
    for q, c in tally.items():
        n = sum(c.values())
        agree = c[(True, True)] + c[(False, False)]
        print(f"{q:<16}{n:>4}{agree / n:>8.0%}{c[(True, False)]:>15}{c[(False, True)]:>15}")
    for q, keys in differ.items():
        print(f"\n{q} differs on:")
        for k in keys:
            print(f"  {k}")


def main():
    if len(sys.argv) in (3, 4) and sys.argv[1] == "--apply":
        return apply(sys.argv[2], *(Path(a) for a in sys.argv[3:]))
    rows = [json.loads(l) for l in GOLD.read_text().splitlines() if l.strip()]
    if sys.argv[1:] == ["--human"]:
        return human(rows)
    if sys.argv[1:] == ["--human-report"]:
        return human_report(rows)
    todo = [r for r in rows if pending(r)]
    print(f"{len(todo)} of {len(rows)} hunks unlabeled\n")
    for i, r in enumerate(todo):
        show(r, i, len(todo))
        quit_ = label(r, r["labels"])
        GOLD.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
        if quit_:
            break
    left = sum(pending(r) for r in rows)
    print(f"\nsaved. {left} hunks still unlabeled")


if __name__ == "__main__":
    main()
