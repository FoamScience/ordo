#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scikit-learn", "lightgbm"]
# ///
"""Can hunch learn P(a finding is warranted)? Per labeled finding (silver
outside gold commits, plus $ORDO_CORPUS/judged.findings.jsonl from
finding-sample.py) it takes the hunk's engine features, the finding's level
and its rule, and scores a per-rule prior, logistic regression and LightGBM
leaving one repo out and, for contrast, in 5 folds grouped by commit; then
LightGBM trained on all of it against gold. Prints the scores and, per
threshold, how many findings P(warranted) < t would demote and how many of
those were warranted. Nothing ships from here: the gate is that some model,
leaving one repo out, ranks findings with AUROC above MIN_AUROC. A label
whose hunk's rationale the engine has since changed is dropped, as the
trainer does.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/finding-eval.py [--workers N]
"""
import argparse
import importlib.util
import json
import os
import sys
from collections import Counter, defaultdict
from pathlib import Path

import lightgbm as lgb
import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import average_precision_score, roc_auc_score
from sklearn.model_selection import GroupKFold
from sklearn.preprocessing import StandardScaler

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("hunch_train", ROOT / "scripts/hunch-train.py")
ht = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ht)

LEVELS = ("note", "warn", "verdict")
GRID = (0.1, 0.15, 0.2, 0.25, 0.3)
MIN_AUROC = 0.6
LGBM = dict(n_estimators=100, learning_rate=0.05, num_leaves=7, min_child_samples=10, verbose=-1,
            random_state=ht.SEED)


def read(p):
    return [json.loads(l) for l in Path(p).read_text().splitlines() if l.strip()]


def records(corpus):
    gold = read(ROOT / "corpus/gold.jsonl")
    gold_commits = {(r["repo"], r["sha"][:8]) for r in gold}
    silver = [r for r in read(corpus / "silver.jsonl")
              if (r["repo"], r["sha"][:8]) not in gold_commits and r["labels"].get("findings")]
    sample = {r["key"]: r for r in read(corpus / "finding-sample.jsonl")}
    labeled = read(corpus / "judged.findings.jsonl")
    if lost := [r["key"] for r in labeled if r["key"] not in sample]:
        sys.exit(f"{len(lost)} labeled keys are not in finding-sample.jsonl (first: {lost[0]}); was it redrawn?")
    extra = [{"key": r["key"], "repo": s["repo"], "sha": s["sha"], "diff": s["diff"],
              "labels": {"findings": r["findings"]}, "findings": s["findings"],
              "text": (s["rationale"], s.get("details", []))}
             for r in labeled for s in [sample[r["key"]]]]
    return [r for r in gold if r["labels"].get("findings")], silver + extra


def rows(x, kept, rules, level):
    """One row per labeled finding: its hunk's features, its level, its rule."""
    X, y, repos, names, commits = [], [], [], [], []
    for xi, r in zip(x, kept):
        for name, v in r["labels"]["findings"].items():
            if v is None:
                continue
            X.append([*xi, *(float(level.get(name) == l) for l in LEVELS), *(float(name == q) for q in rules)])
            y.append(v)
            repos.append(r["repo"])
            names.append(name)
            commits.append(r["key"].split(":")[1])
    return np.array(X), np.array(y, bool), np.array(repos), np.array(names), np.array(commits)


def prior(y, names, at):
    """Each rule's smoothed share of warranted findings (Beta(1,1))."""
    c = defaultdict(lambda: [1.0, 2.0])
    for v, n in zip(y, names):
        c[n][0] += v
        c[n][1] += 1
    return np.array([c[n][0] / c[n][1] for n in at])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    a = ap.parse_args()
    corpus = Path(os.environ.get("ORDO_CORPUS", Path.home() / ".cache/ordo-corpus")).expanduser()
    gold, train = records(corpus)
    level = {f["name"]: f["level"] for r in gold + train for f in r.get("findings", [])}
    rules = sorted({n for r in gold + train for n in r["labels"]["findings"]})
    xs, ks, *_ = ht.load(corpus, train, a.workers, label=None)
    xg, kg, *_ = ht.load(corpus, gold, a.workers, label=None)
    X, y, repos, names, commits = rows(xs, ks, rules, level)
    Xg, yg, _, names_g, _ = rows(xg, kg, rules, level)
    print(f"train {len(y)} findings on {len(ks)} hunks, {y.sum()} warranted; "
          f"gold {len(yg)} on {len(kg)} hunks, {yg.sum()} warranted")

    def lr(tr, te):
        sc = StandardScaler().fit(X[tr])
        return LogisticRegression(C=0.3, max_iter=3000).fit(sc.transform(X[tr]), y[tr]).predict_proba(
            sc.transform(X[te]))[:, 1]

    models = {"prior": lambda tr, te: prior(y[tr], names[tr], names[te]), "lr": lr,
              "lgbm": lambda tr, te: lgb.LGBMClassifier(**LGBM).fit(X[tr], y[tr]).predict_proba(X[te])[:, 1]}
    folds = {"loro": [(repos != r, repos == r) for r in np.unique(repos)],
             "commit5": [(np.isin(np.arange(len(y)), tr), np.isin(np.arange(len(y)), te))
                         for tr, te in GroupKFold(5).split(X, y, commits)]}
    loro_pass = False
    for fname, fs in folds.items():
        for mname, fn in models.items():
            s = np.zeros(len(y))
            for tr, te in fs:
                s[te] = fn(tr, te)
            auc = roc_auc_score(y, s)
            print(f"{fname:8s} {mname:6s} AUROC {auc:.3f}  AP(unwarranted) {average_precision_score(~y, -s):.3f} "
                  f"(base {(~y).mean():.3f})")
            if fname == "loro":
                loro_pass |= auc > MIN_AUROC
                for t in GRID:
                    f = s < t
                    print(f"           P < {t}: demotes {f.sum()}, {(~y[f]).sum()} unwarranted, "
                          f"{y[f].sum()}/{y.sum()} warranted ones wrongly")
    if 0 < yg.sum() < len(yg):
        g = lgb.LGBMClassifier(**LGBM).fit(X, y).predict_proba(Xg)[:, 1]
        print(f"gold ({len(yg)} findings, too few to decide on) lgbm AUROC {roc_auc_score(yg, g):.2f}, "
              f"prior AUROC {roc_auc_score(yg, prior(y, names, names_g)):.2f}")
    per = Counter((n, r, bool(v)) for n, r, v in zip(names, repos, y))
    print("per rule and repo (warranted/all):")
    for n, r in sorted({(n, r) for n, r, _ in per}):
        print(f"  {n:20s} {r:14s} {per[n, r, True]}/{per[n, r, True] + per[n, r, False]}")
    print(f"gate: {'PASS' if loro_pass else 'FAIL'} (a leave-one-repo-out AUROC above {MIN_AUROC})")


if __name__ == "__main__":
    main()
