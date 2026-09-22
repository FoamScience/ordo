#!/usr/bin/env python3
"""Turn the labeled silver set into fine-tuning rows for jeff's llm backend:
one System One request per hunk (the state and nouls judge-gold.py asks) with
the labels as the answers. The split is by hash of the hunk key, so a row
stays on its side across re-draws.

usage: scripts/silver-to-sft.py [--silver ~/.cache/ordo-corpus/silver.jsonl] [--holdout 500] [--out-dir DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path

from judgelib import noul_instructions, noul_labels, state_text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--silver", type=Path,
                    default=Path(os.environ.get("ORDO_CORPUS", "~/.cache/ordo-corpus")).expanduser() / "silver.jsonl")
    ap.add_argument("--holdout", type=int, default=500)
    ap.add_argument("--out-dir", type=Path, default=Path("."))
    a = ap.parse_args()
    rows = [json.loads(l) for l in a.silver.read_text().splitlines() if l.strip()]
    rows = [r for r in rows if noul_labels(r)]
    # the holdout is the lowest hashes, so its size is the only knob
    ranked = sorted(rows, key=lambda r: hashlib.sha1(r["key"].encode()).hexdigest())
    hold, train = ranked[: a.holdout], ranked[a.holdout:]
    for name, part in (("sft-train", train), ("sft-holdout", hold)):
        path = a.out_dir / f"{name}.jsonl"
        with path.open("w") as f:
            for r in part:
                f.write(json.dumps({
                    "key": r["key"],
                    "state": state_text(r),
                    "questions": {k: {"type": "noul", "instructions": v} for k, v in noul_instructions(r).items()},
                    "labels": noul_labels(r),
                }) + "\n")
        n_q = sum(len(noul_labels(r)) for r in part)
        print(f"{path}: {len(part)} hunks, {n_q} nouls")


if __name__ == "__main__":
    main()
