#!/usr/bin/env python3
"""A should_skip label set beyond gold and silver: a stratified, seeded sample
of the judged sweep, rendered as ordo's own hunks for an Opus labeler, and the
labels collected into a file of their own (judged.jsonl is never touched).

Strata are repo x the noise flag the judged sweep recorded (kept as `noise`,
not re-read from today's engine), filled equally and topped up from the
strata with hunks to spare when one runs short. Each sampled row records its
stratum's population (`stratum_n`) so rates can be reweighted to the sweep.

The same sample carries a faithful label set: `faithful-chunks` reruns
today's engine on its hunks outside gold commits and shows each with the
rationale and details the engine now writes; `faithful-collect` keeps that
rationale beside each label, so the trainer drops a label once the engine
writes another.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/skip-sample.py draw [--n 5000] [--seed 131] [--more]
       scripts/skip-sample.py chunks DIR [--size 50]      # labeler inputs, DIR/cNNN.jsonl
       scripts/skip-sample.py collect DIR --model MODEL   # DIR/*.labels.jsonl -> judged.should_skip.jsonl
       scripts/skip-sample.py faithful-chunks DIR [--size 50] [--seed 125] [--unlabeled]
       scripts/skip-sample.py faithful-collect DIR --model MODEL   # -> judged.faithful.jsonl, keeping its other rows
"""
import argparse
import importlib.util
import json
import os
import random
import sys
from collections import defaultdict
from pathlib import Path

from corpuslib import ROOT

CORPUS = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
SAMPLE = CORPUS / "skip-sample.jsonl"
OUT = CORPUS / "judged.should_skip.jsonl"
FAITHFUL_OUT = CORPUS / "judged.faithful.jsonl"
SHOWN = ("key", "repo", "lang", "sha", "path", "subject", "diff")
FAITHFUL_SHOWN = ("key", "repo", "lang", "sha", "path", "subject", "rationale", "details", "diff")
DIFF_MAX = 8000

_spec = importlib.util.spec_from_file_location("gold_sample", ROOT / "scripts/gold-sample.py")
gold_sample = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gold_sample)


def keys_of(path):
    return {json.loads(l)["key"] for l in Path(path).read_text().splitlines() if l.strip()}


def quotas(sizes, n):
    """Equal share per stratum, a stratum smaller than its share giving the
    rest to the others."""
    q, left = {}, n
    order = sorted(sizes, key=lambda s: (sizes[s], s))
    for i, s in enumerate(order):
        q[s] = min(sizes[s], left // (len(order) - i))
        left -= q[s]
    return q


def draw(a):
    if not a.more and (OUT.exists() or FAITHFUL_OUT.exists()):
        sys.exit(f"{OUT} or {FAITHFUL_OUT} holds labels keyed to {SAMPLE}; move them aside before redrawing")
    taken = keys_of(ROOT / "corpus/gold.jsonl") | keys_of(CORPUS / "silver.jsonl")
    cells, seen = defaultdict(list), set()
    for l in (CORPUS / "judged.jsonl").open():
        r = json.loads(l)
        if r["key"] in taken or r["key"] in seen:
            continue
        seen.add(r["key"])
        cells[(r["repo"], r["noise"])].append(r["key"])
    rng = random.Random(a.seed)
    for k in sorted(cells):
        rng.shuffle(cells[k])
    population = {k: len(v) for k, v in cells.items()}
    old = {}
    if a.more:
        old = {r["key"]: r for l in SAMPLE.read_text().splitlines() if l.strip() for r in [json.loads(l)]}
        cells = {k: [key for key in v if key not in old] for k, v in cells.items()}
    sizes = {k: len(v) for k, v in cells.items()}
    want = quotas(sizes, a.n)
    got, have, cursor = {}, defaultdict(int), {k: 0 for k in cells}
    # the engine may no longer emit a sampled hunk at the same line: draw the
    # next key of the same stratum until each quota is met or the stratum dries
    while True:
        ask = []
        for k in sorted(cells):
            short = want[k] - have[k]
            take = cells[k][cursor[k]:cursor[k] + short]
            cursor[k] += len(take)
            ask += [(k, key) for key in take]
        if not ask:
            dry = {k: have[k] for k in cells if cursor[k] >= len(cells[k])}
            if len(got) >= a.n or not dry or quotas({**sizes, **dry}, a.n) == want:
                break
            want = quotas({**sizes, **dry}, a.n)
            continue
        stratum = dict((key, k) for k, key in ask)
        for h in gold_sample.draw_keys(CORPUS, [key for _, key in ask]):
            if h["key"] in stratum and h["key"] not in got:
                k = stratum[h["key"]]
                have[k] += 1
                got[h["key"]] = {**{f: h[f] for f in SHOWN}, "noise": k[1], "stratum_n": population[k]}
        print(f"  drawn {len(got)}/{a.n}", file=sys.stderr)
    rows = sorted({**old, **got}.values(), key=lambda r: r["key"])
    SAMPLE.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    print(f"wrote {len(rows)} hunks ({len(got)} new) to {SAMPLE}")


def chunks(a):
    """Shuffled, so a chunk is not fifty hunks of one commit; keys already
    chunked in DIR are left out and new chunks number on from the last."""
    a.dir.mkdir(parents=True, exist_ok=True)
    old = sorted(p for p in a.dir.glob("c*.jsonl") if ".labels" not in p.name)
    done = set().union(*map(keys_of, old)) if old else set()
    rows = [r for l in SAMPLE.read_text().splitlines() if l.strip()
            if (r := json.loads(l))["key"] not in done]
    random.Random(a.seed).shuffle(rows)
    start = int(old[-1].stem[1:]) + 1 if old else 0
    for i in range(0, len(rows), a.size):
        part = rows[i:i + a.size]
        (a.dir / f"c{start + i // a.size:03d}.jsonl").write_text(
            "".join(json.dumps({f: r[f] for f in SHOWN}, ensure_ascii=False) + "\n" for r in part))
    print(f"{(len(rows) + a.size - 1) // a.size} chunks of {a.size} in {a.dir}, from c{start:03d}")


def collect(a):
    want = [json.loads(l)["key"] for l in SAMPLE.read_text().splitlines() if l.strip()]
    got, bad = {}, []
    for f in sorted(a.dir.glob("*.labels.jsonl")):
        src = keys_of(f.with_name(f.name.replace(".labels", "")))
        for l in f.read_text().splitlines():
            if not l.strip():
                continue
            r = json.loads(l)
            v = r.get("labels", {}).get("should_skip")
            if r.get("key") not in src or not isinstance(v, bool) or r["key"] in got:
                bad.append(f"{f.name}: {l[:120]}")
                continue
            got[r["key"]] = {"key": r["key"], "should_skip": v, "why": r.get("why", ""),
                             "judge": "opus", "model": a.model}
    missing = [k for k in want if k not in got]
    for b in bad:
        print(f"bad line {b}", file=sys.stderr)
    for k in missing:
        print(f"missing {k}", file=sys.stderr)
    if bad or missing:
        sys.exit(f"{len(bad)} bad lines, {len(missing)} keys missing; nothing written")
    OUT.write_text("".join(json.dumps(got[k], ensure_ascii=False) + "\n" for k in want))
    print(f"wrote {len(want)} labels to {OUT}")


def faithful_chunks(a):
    """Chunks of the sample's hunks outside gold commits, as today's engine
    describes them (a hunk it no longer emits at the sampled line is left
    out), shuffled; DIR/manifest.json records each chunk's keys and the
    rationale and details shown."""
    gold = {tuple(k.split(":")[:2]) for k in keys_of(ROOT / "corpus/gold.jsonl")}
    labeled = keys_of(FAITHFUL_OUT) if a.unlabeled and FAITHFUL_OUT.exists() else set()
    sample = [r for l in SAMPLE.read_text().splitlines() if l.strip()
              if tuple((r := json.loads(l))["key"].split(":")[:2]) not in gold and r["key"] not in labeled]
    lang = {r["key"]: r["lang"] for r in sample}
    # a key two hunks share (one ends where the next starts) names the later
    # one, as the trainer's lookup does
    drawn = {h["key"]: h for h in gold_sample.draw_keys(CORPUS, [r["key"] for r in sample])}
    rows = [{**{f: h[f] for f in FAITHFUL_SHOWN}, "lang": lang[k]} for k, h in drawn.items()]
    for r in rows:
        if len(r["diff"]) > DIFF_MAX:
            r["diff"] = r["diff"][:DIFF_MAX] + f"\n... [diff truncated at {DIFF_MAX} of {len(r['diff'])} chars]"
    random.Random(a.seed).shuffle(rows)
    a.dir.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for i in range(0, len(rows), a.size):
        part = rows[i:i + a.size]
        cid = f"c{i // a.size:03d}"
        (a.dir / f"{cid}.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in part))
        manifest[cid] = [[r["key"], r["rationale"], r["details"]] for r in part]
    (a.dir / "manifest.json").write_text(json.dumps(manifest))
    print(f"{len(rows)} of {len(sample)} sampled hunks outside gold commits in {len(manifest)} chunks in {a.dir}")


def faithful_collect(a):
    manifest = json.loads((a.dir / "manifest.json").read_text())
    out, bad = [], []
    for cid, want in manifest.items():
        p = a.dir / f"{cid}.labels.jsonl"
        got = [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []
        if [g.get("key") for g in got] != [k for k, _, _ in want]:
            bad.append(f"{cid}: missing, or keys differ from the chunk")
            continue
        for (key, rationale, details), g in zip(want, got):
            v = g.get("labels", {}).get("faithful")
            if not isinstance(v, bool) or not str(g.get("why", "")).strip():
                bad.append(f"{cid} {key}: bad answer")
                continue
            out.append({"key": key, "faithful": v, "why": g["why"], "rationale": rationale,
                        "details": details, "judge": "opus", "model": a.model})
    for b in bad:
        print(b, file=sys.stderr)
    if bad:
        sys.exit(f"{len(bad)} problems; nothing written")
    if FAITHFUL_OUT.exists():
        old = [json.loads(l) for l in FAITHFUL_OUT.read_text().splitlines() if l.strip()]
        new = {o["key"] for o in out}
        out = [r for r in old if r["key"] not in new] + out
    out.sort(key=lambda r: r["key"])
    FAITHFUL_OUT.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in out))
    print(f"wrote {len(out)} labels ({sum(not r['faithful'] for r in out)} unfaithful) to {FAITHFUL_OUT}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("draw")
    d.add_argument("--n", type=int, default=5000)
    d.add_argument("--seed", type=int, default=131)
    d.add_argument("--more", action="store_true", help="add --n hunks to the sample, keeping its rows and labels")
    c = sub.add_parser("chunks")
    c.add_argument("dir", type=Path)
    c.add_argument("--size", type=int, default=50)
    c.add_argument("--seed", type=int, default=131)
    k = sub.add_parser("collect")
    k.add_argument("dir", type=Path)
    k.add_argument("--model", required=True)
    f = sub.add_parser("faithful-chunks")
    f.add_argument("dir", type=Path)
    f.add_argument("--size", type=int, default=50)
    f.add_argument("--seed", type=int, default=125)
    f.add_argument("--unlabeled", action="store_true", help="leave out hunks already in judged.faithful.jsonl")
    fc = sub.add_parser("faithful-collect")
    fc.add_argument("dir", type=Path)
    fc.add_argument("--model", required=True)
    a = ap.parse_args()
    {"draw": draw, "chunks": chunks, "collect": collect, "faithful-chunks": faithful_chunks,
     "faithful-collect": faithful_collect}[a.cmd](a)


if __name__ == "__main__":
    main()
