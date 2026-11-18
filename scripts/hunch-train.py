#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scikit-learn", "lightgbm"]
# ///
"""Train hunch's heads on the engine's per-hunk features: P(should_skip) (see
`train_skip`) and P(faithful rationale) (see `train_faithful`), each
choosing between logistic regression and LightGBM, trained on silver,
scored on gold and by leave-one-repo-out. The skip head: Training drops every hunk from a gold commit,
so gold stays held out. A label whose key the engine no longer emits (its hunk
boundaries drifted) takes the unclaimed hunk of the same file that overlaps it
by at least OVERLAP_MIN. The tree count is the smallest within TREE_TOL of the
best pooled leave-one-repo-out AUROC. The trees ship when they beat the
regression by MIN_GAIN AUROC on gold, or when, leaving one repo out, the
regression hides more hunks wrongly than the noise flag and the trees do not.
The promotion gate tests the policy `--hunch` applies to the noise flag (see
`promotion`). Writes src/hunch.json (the feature schema, the fingerprint of
the engine's vectors for tests/fixtures/hunch, and per head its trimmed
model, selection, provenance, and its policy's thresholds and gate verdict)
for build.rs, corpus/hunch-report.json (full metrics, the gates and the
tree-count trade-off), and prints the comparison against the engine's
rule-based noise flag and the faithful gate. tests/hunch-parity.json gets the
gold rows with each shipped head's own predictions, for the Rust inference
to match.

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py [--out FILE] [--report FILE] [--parity FILE] [--workers N] [--extra FILE] [--extra-faithful FILE]

--extra adds labeled hunks to the skip head's training, one {key,
should_skip} per line (e.g. $ORDO_CORPUS/judged.should_skip.jsonl);
--extra-faithful does for the faithful head, one {key, faithful, rationale,
details} per line (e.g. $ORDO_CORPUS/judged.faithful.jsonl), dropping a row
whose rationale the engine has since changed. Skip evaluation stays on gold
and silver. Rows of both must come from $ORDO_CORPUS/skip-sample.jsonl, whose
stratum_n weights each row back to its stratum's share of the sweep (see
extra_records).

Build the engine first (cargo build --release); the corpus repos come from
scripts/corpus-fetch.sh.
"""
import argparse
import hashlib
import json
import os
import re
import sys
from collections import defaultdict, namedtuple
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
POLICY_HIDE = 0.9
UNHIDE_GRID = [round(0.05 * i, 2) for i in range(1, 18)]
SKIP_DROPS = ("rationale_",)
DOUBT_GRID = [round(0.05 * i, 2) for i in range(4, 20)]
DOUBT_PRECISION, DOUBT_MIN_FLAGS, DOUBT_RECALL = 0.5, 5, 0.2
GOLD_RECALL, GOLD_FALSE_FLAGS = 0.25, 0.03
LGBM = dict(n_estimators=TREE_MAX, learning_rate=0.05, num_leaves=15, min_child_samples=20,
            subsample=0.8, subsample_freq=1, colsample_bytree=0.8, random_state=SEED,
            deterministic=True, force_row_wise=True, n_jobs=1, verbose=-1)


def commit_features(corpus, repo, sha):
    """(key, old range, new range, feature vector, (rationale, details)) per
    hunk of one commit, and the schema."""
    d = str(corpus / repo)
    parent, inp = commit_input(d, sha)
    if not parent:
        return [], None
    out = engine_output(with_features(d, sha, inp))
    hunks = [(f"{repo}:{sha[:8]}:{f['path']}:{h['new_range'][0]}", h["old_range"], h["new_range"], h["features"],
              (h["rationale"], h.get("details", [])))
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


def load(corpus, records, workers, label="should_skip"):
    """Features and `label` for `records`, each matched to an engine hunk by
    key, else by overlap; the counts of both misses and overlap matches. With
    `label` None, every head's labels: the kept records come back in place of
    the label array, for the caller to read. A record that carries the
    rationale it was labeled on (`text`) is dropped as stale when the engine
    no longer writes that rationale for its hunk."""
    commits = sorted({(r["repo"], r["sha"]) for r in records})
    vecs, texts, per_file, schemas = {}, {}, defaultdict(list), set()
    with ThreadPoolExecutor(workers) as ex:
        for hunks, s in ex.map(lambda c: commit_features(corpus, *c), commits):
            for key, old, new, v, text in hunks:
                vecs[key] = v
                texts[key] = text
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
    is_stale = lambda r, k: "text" in r and tuple(r["text"]) != texts[k]
    stale = [r["key"] for r, k in kept if is_stale(r, k)]
    if stale:
        print(f"{len(stale)} labeled hunks whose rationale the engine has since changed, dropped "
              f"(first: {stale[0]})", file=sys.stderr)
        kept = [(r, k) for r, k in kept if not is_stale(r, k)]
    if label is not None:
        unlabeled = [r["key"] for r, _ in kept if not isinstance(r["labels"].get(label), bool)]
        if unlabeled:
            sys.exit(f"{len(unlabeled)} labeled hunks have no {label} answer (first: {unlabeled[0]})")
    x = np.array([vecs[k] for _, k in kept], dtype=float)
    y = [r for r, _ in kept] if label is None else np.array([r["labels"][label] for r, _ in kept])
    repos = np.array([r["repo"] for r, _ in kept])
    w = np.array([r.get("weight", 1.0) for r, _ in kept])
    return x, y, repos, w, json.loads(schemas.pop()), {"unmatched": len(missing), "matched_by_overlap": by_overlap,
                                                       **({"stale_rationale": len(stale)} if stale else {})}


def extra_records(path, sample_path, label="should_skip"):
    """{key, <label>} lines as silver-shaped records, with the sampled diff
    (for overlap matching) and a weight. A line that records the rationale
    and details it was labeled on keeps them as `text`, so `load` can drop it
    once the engine writes another. The sample fills each repo x
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
                   "labels": {label: r[label]},
                   "weight": s["stratum_n"] / drawn[s["repo"], s["noise"]],
                   **({"text": (r["rationale"], r.get("details", []))} if "rationale" in r else {})}


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
    heads, trees = {}, {}
    for name, head in doc["heads"].items():
        if "trees" in head["model"]:
            trees[name] = head["model"]["trees"]
            head = {**head, "model": {**head["model"], "trees": f"@TREES:{name}@"}}
        heads[name] = head
    text = json.dumps({**doc, "heads": heads}, indent=1, sort_keys=True)
    for name, ts in trees.items():
        rows = ",\n".join("     " + json.dumps(t, separators=(",", ":"), sort_keys=True) for t in ts)
        text = text.replace(f'"@TREES:{name}@"', "[\n" + rows + "\n    ]")
    return text + "\n"


def tree_bytes(trees):
    return len(dump({"heads": {"h": {"model": {"trees": trees}}}}).encode())


def parity(x, p):
    """The gold rows and each shipped head's probability for each, as the
    trained library computes it: tests/hunch.rs checks the Rust inference
    against it."""
    num = lambda v: int(v) if float(v).is_integer() else float(v)
    rows = ",\n".join(" " + json.dumps({"x": [num(v) for v in row], "p": {h: float(q[i]) for h, q in p.items()}},
                                        separators=(",", ":"))
                       for i, row in enumerate(x))
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


def at_repo_hide_rate(score, base, y, repos):
    """at_hide_rate within each repo on its own: hunch hides as many of a
    repo's hunks as the noise flag does there. With the hide count fixed,
    wrong hides and missed skips move together (wrong - missed = hidden -
    skips), so a repo where hunch makes more wrong hides is one it loses."""
    return {r: at_hide_rate(score[repos == r], base[repos == r], y[repos == r], repos[repos == r])["all"]
            for r in sorted(set(repos))}


def policy(score, base, y, repos, unhide):
    """The promoted policy: start from the noise flag, show a hidden hunk
    whose P(skip) is under `unhide`, hide a shown one at P(skip) >=
    POLICY_HIDE. With no score, the noise flag alone."""
    base = base.astype(bool)
    hidden = base if score is None else np.where(base, score >= unhide, score >= POLICY_HIDE)
    counts = lambda m: {"hidden": int(hidden[m].sum()), "unhidden": int((base & ~hidden)[m].sum()),
                        "newly_hidden": int((~base & hidden)[m].sum()),
                        "wrong_hides": int((hidden & ~y)[m].sum()), "missed_skips": int((~hidden & y)[m].sum())}
    return {"all": counts(np.ones(len(y), dtype=bool)),
            "per_repo": {r: counts(repos == r) for r in sorted(set(repos))}}


def pick_unhide(score, base, y, repos):
    """The un-hide threshold with the fewest wrong hides plus missed skips
    leaving one repo out, the lowest on a tie (it un-hides least)."""
    curve = {t: policy(score, base, y, repos, t)["all"] for t in UNHIDE_GRID}
    best = min(UNHIDE_GRID, key=lambda t: (curve[t]["wrong_hides"] + curve[t]["missed_skips"], t))
    return best, curve


def promotion(gold, loro_score, gold_score, xs_noise, xg_noise, ys, yg, rs, rg, shipped):
    """The promotion gate, on the policy that ships: start from the noise
    flag, un-hide a hidden hunk under the `unhide` threshold (picked leaving
    one repo out, never on gold), hide a shown one at POLICY_HIDE. It passes
    when, leaving one repo out and on gold, no repo gets more wrong hides than
    under the noise flag and both make fewer overall. The ranking at the noise
    flag's hide rate is reported beside it, for information only."""
    unhide, curve = pick_unhide(loro_score, xs_noise, ys, rs)
    runs = {"gold": {"noise": policy(None, xg_noise, yg, rg, None), "hunch": policy(gold_score, xg_noise, yg, rg, unhide)},
            "loro": {"noise": policy(None, xs_noise, ys, rs, None), "hunch": policy(loro_score, xs_noise, ys, rs, unhide)}}
    verdict = {}
    for where, run in runs.items():
        n, h = run["noise"], run["hunch"]
        verdict[where] = {"raised_in": sorted(r for r in n["per_repo"]
                                              if h["per_repo"][r]["wrong_hides"] > n["per_repo"][r]["wrong_hides"]),
                          "lowered_overall": h["all"]["wrong_hides"] < n["all"]["wrong_hides"]}
    passes = all(v["lowered_overall"] and not v["raised_in"] for v in verdict.values())
    g_rule, g_hunch = (gold[k]["at_noise_hide_rate"]["all"] for k in ("noise", shipped))
    rule_repo = at_repo_hide_rate(xs_noise, xs_noise, ys, rs)
    hunch_repo = at_repo_hide_rate(loro_score, xs_noise, ys, rs)
    return {
        "model": shipped,
        "gate": {"rule": "the policy raises wrong hides in no repo, leaving one repo out and on gold, "
                         "and lowers them overall in both",
                 **verdict, "passes": passes},
        "policy": {"rule": f"noise flag; un-hide at P(skip) < unhide; hide a shown hunk at P(skip) >= {POLICY_HIDE}",
                   "unhide": unhide, "hide": POLICY_HIDE,
                   "picked_on": "leave-one-repo-out on silver, fewest wrong hides + missed skips",
                   "unhide_curve_loro": {str(t): c for t, c in curve.items()}, **runs},
        "ranking": {"note": "informational, not the gate: hunch's ranking at the noise flag's hide rate",
                    "gold_at_noise_hide_rate": {"noise": g_rule, "hunch": g_hunch},
                    "loro_at_repo_hide_rate": {"noise": rule_repo, "hunch": hunch_repo,
                                               "more_wrong_hides": sorted(r for r in rule_repo
                                                                          if hunch_repo[r]["wrong_hides"] > rule_repo[r]["wrong_hides"])}},
    }


def promotion_table(p):
    g, q = p["gate"], p["policy"]
    print(f"\npromotion gate ({p['model']}, the policy: un-hide below {q['unhide']}, hide at >= {q['hide']}): "
          f"{'PASS' if g['passes'] else 'FAIL'}")
    for where in ("gold", "loro"):
        n, h = q[where]["noise"], q[where]["hunch"]
        print(f"{where}: noise {n['all']['wrong_hides']}/{n['all']['missed_skips']} -> policy "
              f"{h['all']['wrong_hides']}/{h['all']['missed_skips']} (un-hid {h['all']['unhidden']}, newly hid "
              f"{h['all']['newly_hidden']}); more wrong hides in: {', '.join(g[where]['raised_in']) or 'none'}")
        for r, c in n["per_repo"].items():
            d = h["per_repo"][r]
            if (c["wrong_hides"], c["missed_skips"]) != (d["wrong_hides"], d["missed_skips"]):
                print(f"  {r:14} {c['wrong_hides']:>3}/{c['missed_skips']:<3} -> {d['wrong_hides']:>3}/{d['missed_skips']:<3}")
    k = p["ranking"]
    a = k["gold_at_noise_hide_rate"]
    print(f"\nranking (informational), gold at the noise hide rate, wrong/missed: noise "
          f"{a['noise']['wrong_hides']}/{a['noise']['missed_skips']}, hunch {a['hunch']['wrong_hides']}/{a['hunch']['missed_skips']}")
    l = k["loro_at_repo_hide_rate"]
    print(f"{'repo':14} {'hidden':>6} {'noise':>13} {'hunch':>13}   (LORO at the repo's hide rate, wrong/missed)")
    for r, c in l["noise"].items():
        h = l["hunch"][r]
        print(f"{r:14} {c['hidden']:6} {c['wrong_hides']:>6}/{c['missed_skips']:<6} {h['wrong_hides']:>6}/{h['missed_skips']:<6}"
              + ("  more wrong" if r in l["more_wrong_hides"] else ""))


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


Data = namedtuple("Data", "corpus schema xs s_recs rs ws xg g_recs rg held_out provenance workers")


def labels(records, label):
    missing = [r["key"] for r in records if not isinstance(r["labels"].get(label), bool)]
    if missing:
        sys.exit(f"{len(missing)} labeled hunks have no {label} answer (first: {missing[0]})")
    return np.array([r["labels"][label] for r in records])


def train_skip(c, extra):
    """The should_skip head: P(a reviewer can skip the hunk), its model chosen
    as `selection` says and its noise-flag policy gated by `promotion`.
    `extra` adds {key, should_skip} rows from the skip sample to training.
    It reads every column but the faithful head's own (SKIP_DROPS)."""
    width = len(c.schema["names"])
    cols = [i for i, n in enumerate(c.schema["names"]) if not n.startswith(SKIP_DROPS)]
    names = [c.schema["names"][i] for i in cols]
    xs, rs, ws, xg, rg = c.xs[:, cols], c.rs, c.ws, c.xg[:, cols], c.rg
    ys, yg = labels(c.s_recs, "should_skip"), labels(c.g_recs, "should_skip")
    noise, hist = names.index("noise"), [names.index(n) for n in HISTORY]
    xt, yt, rt, wt, e_match, e_gold, e_weights = xs, ys, rs, ws, None, 0, None
    if extra:
        rows = list(extra_records(extra, c.corpus / "skip-sample.jsonl"))
        extra_kept = c.held_out(rows)
        e_gold = len(rows) - len(extra_kept)
        xe, ye, re_, we, eschema, e_match = load(c.corpus, extra_kept, c.workers)
        if eschema != c.schema:
            sys.exit("silver and extra feature schemas differ")
        xe = xe[:, cols]
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
                         "bytes": tree_bytes(t),
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
        "train": {**c.provenance["train"], "positives": int(ys.sum())},
        "eval": {**c.provenance["eval"], "positives": int(yg.sum())},
        "extra": extra and {"file": extra.name, "sha256": sha256(extra),
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
    shipped = "lr" if fired is None else "trees"
    promo = promotion(gold, loro[shipped], proba(shipped, xg), xs[:, noise], xg[:, noise], ys, yg, rs, rg, shipped)
    shipped_policy = {"passes": promo["gate"]["passes"], "unhide": promo["policy"]["unhide"],
                      "hide": promo["policy"]["hide"], "rule": promo["policy"]["rule"]}
    table(f"gold ({len(yg)} hunks, {int(yg.sum())} skip)", gold)
    table("gold, history zeroed", gold_nohist)
    table(f"leave-one-repo-out on silver ({len(ys)} hunks)", loro_ev)
    print(f"\ngold AUROC gain trees - lr: {gain:+.4f}; LORO wrong hides {wrong}; "
          f"rule fired: {fired}; shipping {model['type']}")
    promotion_table(promo)
    head = {"model": widen(model, cols, width), "columns": names, "selection": selection,
            "provenance": provenance, "policy": shipped_policy}
    report = {"selection": selection, "provenance": provenance, "promotion": promo,
              "metrics": {"gold": gold, "gold_without_history": gold_nohist,
                          "leave_one_repo_out": loro_ev, "leave_one_repo_out_auroc_per_repo": loro_per_repo},
              "tree_count_tradeoff": tradeoff,
              "tree_count_curve": {"trees": grid, "loro_auroc": [round(v, 4) for v in loro_auroc]}}
    return head, report, proba(shipped, xg)


def widen(model, cols, width):
    """A model trained on columns `cols` of the schema, as one over all
    `width` of them: tree splits renumbered, a regression's unused columns
    weighted 0."""
    if model["type"] == "lightgbm":
        for t in model["trees"]:
            for n in t:
                if "feature" in n:
                    n["feature"] = cols[n["feature"]]
        return model
    full = {"mean": [0.0] * width, "scale": [1.0] * width, "coef": [0.0] * width}
    for k, v in full.items():
        for i, c in enumerate(cols):
            v[c] = model[k][i]
    return {**model, **full}


def doubt_policy(score, y, w, t):
    """Flag a rationale when P(faithful) < t: flags, how many were unfaithful,
    precision and recall, each weighted by `w`, and the share of faithful
    rationales flagged."""
    flag, bad = score < t, ~y
    tp = float(w[flag & bad].sum())
    flags = float(w[flag].sum())
    return {"flags": int(flag.sum()), "unfaithful_flagged": int((flag & bad).sum()),
            "unfaithful": int(bad.sum()),
            "precision": round(tp / flags, 4) if flags else None,
            "recall": round(tp / float(w[bad].sum()), 4) if bad.any() else None,
            "false_flag_rate": round(float(w[flag & ~bad].sum()) / float(w[~bad].sum()), 4)}


def doubt_gate(loro_score, y, w, gold_score, yg):
    """The threshold under which `--hunch` marks a rationale possibly off,
    picked leaving one repo out (never on gold): the highest on DOUBT_GRID
    whose flags are right at least DOUBT_PRECISION of the time, over at
    least DOUBT_MIN_FLAGS flags. It passes when there it also finds
    DOUBT_RECALL of the unfaithful rationales, and on gold finds GOLD_RECALL
    of them while flagging at most GOLD_FALSE_FLAGS of the faithful ones."""
    curve = {t: doubt_policy(loro_score, y, w, t) for t in DOUBT_GRID}
    ok = [t for t in DOUBT_GRID if curve[t]["flags"] >= DOUBT_MIN_FLAGS
          and curve[t]["precision"] >= DOUBT_PRECISION]
    t = max(ok, default=None)
    gold_curve = {t: doubt_policy(gold_score, yg, np.ones(len(yg)), t) for t in DOUBT_GRID}
    verdict = {"loro": None, "gold": None, "passes": False}
    if t is not None:
        lo, go = curve[t], gold_curve[t]
        verdict = {"loro": lo, "gold": go,
                   "passes": lo["recall"] >= DOUBT_RECALL and (go["recall"] or 0) >= GOLD_RECALL
                   and go["false_flag_rate"] <= GOLD_FALSE_FLAGS}
    return {"rule": f"doubt below the highest P(faithful) threshold whose leave-one-repo-out flags are "
                    f"unfaithful >= {DOUBT_PRECISION} of the time (>= {DOUBT_MIN_FLAGS} flags); passes "
                    f"when there it finds >= {DOUBT_RECALL} of the unfaithful rationales and on gold "
                    f">= {GOLD_RECALL} while flagging <= {GOLD_FALSE_FLAGS} of the faithful",
            "doubt": t, **verdict,
            "curve_loro": {str(k): v for k, v in curve.items()},
            "curve_gold": {str(k): v for k, v in gold_curve.items()}}


def train_faithful(c, extra):
    """The faithful head: P(the rationale states nothing false or misleading),
    trained on silver and the weighted `extra` rows ({key, faithful,
    rationale, details}; a row whose rationale the engine has since changed
    is dropped), evaluated leaving one repo out over both and on gold. Its
    doubt threshold and gate are `doubt_gate`'s."""
    names = c.schema["names"]
    hist = [names.index(n) for n in HISTORY]
    ys, yg = labels(c.s_recs, "faithful"), labels(c.g_recs, "faithful")
    xe, ye, re_, we = np.zeros((0, len(names))), np.zeros(0, dtype=bool), np.zeros(0, dtype=str), np.zeros(0)
    e_prov = None
    if extra:
        rows = list(extra_records(extra, c.corpus / "skip-sample.jsonl", "faithful"))
        kept = c.held_out(rows)
        xe, ye, re_, we, eschema, e_match = load(c.corpus, kept, c.workers, "faithful")
        if eschema != c.schema:
            sys.exit("silver and extra feature schemas differ")
        we = we / we.mean()
        e_prov = {"file": extra.name, "sha256": sha256(extra), "hunks": len(ye), "unfaithful": int((~ye).sum()),
                  "dropped_in_gold_commits": len(rows) - len(kept), **e_match}
    x, y = np.vstack([c.xs, xe]), np.concatenate([ys, ye])
    r, w = np.concatenate([c.rs, re_]), np.concatenate([c.ws, we])
    silver = np.arange(len(y)) < len(ys)
    xt = drop_history(x, hist, np.random.default_rng(SEED))

    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    loro, curve = {"lr": np.zeros(len(y))}, np.zeros((len(grid), len(y)))
    for repo in sorted(set(r)):
        held = r == repo
        loro["lr"][held] = fit("lr", xt[~held], y[~held], w[~held]).predict_proba(x[held])[:, 1]
        m = fit("trees", xt[~held], y[~held], w[~held])
        for i, n in enumerate(grid):
            curve[i, held] = m.predict_proba(x[held], num_iteration=n)[:, 1]
    loro_auroc = [roc_auc_score(y, s, sample_weight=w) for s in curve]
    best = int(np.argmax(loro_auroc))
    ship = next(i for i, v in enumerate(loro_auroc) if v >= loro_auroc[best] - TREE_TOL)
    n_trees = grid[ship]
    loro["trees"] = curve[ship]
    models = {k: fit(k, xt, y, w) for k in ("lr", "trees")}
    proba = lambda k, x_, n=n_trees: models[k].predict_proba(x_, **({"num_iteration": n} if k == "trees" else {}))[:, 1]
    # unfaithful is the class a doubt flag looks for, so the scores below rank it
    ap = lambda yy, s, ww=None: round(average_precision_score(~yy, 1 - s, sample_weight=ww), 4)
    auroc = lambda yy, s, ww=None: round(roc_auc_score(yy, s, sample_weight=ww), 4)
    ev = {k: {"loro": {"auroc": auroc(y, s, w), "unfaithful_ap": ap(y, s, w),
                       "silver_auroc": auroc(ys, s[silver]), "silver_unfaithful_ap": ap(ys, s[silver])},
              "gold": {"auroc": auroc(yg, proba(k, c.xg)), "unfaithful_ap": ap(yg, proba(k, c.xg)),
                       "brier": round(brier_score_loss(yg, proba(k, c.xg)), 4)}}
          for k, s in loro.items()}
    shipped = "trees" if ev["trees"]["loro"]["unfaithful_ap"] >= ev["lr"]["loro"]["unfaithful_ap"] else "lr"
    selection = {"rule": "the model with the higher leave-one-repo-out average precision for unfaithful "
                         "rationales, lightgbm on a tie", "chosen": shipped}
    if shipped == "lr":
        scaler, lr = models["lr"].named_steps.values()
        model = {"type": "logistic", "mean": [f32(v) for v in scaler.mean_],
                 "scale": [f32(v) for v in scaler.scale_], "coef": [f32(v) for v in lr.coef_[0]],
                 "intercept": f32(lr.intercept_[0])}
    else:
        trees = trimmed_trees(models["trees"].booster_, n_trees)
        both = np.vstack([x, c.xg])
        export_err = float(np.abs(trees_proba(trees, both) - proba("trees", both)).max())
        if export_err > 1e-4:
            sys.exit(f"faithful: trimmed trees disagree with LightGBM by {export_err:.2g}")
        model = {"type": "lightgbm", "objective": "binary", "link": "sigmoid",
                 "score": "sum of the trees' leaf_value", "split": "x[feature] <= threshold goes left",
                 "trees": trees}
    gold_score = proba(shipped, c.xg)
    gate = doubt_gate(loro[shipped], y, w, gold_score, yg)
    provenance = {
        "label": "labels.faithful",
        "train": {**c.provenance["train"], "unfaithful": int((~ys).sum())},
        "eval": {**c.provenance["eval"], "unfaithful": int((~yg).sum())},
        "extra": e_prov, "overlap_min": OVERLAP_MIN, "history_drop": HISTORY_DROP, "seed": SEED,
        "lightgbm_params": {**LGBM, "n_estimators": n_trees},
        "tree_count": {"rule": f"smallest count within {TREE_TOL} of the best pooled leave-one-repo-out "
                               f"AUROC (silver and extra, weighted), every {TREE_STEP} up to {TREE_MAX}",
                       "best": grid[best], "shipped": n_trees, "shipped_loro_auroc": round(loro_auroc[ship], 4)},
    }
    policy = {"passes": gate["passes"], "doubt": gate["doubt"], "rule": gate["rule"]}
    print(f"\nfaithful ({len(ys)} silver, {len(ye)} extra, {len(yg)} gold; unfaithful "
          f"{int((~ys).sum())}/{int((~ye).sum())}/{int((~yg).sum())}), shipping {model['type']} ({n_trees} trees)")
    for k, e in ev.items():
        print(f"  {k:6} LORO AUROC {e['loro']['auroc']:.3f} AP(unfaithful) {e['loro']['unfaithful_ap']:.3f} | "
              f"gold AUROC {e['gold']['auroc']:.3f} AP {e['gold']['unfaithful_ap']:.3f}")
    print(f"doubt gate: {'PASS' if gate['passes'] else 'FAIL'}, threshold {gate['doubt']}")
    for where in ("loro", "gold"):
        if gate[where]:
            g = gate[where]
            print(f"  {where}: {g['flags']} flags, {g['unfaithful_flagged']}/{g['unfaithful']} unfaithful found, "
                  f"precision {g['precision']}, recall {g['recall']}, false-flag rate {g['false_flag_rate']}")
    head = {"model": model, "columns": names, "selection": selection, "provenance": provenance, "policy": policy}
    report = {"selection": selection, "provenance": provenance, "gate": gate, "metrics": ev,
              "tree_count_curve": {"trees": grid, "loro_auroc": [round(v, 4) for v in loro_auroc]}}
    return head, report, gold_score


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "src/hunch.json")
    ap.add_argument("--report", type=Path, default=ROOT / "corpus/hunch-report.json")
    ap.add_argument("--parity", type=Path, default=ROOT / "tests/hunch-parity.json")
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    ap.add_argument("--extra", type=Path, help="more training labels, {key, should_skip} per line")
    ap.add_argument("--extra-faithful", type=Path,
                    help="more training labels, {key, faithful, rationale, details} per line")
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
    xs, s_recs, rs, ws, schema, s_match = load(corpus, silver_kept, a.workers, None)
    xg, g_recs, rg, _, gschema, g_match = load(corpus, gold_records, a.workers, None)
    if gschema != schema:
        sys.exit("silver and gold feature schemas differ")
    c = Data(corpus, schema, xs, s_recs, rs, ws, xg, g_recs, rg, held_out,
             {"train": {"file": "silver.jsonl", "sha256": sha256(silver_path), "hunks": len(rs),
                        "dropped_in_gold_commits": len(silver_records) - len(silver_kept), **s_match},
              "eval": {"file": "corpus/gold.jsonl", "sha256": sha256(gold_path), "hunks": len(rg), **g_match}},
             a.workers)
    skip, skip_report, skip_gold = train_skip(c, a.extra)
    faithful, faithful_report, faithful_gold = train_faithful(c, a.extra_faithful)
    a.out.write_text(dump({"feature_schema": schema, "fingerprint": fp,
                           "heads": {"should_skip": skip, "faithful": faithful}}))
    a.parity.write_text(parity(xg, {"should_skip": skip_gold, "faithful": faithful_gold}))
    a.report.write_text(json.dumps({"heads": {"should_skip": skip_report, "faithful": faithful_report}},
                                   indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
