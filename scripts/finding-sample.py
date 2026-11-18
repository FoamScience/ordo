#!/usr/bin/env python3
"""A finding label set beyond gold and silver: every hunk of the judged sweep
that carried a finding, outside gold and silver and gold's commits, rerun
through today's engine and kept where it still carries one, rendered for an
Opus labeler who answers, per finding, whether the warning is warranted (the
question gold and silver were labeled with). judged.jsonl is never touched.

Usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/finding-sample.py draw
       scripts/finding-sample.py chunks DIR [--size 40] [--seed 126]  # DIR/cNNN.jsonl + manifest.json
       scripts/finding-sample.py collect DIR --model MODEL            # -> judged.findings.jsonl
"""
import argparse
import importlib.util
import json
import os
import random
import sys
from pathlib import Path

from corpuslib import ROOT

CORPUS = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
SAMPLE = CORPUS / "finding-sample.jsonl"
OUT = CORPUS / "judged.findings.jsonl"
SHOWN = ("key", "repo", "lang", "sha", "path", "subject", "rationale", "details", "findings", "diff")
DIFF_MAX = 8000

_spec = importlib.util.spec_from_file_location("gold_sample", ROOT / "scripts/gold-sample.py")
gold_sample = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gold_sample)


def keys_of(path):
    return [json.loads(l)["key"] for l in Path(path).read_text().splitlines() if l.strip()]


def question(f):
    return f"Is this reviewer warning warranted for this code? Warning: {f['message']}"


def draw(a):
    if OUT.exists():
        sys.exit(f"{OUT} holds labels keyed to {SAMPLE}; move it aside before redrawing")
    taken = set(keys_of(ROOT / "corpus/gold.jsonl")) | set(keys_of(CORPUS / "silver.jsonl"))
    gold_commits = {tuple(k.split(":")[:2]) for k in keys_of(ROOT / "corpus/gold.jsonl")}
    want = []
    for l in (CORPUS / "judged.jsonl").open():
        r = json.loads(l)
        if r.get("findings") and r["key"] not in taken and tuple(r["key"].split(":")[:2]) not in gold_commits:
            want.append(r["key"])
    want = list(dict.fromkeys(want))
    # a key two hunks share names the later one, as the trainer's lookup does
    drawn = {h["key"]: h for h in gold_sample.draw_keys(CORPUS, want)}
    rows = [{f: h[f] for f in SHOWN} for h in drawn.values() if h["findings"]]
    rows.sort(key=lambda r: r["key"])
    SAMPLE.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    print(f"{len(want)} swept hunks with findings; {len(rows)} still carry one "
          f"({sum(len(r['findings']) for r in rows)} findings) -> {SAMPLE}")


def chunks(a):
    rows = [json.loads(l) for l in SAMPLE.read_text().splitlines() if l.strip()]
    random.Random(a.seed).shuffle(rows)
    a.dir.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for i in range(0, len(rows), a.size):
        part = rows[i:i + a.size]
        cid = f"c{i // a.size:03d}"
        out = []
        for r in part:
            r = dict(r)
            if len(r["diff"]) > DIFF_MAX:
                r["diff"] = r["diff"][:DIFF_MAX] + f"\n... [diff truncated at {DIFF_MAX} of {len(r['diff'])} chars]"
            r["questions"] = {f"finding:{f['name']}": question(f) for f in r["findings"]}
            out.append(r)
        (a.dir / f"{cid}.jsonl").write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in out))
        manifest[cid] = [[r["key"], sorted(r["questions"])] for r in out]
    (a.dir / "manifest.json").write_text(json.dumps(manifest))
    print(f"{len(rows)} hunks in {len(manifest)} chunks in {a.dir}")


def collect(a):
    manifest = json.loads((a.dir / "manifest.json").read_text())
    shown = {r["key"]: r["findings"] for l in SAMPLE.read_text().splitlines() if l.strip() for r in [json.loads(l)]}
    out, bad = [], []
    for cid, want in manifest.items():
        p = a.dir / f"{cid}.labels.jsonl"
        got = [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []
        if [g.get("key") for g in got] != [k for k, _ in want]:
            bad.append(f"{cid}: missing, or keys differ from the chunk")
            continue
        for (key, qs), g in zip(want, got):
            L = g.get("labels", {})
            if sorted(L) != qs or not all(isinstance(v, bool) for v in L.values()) or not str(g.get("why", "")).strip():
                bad.append(f"{cid} {key}: bad answer {L}")
                continue
            out.append({"key": key, "findings": {q.split(":", 1)[1]: v for q, v in L.items()}, "why": g["why"],
                        "shown": [{k: f[k] for k in ("name", "level")} for f in shown[key]],
                        "judge": "opus", "model": a.model})
    for b in bad:
        print(b, file=sys.stderr)
    if bad:
        sys.exit(f"{len(bad)} problems; nothing written")
    out.sort(key=lambda r: r["key"])
    OUT.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in out))
    n = sum(len(r["findings"]) for r in out)
    real = sum(v for r in out for v in r["findings"].values())
    print(f"wrote {len(out)} hunks, {n} finding labels ({real} warranted) to {OUT}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("draw")
    c = sub.add_parser("chunks")
    c.add_argument("dir", type=Path)
    c.add_argument("--size", type=int, default=40)
    c.add_argument("--seed", type=int, default=126)
    k = sub.add_parser("collect")
    k.add_argument("dir", type=Path)
    k.add_argument("--model", required=True)
    a = ap.parse_args()
    {"draw": draw, "chunks": chunks, "collect": collect}[a.cmd](a)


if __name__ == "__main__":
    main()
