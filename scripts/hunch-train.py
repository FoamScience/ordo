#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scikit-learn", "lightgbm"]
# ///
"""Train hunch's first head, P(should_skip), on the engine's per-hunk features:
logistic regression and LightGBM, trained on silver, scored on gold and by
leave-one-repo-out over silver. Training drops every hunk from a gold commit,
so gold stays held out. A label whose key the engine no longer emits (its hunk
boundaries drifted) takes the unclaimed hunk of the same file that overlaps it
by at least OVERLAP_MIN. The tree count is the smallest within TREE_TOL of the
best pooled leave-one-repo-out AUROC. The trees ship when they beat the
regression by MIN_GAIN AUROC on gold, or when, leaving one repo out, the
regression hides more hunks wrongly than the noise flag and the trees do not.
Writes src/hunch.json (trimmed model, feature schema, the fingerprint of the
engine's vectors for tests/fixtures/hunch, selection, provenance) for
build.rs, corpus/hunch-report.json (full metrics and the tree-count
trade-off), and prints the comparison against the engine's rule-based noise
flag at its own hide rate. tests/hunch-parity.json gets the gold rows with the
shipped model's own predictions, for the Rust inference to match.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py [--out FILE] [--report FILE] [--parity FILE] [--workers N] [--extra FILE]

--extra adds labeled hunks to training, one {key, should_skip} per line
(e.g. $ORDO_CORPUS/judged.should_skip.jsonl); evaluation stays on gold and silver.
Its rows must come from $ORDO_CORPUS/skip-sample.jsonl, whose stratum_n weights
each row back to its stratum's share of the sweep (see extra_records).

Build the engine first (cargo build --release); the corpus repos come from
scripts/corpus-fetch.sh.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from collections import defaultdict
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
OVERLAP_MIN = 0.8
TREE_MAX, TREE_STEP, TREE_TOL = 1000, 5, 0.005
LGBM = dict(n_estimators=TREE_MAX, learning_rate=0.05, num_leaves=15, min_child_samples=20,
            subsample=0.8, subsample_freq=1, colsample_bytree=0.8, random_state=SEED,
            deterministic=True, force_row_wise=True, n_jobs=1, verbose=-1)


def commit_features(corpus, repo, sha):
    """(key, old range, new range, feature vector) per hunk of one commit, and
    the schema."""
    d = str(corpus / repo)
    parent, inp = commit_input(d, sha)
    if not parent:
        return [], None
    out = engine_output(with_features(d, sha, inp))
    hunks = [(f"{repo}:{sha[:8]}:{f['path']}:{h['new_range'][0]}", h["old_range"], h["new_range"], h["features"])
             for f in out["files"] for h in f["hunks"] if h.get("features") is not None]
    return hunks, out.get("feature_schema")


def fingerprint():
    """FNV-1a of the engine's feature vectors for tests/fixtures/hunch, as
    `hunch::fingerprint` writes them: per fixture its file name on a line,
    then a line per hunk of its values to six decimals. tests/hunch.rs
    recomputes it, so a column whose values move fails CI until retrained."""
    text = ""
    for p in sorted((ROOT / "tests/fixtures/hunch").glob("*.json")):
        out = engine_output(json.loads(p.read_text()))
        text += p.name + "\n" + "".join(",".join(f"{v:.6f}" for v in h["features"]) + "\n"
                                         for f in out["files"] for h in f["hunks"])
    h = 0xCBF29CE484222325
    for b in text.encode():
        h = ((h ^ b) * 0x100000001B3) % 2**64
    return f"{h:016x}"


def changed(diff):
    """The old and new line ranges a labeled hunk changes, from its header,
    which gold-sample.py writes as ordo's own ranges; inclusive, empty when
    start > end."""
    m = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", diff)
    o0, oc, n0, nc = (int(g) if g is not None else 1 for g in m.groups())
    return (o0, o0 + oc - 1), (n0, n0 + nc - 1)


def overlap(a, b):
    """Jaccard of two hunks' changed lines, old and new side together."""
    size = lambda r: max(0, r[1] - r[0] + 1)
    inter = sum(size((max(x[0], y[0]), min(x[1], y[1]))) for x, y in zip(a, b))
    union = sum(size(x) + size(y) for x, y in zip(a, b)) - inter
    return inter / union if union else 0.0


def load(corpus, records, workers):
    """Features and labels for `records`, each matched to an engine hunk by
    key, else by overlap; the counts of both misses and overlap matches."""
    commits = sorted({(r["repo"], r["sha"]) for r in records})
    vecs, per_file, schemas = {}, defaultdict(list), set()
    with ThreadPoolExecutor(workers) as ex:
        for hunks, s in ex.map(lambda c: commit_features(corpus, *c), commits):
            for key, old, new, v in hunks:
                vecs[key] = v
                per_file[key.rsplit(":", 1)[0]].append((key, (tuple(old), tuple(new))))
            if s:
                schemas.add(json.dumps(s, sort_keys=True))
    if len(schemas) != 1:
        sys.exit(f"expected one feature schema, got {len(schemas)}")
    claimed = {r["key"] for r in records if r["key"] in vecs}
    keys, missing, by_overlap = [], [], 0
    for r in records:
        key = r["key"] if r["key"] in vecs else None
        if key is None and r.get("diff"):
            span = changed(r["diff"])
            best = max(((overlap(span, rng), k) for k, rng in per_file[r["key"].rsplit(":", 1)[0]]
                        if k not in claimed), default=(0.0, None))
            if best[0] >= OVERLAP_MIN:
                key = best[1]
                claimed.add(key)
                by_overlap += 1
        if key is None:
            missing.append(r["key"])
        keys.append(key)
    if missing:
        print(f"{len(missing)} labeled hunks not found in engine output, dropped "
              f"(first: {missing[0]})", file=sys.stderr)
    kept = [(r, k) for r, k in zip(records, keys) if k is not None]
    unlabeled = [r["key"] for r, _ in kept if not isinstance(r["labels"].get("should_skip"), bool)]
    if unlabeled:
        sys.exit(f"{len(unlabeled)} labeled hunks have no should_skip answer (first: {unlabeled[0]})")
    x = np.array([vecs[k] for _, k in kept], dtype=float)
    y = np.array([r["labels"]["should_skip"] for r, _ in kept])
    repos = np.array([r["repo"] for r, _ in kept])
    w = np.array([r.get("weight", 1.0) for r, _ in kept])
    return x, y, repos, w, json.loads(schemas.pop()), {"unmatched": len(missing), "matched_by_overlap": by_overlap}


def extra_records(path, sample_path):
    """{key, should_skip} lines as silver-shaped records, with the sampled
    diff (for overlap matching) and a weight. The sample fills each repo x
    noise stratum equally, so a row stands for stratum_n / (rows drawn from its
    stratum) hunks of the sweep; main() rescales those to mean 1 so the set
    keeps the pull of its row count against silver (weight 1) and only its
    internal proportions move back to the sweep's. The key's sha is the short
    one, which git and the engine's keys both accept."""
    sample = {r["key"]: r for l in sample_path.read_text().splitlines() if l.strip() for r in [json.loads(l)]}
    drawn = defaultdict(int)
    for r in sample.values():
        drawn[r["repo"], r["noise"]] += 1
    for l in path.read_text().splitlines():
        if l.strip():
            r = json.loads(l)
            s = sample.get(r["key"])
            if s is None:
                sys.exit(f"{r['key']} is not in {sample_path}")
            repo, sha, _ = r["key"].split(":", 2)
            yield {"key": r["key"], "repo": repo, "sha": sha, "diff": s["diff"],
                   "labels": {"should_skip": r["should_skip"]},
                   "weight": s["stratum_n"] / drawn[s["repo"], s["noise"]]}


def drop_history(x, cols, rng):
    x = x.copy()
    x[np.ix_(rng.random(len(x)) < HISTORY_DROP, cols)] = 0
    return x


def fit(kind, x, y, w):
    if kind == "lr":
        m = make_pipeline(StandardScaler(), LogisticRegression(max_iter=5000, random_state=SEED))
        return m.fit(x, y, standardscaler__sample_weight=w, logisticregression__sample_weight=w)
    return lightgbm.LGBMClassifier(**LGBM).fit(x, y, sample_weight=w)


def f32(v):
    """v rounded to the nearest f32, printed as the shortest decimal that
    parses back to it."""
    return float(str(np.float32(v)))


def flat_tree(root):
    """A LightGBM tree as a node list, root first: splits send `x[feature] <=
    threshold` left; internal_value (the split's output before it) gives
    path contributions."""
    nodes = []

    def visit(n):
        i = len(nodes)
        nodes.append(None)
        if "leaf_value" in n:
            nodes[i] = {"leaf_value": f32(n["leaf_value"])}
            return i
        if n["decision_type"] != "<=" or n["missing_type"] != "None":
            sys.exit(f"unsupported split {n['decision_type']} / missing {n['missing_type']}")
        left, right = visit(n["left_child"]), visit(n["right_child"])
        nodes[i] = {"feature": n["split_feature"], "threshold": f32(n["threshold"]),
                    "left": left, "right": right, "internal_value": f32(n["internal_value"])}
        return i

    visit(root)
    return nodes


def trimmed_trees(booster, n):
    return [flat_tree(t["tree_structure"]) for t in booster.dump_model(num_iteration=n)["tree_info"]]


def trees_proba(trees, x):
    """The trimmed model's P(skip), walked in f32 as build.rs will; checks
    the export against LightGBM's own prediction."""
    def leaf(t, row):
        n = t[0]
        while "leaf_value" not in n:
            n = t[n["left"] if row[n["feature"]] <= n["threshold"] else n["right"]]
        return n["leaf_value"]
    raw = np.array([sum(leaf(t, row) for t in trees) for row in x.astype(np.float32)])
    return 1 / (1 + np.exp(-raw))


def dump(doc):
    """JSON with one tree per line, so the file diffs by tree."""
    trees = doc["model"].get("trees")
    if trees is None:
        return json.dumps(doc, indent=1, sort_keys=True) + "\n"
    text = json.dumps({**doc, "model": {**doc["model"], "trees": "@TREES@"}}, indent=1, sort_keys=True)
    rows = ",\n".join("   " + json.dumps(t, separators=(",", ":"), sort_keys=True) for t in trees)
    return text.replace('"@TREES@"', "[\n" + rows + "\n  ]") + "\n"


def parity(x, p):
    """The gold rows and the shipped model's P(skip) for each, as the trained
    library computes it: tests/hunch.rs checks the Rust inference against it."""
    num = lambda v: int(v) if float(v).is_integer() else float(v)
    rows = ",\n".join(" " + json.dumps({"x": [num(v) for v in row], "p": float(q)}, separators=(",", ":"))
                       for row, q in zip(x, p))
    return "[\n" + rows + "\n]\n"


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
    ap.add_argument("--report", type=Path, default=ROOT / "corpus/hunch-report.json")
    ap.add_argument("--parity", type=Path, default=ROOT / "tests/hunch-parity.json")
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    ap.add_argument("--extra", type=Path, help="more training labels, {key, should_skip} per line")
    a = ap.parse_args()
    fp = fingerprint()
    corpus = Path(os.environ.get("ORDO_CORPUS", "~/.cache/ordo-corpus")).expanduser()
    gold_path, silver_path = ROOT / "corpus/gold.jsonl", corpus / "silver.jsonl"
    read = lambda p: [json.loads(l) for l in p.read_text().splitlines() if l.strip()]
    gold_records = read(gold_path)
    gold_commits = {(r["repo"], r["sha"][:8]) for r in gold_records}
    held_out = lambda records: [r for r in records if (r["repo"], r["sha"][:8]) not in gold_commits]
    silver_records = read(silver_path)
    silver_kept = held_out(silver_records)
    xs, ys, rs, ws, schema, s_match = load(corpus, silver_kept, a.workers)
    xg, yg, rg, _, gschema, g_match = load(corpus, gold_records, a.workers)
    if gschema != schema:
        sys.exit("silver and gold feature schemas differ")
    names = schema["names"]
    noise, hist = names.index("noise"), [names.index(c) for c in HISTORY]
    xt, yt, rt, wt, e_match, e_gold, e_weights = xs, ys, rs, ws, None, 0, None
    if a.extra:
        extra = list(extra_records(a.extra, corpus / "skip-sample.jsonl"))
        extra_kept = held_out(extra)
        e_gold = len(extra) - len(extra_kept)
        xe, ye, re_, we, eschema, e_match = load(corpus, extra_kept, a.workers)
        if eschema != schema:
            sys.exit("silver and extra feature schemas differ")
        we = we / we.mean()
        e_weights = {"min": round(we.min(), 4), "max": round(we.max(), 4),
                     "skip_rate_raw": round(ye.mean(), 4), "skip_rate_weighted": round(np.average(ye, weights=we), 4)}
        xt, yt, rt, wt = np.vstack([xs, xe]), np.concatenate([ys, ye]), np.concatenate([rs, re_]), np.concatenate([ws, we])
    xt = drop_history(xt, hist, np.random.default_rng(SEED))

    # pooled leave-one-repo-out AUROC per tree count picks the trees' size
    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    loro, curve = {"lr": np.zeros(len(ys))}, np.zeros((len(grid), len(ys)))
    for r in sorted(set(rs)):
        held, train = rs == r, rt != r
        loro["lr"][held] = fit("lr", xt[train], yt[train], wt[train]).predict_proba(xs[held])[:, 1]
        m = fit("trees", xt[train], yt[train], wt[train])
        for i, n in enumerate(grid):
            curve[i, held] = m.predict_proba(xs[held], num_iteration=n)[:, 1]
    loro_auroc = [roc_auc_score(ys, c) for c in curve]
    best = int(np.argmax(loro_auroc))
    ship = next(i for i, v in enumerate(loro_auroc) if v >= loro_auroc[best] - TREE_TOL)
    n_trees = grid[ship]
    loro["trees"] = curve[ship]
    loro_ev = evaluate(loro, xs[:, noise], ys, rs)

    models = {k: fit(k, xt, yt, wt) for k in ("lr", "trees")}
    proba = lambda k, x, n=n_trees: models[k].predict_proba(x, **({"num_iteration": n} if k == "trees" else {}))[:, 1]
    xg_nohist = xg.copy()
    xg_nohist[:, hist] = 0
    gold = evaluate({k: proba(k, xg) for k in models}, xg[:, noise], yg, rg)
    gold_nohist = evaluate({k: proba(k, xg_nohist) for k in models}, xg[:, noise], yg, rg)
    booster = models["trees"].booster_
    tradeoff = []
    for i in sorted({ship, best, grid.index(200), len(grid) - 1}):
        t = trimmed_trees(booster, grid[i])
        tradeoff.append({"trees": grid[i], "nodes": sum(map(len, t)),
                         "bytes": len(dump({"model": {"trees": t}}).encode()),
                         "loro_auroc": round(loro_auroc[i], 4),
                         "gold_auroc": round(roc_auc_score(yg, proba("trees", xg, grid[i])), 4)})
    loro_per_repo = {r: {k: round(roc_auc_score(ys[rs == r], s[rs == r]), 4)
                         for k, s in [("noise", xs[:, noise]), *loro.items()]}
                     for r in sorted(set(rs)) if len(set(ys[rs == r])) == 2}

    gain = gold["trees"]["auroc"] - gold["lr"]["auroc"]
    wrong = {k: m["at_noise_hide_rate"]["all"]["wrong_hides"] for k, m in loro_ev.items()}
    fired = ("gold_auroc_gain" if gain >= MIN_GAIN else
             "loro_wrong_hides" if wrong["lr"] > wrong["noise"] >= wrong["trees"] else None)
    if fired is None:
        scaler, lr = models["lr"].named_steps.values()
        model = {"type": "logistic", "mean": [f32(v) for v in scaler.mean_],
                 "scale": [f32(v) for v in scaler.scale_], "coef": [f32(v) for v in lr.coef_[0]],
                 "intercept": f32(lr.intercept_[0])}
    else:
        trees = trimmed_trees(booster, n_trees)
        export_err = float(np.abs(trees_proba(trees, np.vstack([xs, xg])) -
                                  proba("trees", np.vstack([xs, xg]))).max())
        if export_err > 1e-4:
            sys.exit(f"trimmed trees disagree with LightGBM by {export_err:.2g}")
        print(f"trimmed trees vs LightGBM over silver and gold: max |dP| {export_err:.2g}")
        model = {"type": "lightgbm", "objective": "binary", "link": "sigmoid",
                 "score": "sum of the trees' leaf_value", "split": "x[feature] <= threshold goes left",
                 "trees": trees}

    selection = {"rule": f"lightgbm if gold AUROC gain over logistic >= {MIN_GAIN}, or if leaving "
                         "one repo out logistic makes more wrong hides than the noise flag at its "
                         "hide rate and lightgbm does not",
                 "gold_auroc_gain": round(gain, 4), "loro_wrong_hides": wrong, "fired": fired,
                 "chosen": model["type"]}
    provenance = {
            "label": "labels.should_skip",
            "train": {"file": "silver.jsonl", "sha256": sha256(silver_path), "hunks": len(ys),
                      "positives": int(ys.sum()), "dropped_in_gold_commits": len(silver_records) - len(silver_kept),
                      **s_match},
            "eval": {"file": "corpus/gold.jsonl", "sha256": sha256(gold_path), "hunks": len(yg),
                     "positives": int(yg.sum()), **g_match},
            "extra": a.extra and {"file": a.extra.name, "sha256": sha256(a.extra),
                                  "hunks": len(yt) - len(ys), "positives": int(yt.sum() - ys.sum()),
                                  "dropped_in_gold_commits": e_gold, **e_match},
        "extra_weights": e_weights,
        "overlap_min": OVERLAP_MIN,
        "history_drop": HISTORY_DROP, "seed": SEED, "lightgbm_params": {**LGBM, "n_estimators": n_trees},
        "tree_count": {"rule": f"smallest count within {TREE_TOL} of the best pooled leave-one-repo-out "
                               f"AUROC, every {TREE_STEP} up to {TREE_MAX}",
                       "best": grid[best], "best_loro_auroc": round(loro_auroc[best], 4),
                       "shipped": n_trees, "shipped_loro_auroc": round(loro_auroc[ship], 4)},
        "versions": {"scikit-learn": sklearn.__version__, "lightgbm": lightgbm.__version__,
                     "numpy": np.__version__},
    }
    a.out.write_text(dump({"head": "should_skip", "model": model, "feature_schema": schema,
                           "fingerprint": fp, "selection": selection, "provenance": provenance}))
    a.parity.write_text(parity(xg, proba("lr" if fired is None else "trees", xg)))
    a.report.write_text(json.dumps({
        "head": "should_skip", "selection": selection, "provenance": provenance,
        "metrics": {"gold": gold, "gold_without_history": gold_nohist,
                    "leave_one_repo_out": loro_ev, "leave_one_repo_out_auroc_per_repo": loro_per_repo},
        "tree_count_tradeoff": tradeoff,
        "tree_count_curve": {"trees": grid, "loro_auroc": [round(v, 4) for v in loro_auroc]},
    }, indent=1, sort_keys=True) + "\n")
    table(f"gold ({len(yg)} hunks, {int(yg.sum())} skip)", gold)
    table("gold, history zeroed", gold_nohist)
    table(f"leave-one-repo-out on silver ({len(ys)} hunks)", loro_ev)
    print(f"\ngold AUROC gain trees - lr: {gain:+.4f}; LORO wrong hides {wrong}; "
          f"rule fired: {fired}; shipping {model['type']} -> {a.out}")


if __name__ == "__main__":
    main()
