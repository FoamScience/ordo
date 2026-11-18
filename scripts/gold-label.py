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

A model can audit the same sample instead: its answers go in `labels_audit`,
shaped like `labels_human`, with the model id in `audited_by`; this script
renders that sample for it (`--audit-render`), merges its answers
(`--audit-merge`) and reports on them (`--audit-report`).

A model audit, end to end:
  1. `--audit-render DIR` writes DIR/chunk<i>.txt (rubric, then each hunk as
     --human shows it, then its question ids) and DIR/manifest.json (chunk ->
     key -> question ids). Judge labels and skip reasons never reach the files.
  2. The model reads each chunk<i>.txt and writes DIR/chunk<i>.answers.jsonl,
     one line per hunk in chunk order:
     {"key": ..., "answers": {question id: true|false, ...}, "why": "..."}
  3. `--audit-merge DIR/chunk*.answers.jsonl --auditor MODEL` checks every
     chunk of the manifest is answered with its keys in order, exactly its
     question ids, booleans only and a non-empty why; then backs up the gold
     file to DIR/gold.jsonl.bak and adds `labels_audit` and `audited_by` to
     the sampled hunks, refusing hunks that already carry an audit unless
     `--reaudit` is given: then the previous `labels_audit` and `audited_by`
     are appended to `labels_audit_prev` before being replaced.

After a refresh (gold-sample.py --refresh) drops sampled hunks or changes
their rationale, `--audit-replace BEFORE` swaps each sampled key missing from
gold for a seeded draw from the same repo and pool (judge-rejected or not, as
the key was in BEFORE, the pre-refresh gold file), and `--audit-render DIR
--keys FILE` renders only the keys listed in FILE (one per line), so just the
stale and replaced hunks are re-audited.

Usage: scripts/gold-label.py                      # interactive
       scripts/gold-label.py --apply labels.jsonl [target.jsonl]  # merge {key, labels} lines produced elsewhere
       scripts/gold-label.py --human              # label the audit sample
       scripts/gold-label.py --human-report       # human-vs-judge agreement per question
       scripts/gold-label.py --audit-report       # the same for a model auditor's `labels_audit`
       scripts/gold-label.py --audit-render DIR [--chunks N] [--gold PATH] [--keys FILE]   # auditor inputs, N files (default 4)
       scripts/gold-label.py --audit-merge ANSWERS... --auditor MODEL [--gold PATH] [--reaudit]
       scripts/gold-label.py --audit-replace BEFORE.jsonl [--gold PATH]   # resample sampled keys gone from gold
"""
import contextlib
import io
import json
import os
import random
import shutil
import sys
from collections import Counter, defaultdict
from pathlib import Path

from judgelib import QUESTIONS

GOLD = Path(__file__).resolve().parent.parent / "corpus/gold.jsonl"
AUDIT = GOLD.with_name("human-audit.json")
SKIP_WHY = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")) / "skip.labels.jsonl"
SKIP_PROMPT = SKIP_WHY.with_name("skip.prompt.txt")
AUDIT_SEED, AUDIT_PER_REPO, AUDIT_REJECTED = 120, 4, 2
LABEL_FIELDS = ("labels", "labels_human", "labels_audit", "audited_by", "labels_audit_prev")


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
    questions = [("faithful", "does the rationale state nothing false or misleading about this diff (terse but true is faithful)?"),
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


def audit_sample(rows, audit=AUDIT):
    """Gold keys for the human audit, drawn once and then read back: per repo,
    up to AUDIT_REJECTED hunks the judge rejected a claim on, the rest of
    AUDIT_PER_REPO from the others, in a seeded shuffle."""
    if audit.exists():
        return json.loads(audit.read_text())["keys"]
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
    audit.write_text(json.dumps({"seed": AUDIT_SEED, "per_repo": AUDIT_PER_REPO,
                                 "rejected_per_repo": AUDIT_REJECTED, "keys": keys}, indent=1) + "\n")
    print(f"drew {len(keys)} hunks into {audit}")
    return keys


def audit_replace(rows, before, audit=AUDIT):
    """Swap each sampled key missing from `rows` for a draw from the same repo
    and pool it had in `before`, seeded by the key; recorded under "replaced"."""
    sample = json.loads(audit.read_text())
    have = {r["key"]: r for r in rows}
    was = {r["key"]: r for r in before}
    for i, k in enumerate(sample["keys"]):
        if k in have:
            continue
        if k not in was:
            sys.exit(f"{k}: in neither gold nor {len(before)} BEFORE rows, so its pool is unknown")
        pool = rejected(was[k])
        cand = sorted(r["key"] for r in rows if r["repo"] == was[k]["repo"]
                      and rejected(r) == pool and r["key"] not in sample["keys"])
        if not cand:
            sys.exit(f"{k}: no {'rejected' if pool else 'other'} hunk left in {was[k]['repo']}")
        new = random.Random(f"{AUDIT_SEED}:{k}").choice(cand)
        sample["keys"][i] = new
        sample.setdefault("replaced", {})[k] = new
        print(f"{k} -> {new} ({'rejected' if pool else 'other'} pool, {len(cand)} candidates)")
    audit.write_text(json.dumps(sample, indent=1) + "\n")


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


def agreement_report(rows, field="labels_human", who="you", hint="run --human first"):
    """Per question: hunks both answered, how many of them came from the
    judge-rejected pool, agreement, and each kind of disagreement; findings
    pooled under `finding`. Disagreeing keys from that pool are starred."""
    tally = defaultdict(Counter)
    pool = Counter()
    differ = defaultdict(list)
    for r in rows:
        if field not in r:
            continue
        judge = answers(r["labels"])
        for q, h in answers(r[field]).items():
            j = judge.get(q)
            if h is None or j is None:
                continue
            q = q.split(":")[0]
            tally[q][(j, h)] += 1
            pool[q] += rejected(r)
            if j != h:
                differ[q].append(f"{'*' if rejected(r) else ' '} {r['key']}")
    if not tally:
        return print(f"no {field} yet; {hint}")
    audited = [r for r in rows if field in r and any(v is not None for v in answers(r[field]).values())]
    print(f"{len(audited)} hunks audited, {sum(map(rejected, audited))} drawn from the judge-rejected "
          f"pool (oversampled, so agreement leans on them)")
    by = Counter(r.get("audited_by", who) for r in audited)
    print("answered by: " + ", ".join(f"{k} ({v})" for k, v in by.items()) + "\n")
    yn, ny = f"judge y/{who} n", f"judge n/{who} y"
    w = len(yn) + 2
    print(f"{'question':<16}{'n':>4}{'pool':>6}{'agree':>8}{yn:>{w}}{ny:>{w}}")
    for q, c in tally.items():
        n = sum(c.values())
        agree = c[(True, True)] + c[(False, False)]
        print(f"{q:<16}{n:>4}{pool[q]:>6}{agree / n:>8.0%}{c[(True, False)]:>{w}}{c[(False, True)]:>{w}}")
    for q, keys in differ.items():
        print(f"\n{q} differs on (* judge-rejected pool):")
        for k in keys:
            print(f"  {k}")


def audit_questions(r):
    """Question id -> wording an auditor answers for one hunk, the ids `blank` holds."""
    L = blank(r)
    qs = {k: QUESTIONS[k] for k in L if k != "findings"}
    for name in L["findings"]:
        msg = next((f["message"] for f in r["findings"] if f["name"] == name), name)
        qs[f"finding:{name}"] = f"Is this reviewer warning warranted for this code? Warning: {msg}"
    return qs


def audit_render(rows, out, chunks=4, only=None, audit=AUDIT):
    """The audit sample, or its keys in `only`, as `chunks` auditor input files
    plus manifest.json. Hunks are shown from a copy without any label field,
    and the output is checked for the judge's should_skip reasons."""
    rubric = [l for l in SKIP_PROMPT.read_text().splitlines()
              if not l.startswith(("Input:", "Output:", "Do not skip lines", "{"))]
    by = {r["key"]: r for r in rows}
    keys = audit_sample(rows, audit)
    if only is not None:
        assert set(only) <= set(keys), f"not in the audit sample: {sorted(set(only) - set(keys))}"
        keys = [k for k in keys if k in only]
    why = {}
    if SKIP_WHY.exists():
        why = {x["key"]: x.get("why", "") for x in map(json.loads, filter(str.strip, SKIP_WHY.read_text().splitlines()))}
    out.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for c in range(chunks):
        part = keys[c::chunks]
        buf = io.StringIO()
        buf.write("RUBRIC for should_skip (answer the other questions from the hunk itself):\n")
        buf.write("\n".join(rubric).strip() + "\n\n")
        for i, k in enumerate(part):
            r = {f: v for f, v in by[k].items() if f not in LABEL_FIELDS}
            with contextlib.redirect_stdout(buf):
                print(f"repo: {r['repo']}   path: {r['path']}   lang: {r['lang']}   sha: {r['sha']}")
                show(r, i, len(part))
                print("QUESTIONS (answer every id, true or false):")
                for q, text in audit_questions(by[k]).items():
                    print(f"  {q}: {text}")
                print()
        text = buf.getvalue()
        public = {k: "\n".join(map(str, (by[k]["diff"], by[k]["subject"], by[k]["rationale"], *by[k]["details"]))) for k in part}
        leaked = [k for k in part if why.get(k) and why[k] not in public[k] and why[k] in text]
        assert not leaked, f"judge skip reasons in chunk{c}: {leaked}"
        (out / f"chunk{c}.txt").write_text(text)
        manifest[c] = {k: list(audit_questions(by[k])) for k in part}
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))
    print(f"rendered {len(keys)} hunks into {out}: " + ", ".join(f"chunk{c} ({len(m)})" for c, m in manifest.items()))


def audit_merge(paths, auditor, target=GOLD, reaudit=False):
    """Validate a model's answers against the manifest beside them, back up
    `target` there and add labels_audit + audited_by to the sampled hunks;
    with `reaudit`, an existing audit moves to labels_audit_prev first."""
    out = paths[0].parent
    manifest = json.loads((out / "manifest.json").read_text())
    files = {p.name.split(".")[0].removeprefix("chunk"): p for p in paths}
    errs = [f"chunk{c}: no answers file" for c in manifest if c not in files]
    errs += [f"{p}: not in manifest" for c, p in files.items() if c not in manifest or p.parent != out]
    got = {}
    for c, want in manifest.items():
        if c not in files:
            continue
        rows = [json.loads(l) for l in files[c].read_text().splitlines() if l.strip()]
        if [r["key"] for r in rows] != list(want):
            errs.append(f"chunk{c}: keys/order differ from manifest")
        for r in rows:
            qs, a = want.get(r["key"], []), r.get("answers")
            if not isinstance(a, dict):
                errs.append(f"chunk{c} {r['key']}: no answers object")
                continue
            if sorted(a) != sorted(qs):
                errs.append(f"chunk{c} {r['key']}: answers {sorted(a)} vs {sorted(qs)}")
            if not all(isinstance(v, bool) for v in a.values()):
                errs.append(f"chunk{c} {r['key']}: non-boolean answers")
            if not (isinstance(r.get("why"), str) and r["why"].strip()):
                errs.append(f"chunk{c} {r['key']}: no why")
            got[r["key"]] = {q: a[q] for q in qs if q in a}
    old = [json.loads(l) for l in target.read_text().splitlines() if l.strip()]
    if not reaudit:
        errs += [f"{r['key']}: already audited (pass --reaudit)" for r in old if r["key"] in got and "labels_audit" in r]
    if errs:
        sys.exit("\n".join(errs))
    shutil.copy(target, out / "gold.jsonl.bak")
    merged = set(got)
    new = []
    for o in old:
        r = dict(o)
        a = got.pop(r["key"], None)
        if a is not None:
            if "labels_audit" in r:
                r["labels_audit_prev"] = r.get("labels_audit_prev", []) + \
                    [{"labels": r["labels_audit"], "audited_by": r["audited_by"]}]
            L = {k: v for k, v in a.items() if not k.startswith("finding:")}
            L["findings"] = {k.split(":", 1)[1]: v for k, v in a.items() if k.startswith("finding:")}
            r["labels_audit"], r["audited_by"] = L, auditor
        new.append(r)
    assert not got, f"keys not in {target}: {sorted(got)}"
    audit = {"labels_audit", "audited_by", "labels_audit_prev"}
    for o, n in zip(old, new):
        assert o["key"] in merged or n == o, o["key"]
        assert {k: n[k] for k in o if k not in audit} == {k: v for k, v in o.items() if k not in audit} \
            and set(n) - set(o) <= audit, o["key"]
    target.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in new))
    redone = sum(len(n.get("labels_audit_prev", [])) > len(o.get("labels_audit_prev", [])) for o, n in zip(old, new))
    print(f"merged {sum('labels_audit' in r for r in new) - sum('labels_audit' in r for r in old)} new and "
          f"{redone} re-audits by {auditor} into {target}; backup {out / 'gold.jsonl.bak'}")


def opt(args, name, default=None):
    if name not in args:
        return default
    i = args.index(name)
    if i + 1 == len(args):
        sys.exit(f"{name} needs a value")
    v = args[i + 1]
    del args[i:i + 2]
    return v


def main():
    args = sys.argv[1:]
    if args[:1] in (["--audit-render"], ["--audit-merge"], ["--audit-replace"]):
        gold = Path(opt(args, "--gold", GOLD))
        audit = gold.with_name("human-audit.json")
        if args[0] == "--audit-merge":
            auditor = opt(args, "--auditor")
            reaudit = "--reaudit" in args
            if reaudit:
                args.remove("--reaudit")
            if not auditor or len(args) < 2:
                sys.exit("usage: --audit-merge ANSWERS... --auditor MODEL [--gold PATH] [--reaudit]")
            return audit_merge([Path(a) for a in args[1:]], auditor, gold, reaudit)
        rows = [json.loads(l) for l in gold.read_text().splitlines() if l.strip()]
        if args[0] == "--audit-replace":
            if len(args) != 2:
                sys.exit("usage: --audit-replace BEFORE.jsonl [--gold PATH]")
            return audit_replace(rows, [json.loads(l) for l in Path(args[1]).read_text().splitlines() if l.strip()],
                                 audit)
        chunks = int(opt(args, "--chunks", 4))
        only = opt(args, "--keys")
        if len(args) != 2:
            sys.exit("usage: --audit-render DIR [--chunks N] [--gold PATH] [--keys FILE]")
        if only is not None:
            only = set(filter(None, map(str.strip, Path(only).read_text().splitlines())))
        return audit_render(rows, Path(args[1]), chunks, only, audit)
    if len(sys.argv) in (3, 4) and sys.argv[1] == "--apply":
        return apply(sys.argv[2], *(Path(a) for a in sys.argv[3:]))
    rows = [json.loads(l) for l in GOLD.read_text().splitlines() if l.strip()]
    if sys.argv[1:] == ["--human"]:
        return human(rows)
    if sys.argv[1:] == ["--human-report"]:
        return agreement_report(rows)
    if sys.argv[1:] == ["--audit-report"]:
        return agreement_report(rows, "labels_audit", "auditor", "no model audit merged")
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
