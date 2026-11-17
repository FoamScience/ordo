#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scikit-learn", "lightgbm"]
# ///
"""Train hunch's first head, P(should_skip), on the engine's per-hunk features:
logistic regression and LightGBM, trained on silver, scored on gold and by
leave-one-repo-out over silver. The trees ship only when they beat the
regression by MIN_GAIN AUROC on gold. Writes src/hunch.json (model, feature
schema, metrics, provenance) and prints the comparison against the engine's
rule-based noise flag at its own hide rate.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py [--out FILE] [--workers N] [--extra FILE]

--extra adds labeled hunks to training, one {key, should_skip} per line
(e.g. $ORDO_CORPUS/judged.should_skip.jsonl); evaluation stays on gold and silver.

Build the engine first (cargo build --release); the corpus repos come from
scripts/corpus-fetch.sh.
"""
import argparse
import hashlib
import json
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import lightgbm
import numpy as np
import sklearn
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import average_precision_score, brier_score_loss, roc_auc_score
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

from corpuslib import ROOT, commit_input, engine_output, with_features

SEED = 0
HISTORY_DROP = 0.2
HISTORY = ("has_history", "churn", "fixes")
MIN_GAIN = 0.03
LGBM = dict(n_estimators=200, learning_rate=0.05, num_leaves=15, min_child_samples=20,
            subsample=0.8, subsample_freq=1, colsample_bytree=0.8, random_state=SEED,
            deterministic=True, force_row_wise=True, n_jobs=1, verbose=-1)


def commit_features(corpus, repo, sha):
    """key -> feature vector for one commit's hunks, and the schema."""
    d = str(corpus / repo)
    parent, inp = commit_input(d, sha)
    if not parent:
        return {}, None
    out = engine_output(with_features(d, sha, inp))
    vecs = {f"{repo}:{sha[:8]}:{f['path']}:{h['new_range'][0]}": h["features"]
            for f in out["files"] for h in f["hunks"] if h.get("features") is not None}
    return vecs, out.get("feature_schema")


def load(corpus, records, workers):
    commits = sorted({(r["repo"], r["sha"]) for r in records})
    vecs, schemas = {}, set()
    with ThreadPoolExecutor(workers) as ex:
        for v, s in ex.map(lambda c: commit_features(corpus, *c), commits):
            vecs.update(v)
            if s:
                schemas.add(json.dumps(s, sort_keys=True))
    if len(schemas) != 1:
        sys.exit(f"expected one feature schema, got {len(schemas)}")
    missing = [r["key"] for r in records if r["key"] not in vecs]
    if missing:
        print(f"{len(missing)} labeled hunks not found in engine output, dropped "
              f"(first: {missing[0]})", file=sys.stderr)
    kept = [r for r in records if r["key"] in vecs]
    x = np.array([vecs[r["key"]] for r in kept], dtype=float)
    y = np.array([bool(r["labels"]["should_skip"]) for r in kept])
    repos = np.array([r["repo"] for r in kept])
    return x, y, repos, json.loads(schemas.pop()), len(missing)


def extra_records(path):
    """{key, should_skip} lines as silver-shaped records; the key's sha is the
    short one, which git and the engine's keys both accept."""
    for l in path.read_text().splitlines():
        if l.strip():
            r = json.loads(l)
            repo, sha, _ = r["key"].split(":", 2)
            yield {"key": r["key"], "repo": repo, "sha": sha, "labels": {"should_skip": r["should_skip"]}}


def drop_history(x, cols, rng):
    x = x.copy()
    x[np.ix_(rng.random(len(x)) < HISTORY_DROP, cols)] = 0
    return x


def fit(kind, x, y):
    if kind == "lr":
        m = make_pipeline(StandardScaler(), LogisticRegression(max_iter=5000, random_state=SEED))
    else:
        m = lightgbm.LGBMClassifier(**LGBM)
    return m.fit(x, y)


def at_hide_rate(score, base, y, repos):
    """Hide as many hunks as the noise flag does, highest score first; wrong
    hides and missed skips overall and per repo."""
    hidden = np.zeros(len(score), dtype=bool)
    hidden[np.argsort(-score, kind="stable")[: int(base.sum())]] = True
    counts = lambda m: {"hidden": int(hidden[m].sum()),
                        "wrong_hides": int((hidden & ~y)[m].sum()),
                        "missed_skips": int((~hidden & y)[m].sum())}
    return {"all": counts(np.ones(len(y), dtype=bool)),
            "per_repo": {r: counts(repos == r) for r in sorted(set(repos))}}


def metrics(score, base, y, repos):
    return {"auroc": round(roc_auc_score(y, score), 4),
            "pr_auc": round(average_precision_score(y, score), 4),
            "brier": round(brier_score_loss(y, score), 4),
            "at_noise_hide_rate": at_hide_rate(score, base, y, repos)}


def evaluate(scores, base, y, repos):
    return {name: metrics(s, base, y, repos) for name, s in [("noise", base), *scores.items()]}


def sha256(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def table(title, ev):
    print(f"\n{title}")
    print(f"{'model':8} {'AUROC':>6} {'PR-AUC':>6} {'Brier':>6} {'wrong':>6} {'missed':>6}")
    for name, m in ev.items():
        a = m["at_noise_hide_rate"]["all"]
        print(f"{name:8} {m['auroc']:6.3f} {m['pr_auc']:6.3f} {m['brier']:6.3f} "
              f"{a['wrong_hides']:6} {a['missed_skips']:6}")
    print(f"{'repo':14} " + " ".join(f"{n:>13}" for n in ev) + "   (wrong/missed)")
    for r in next(iter(ev.values()))["at_noise_hide_rate"]["per_repo"]:
        cells = (ev[n]["at_noise_hide_rate"]["per_repo"][r] for n in ev)
        print(f"{r:14} " + " ".join(f"{c['wrong_hides']:>6}/{c['missed_skips']:<6}" for c in cells))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "src/hunch.json")
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    ap.add_argument("--extra", type=Path, help="more training labels, {key, should_skip} per line")
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", "~/.cache/ordo-corpus")).expanduser()
    gold_path, silver_path = ROOT / "corpus/gold.jsonl", corpus / "silver.jsonl"
    read = lambda p: [json.loads(l) for l in p.read_text().splitlines() if l.strip()]
    xs, ys, rs, schema, s_missing = load(corpus, read(silver_path), a.workers)
    xg, yg, rg, gschema, g_missing = load(corpus, read(gold_path), a.workers)
    if gschema != schema:
        sys.exit("silver and gold feature schemas differ")
    names = schema["names"]
    noise, hist = names.index("noise"), [names.index(c) for c in HISTORY]
    xt, yt, rt, e_missing = xs, ys, rs, None
    if a.extra:
        xe, ye, re_, eschema, e_missing = load(corpus, list(extra_records(a.extra)), a.workers)
        if eschema != schema:
            sys.exit("silver and extra feature schemas differ")
        xt, yt, rt = np.vstack([xs, xe]), np.concatenate([ys, ye]), np.concatenate([rs, re_])
    xt = drop_history(xt, hist, np.random.default_rng(SEED))

    models = {k: fit(k, xt, yt) for k in ("lr", "trees")}
    xg_nohist = xg.copy()
    xg_nohist[:, hist] = 0
    gold = evaluate({k: m.predict_proba(xg)[:, 1] for k, m in models.items()}, xg[:, noise], yg, rg)
    gold_nohist = evaluate({k: m.predict_proba(xg_nohist)[:, 1] for k, m in models.items()},
                           xg[:, noise], yg, rg)

    loro = {k: np.zeros(len(ys)) for k in models}
    for r in sorted(set(rs)):
        held = rs == r
        for k in models:
            loro[k][held] = fit(k, xt[rt != r], yt[rt != r]).predict_proba(xs[held])[:, 1]
    loro_ev = evaluate(loro, xs[:, noise], ys, rs)
    loro_per_repo = {r: {k: round(roc_auc_score(ys[rs == r], s[rs == r]), 4)
                         for k, s in [("noise", xs[:, noise]), *loro.items()]}
                     for r in sorted(set(rs)) if len(set(ys[rs == r])) == 2}

    gain = gold["trees"]["auroc"] - gold["lr"]["auroc"]
    chosen = "trees" if gain >= MIN_GAIN else "lr"
    if chosen == "lr":
        scaler, lr = models["lr"].named_steps.values()
        model = {"type": "logistic", "mean": scaler.mean_.tolist(), "scale": scaler.scale_.tolist(),
                 "coef": lr.coef_[0].tolist(), "intercept": float(lr.intercept_[0])}
    else:
        model = {"type": "lightgbm", **models["trees"].booster_.dump_model()}

    report = {
        "head": "should_skip",
        "model": model,
        "feature_schema": schema,
        "selection": {"rule": f"lightgbm if gold AUROC gain over logistic >= {MIN_GAIN}",
                      "gold_auroc_gain": round(gain, 4), "chosen": model["type"]},
        "metrics": {"gold": gold, "gold_without_history": gold_nohist,
                    "leave_one_repo_out": loro_ev, "leave_one_repo_out_auroc_per_repo": loro_per_repo},
        "provenance": {
            "label": "labels.should_skip",
            "train": {"file": "silver.jsonl", "sha256": sha256(silver_path), "hunks": len(ys),
                      "positives": int(ys.sum()), "unmatched": s_missing},
            "eval": {"file": "corpus/gold.jsonl", "sha256": sha256(gold_path), "hunks": len(yg),
                     "positives": int(yg.sum()), "unmatched": g_missing},
            "extra": a.extra and {"file": a.extra.name, "sha256": sha256(a.extra),
                                  "hunks": len(yt) - len(ys), "positives": int(yt.sum() - ys.sum()),
                                  "unmatched": e_missing},
            "history_drop": HISTORY_DROP, "seed": SEED, "lightgbm_params": LGBM,
            "versions": {"scikit-learn": sklearn.__version__, "lightgbm": lightgbm.__version__,
                         "numpy": np.__version__},
        },
    }
    a.out.write_text(json.dumps(report, indent=1, sort_keys=True) + "\n")
    table(f"gold ({len(yg)} hunks, {int(yg.sum())} skip)", gold)
    table("gold, history zeroed", gold_nohist)
    table(f"leave-one-repo-out on silver ({len(ys)} hunks)", loro_ev)
    print(f"\ngold AUROC gain trees - lr: {gain:+.4f}; shipping {model['type']} -> {a.out}")


if __name__ == "__main__":
    main()
