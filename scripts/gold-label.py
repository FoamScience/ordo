#!/usr/bin/env python3
"""Fill the null labels in corpus/gold.jsonl, one hunk at a time.

Shows the diff, the commit subject and ordo's claims; asks y/n per claim.
`s` skips the hunk for now, `q` saves and quits. Progress is written after
every hunk, so a session can stop and resume at any point.

Usage: scripts/gold-label.py                      # interactive
       scripts/gold-label.py --apply labels.jsonl [target.jsonl]  # merge {key, labels} lines produced elsewhere
"""
import json
import sys
from pathlib import Path

GOLD = Path(__file__).resolve().parent.parent / "corpus/gold.jsonl"


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
            elif k in r["labels"]:
                r["labels"][k] = v
        n += 1
    target.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
    print(f"applied {n} label sets to {target}")


def main():
    if len(sys.argv) in (3, 4) and sys.argv[1] == "--apply":
        return apply(sys.argv[2], *(Path(a) for a in sys.argv[3:]))
    rows = [json.loads(l) for l in GOLD.read_text().splitlines() if l.strip()]

    def pending(r):
        L = r["labels"]
        return any(v is None for k, v in L.items() if k != "findings") or \
            any(v is None for v in L["findings"].values())

    todo = [r for r in rows if pending(r)]
    print(f"{len(todo)} of {len(rows)} hunks unlabeled\n")
    for i, r in enumerate(todo):
        L = r["labels"]
        print("=" * 78)
        print(f"[{i + 1}/{len(todo)}] {r['key']}  ({r['lang']})")
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
        questions = [("faithful", "does the rationale accurately describe this diff?"),
                     ("matches_commit", "is the rationale consistent with the commit message?")]
        if r["noise"]:
            questions.append(("noise_correct", "is this hunk really pure formatting/generated?"))
        quit_ = skip = False
        for key, q in questions:
            if L[key] is not None:
                continue
            a = ask(q)
            if a == "q":
                quit_ = True; break
            if a == "s":
                skip = True; break
            L[key] = a == "y"
        if not (quit_ or skip):
            for name in L["findings"]:
                if L["findings"][name] is not None:
                    continue
                a = ask(f"is the finding '{name}' warranted here?")
                if a == "q":
                    quit_ = True; break
                if a == "s":
                    break
                L["findings"][name] = a == "y"
        GOLD.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
        if quit_:
            break
    left = sum(pending(r) for r in rows)
    print(f"\nsaved. {left} hunks still unlabeled")


if __name__ == "__main__":
    main()
