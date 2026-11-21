#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["numpy", "scipy", "scikit-learn", "lightgbm"]
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

usage: ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py [--out FILE] [--report FILE] [--parity FILE] [--workers N] [--extra FILE] [--extra-faithful FILE] [--extra-attention FILE] [--learning-curve [--curve-out FILE] [--curve-heads LIST]]

--extra adds labeled hunks to the skip head's training, one {key,
should_skip} per line (e.g. $ORDO_CORPUS/judged.should_skip.jsonl);
--extra-faithful does for the faithful head, one {key, faithful, rationale,
details} per line (e.g. $ORDO_CORPUS/judged.faithful.jsonl), dropping a row
whose rationale the engine has since changed. --extra-attention adds
the attention head's review labels (scripts/review-label.py's
attention.review.jsonl): the head then trains as a LightGBM lambdarank over
Opus's levels (a group per commit) and the reviewed snapshots (a group per
snapshot, commented over uncommented), and is gated on the review set too (see
`train_attention_ranked`). Skip evaluation stays on gold
and silver. Rows of both must come from $ORDO_CORPUS/skip-sample.jsonl, whose
stratum_n weights each row back to its stratum's share of the sweep (see
extra_records).

--learning-curve retrains each head on 10/25/50/100% of its training commits
(review labels: snapshots), three seeds per point, and the review head on
10/20/30/41 review repos, scoring every run on the full leave-one-repo-out
folds and gold; it writes --curve-out (corpus/hunch-curve.json), prints a
table, and writes no model, report or parity file.

Build the engine first (cargo build --release); the corpus repos come from
scripts/corpus-fetch.sh.
"""
import argparse
import contextlib
import functools
import hashlib
import io
import itertools
import json
import math
import os
import re
import sys
import zlib
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
from scipy.stats import spearmanr

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
ATTENTION_GATE = ("within-commit pairwise accuracy (pairs with different attention labels, a predicted tie half "
                  "right): above file-position order leaving one repo out overall, below it in at most 2 "
                  "repos, and above it on gold")
REVIEW_OPUS_SHARE = 0.5
REVIEW_LOSING_REPOS = 10
REVIEW_GATE = ("leaving one repo out over the review repos, a commented hunk ranked above an uncommented one of its "
               "snapshot more often than file-position order does, and found in the top 3 and the top 20% of "
               "snapshots with more than 3 hunks more often, losing to file position in at most "
               f"{REVIEW_LOSING_REPOS} repos on pairwise accuracy")
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
    read = {(o["path"], o["hunk"]): i for i, o in enumerate(out["order"])}
    group = {m: gi for gi, g in enumerate(out["groups"]) for m in g["members"]}
    succ = defaultdict(set)
    for e in out["edges"]:
        if group[e["from"]] != group[e["to"]]:
            succ[group[e["from"]]].add(group[e["to"]])
    graph = Graph(dict(succ))
    hunks = [(f"{repo}:{sha[:8]}:{f['path']}:{h['new_range'][0]}", h["old_range"], h["new_range"], h["features"],
              (h["rationale"], h.get("details", [])),
              Where(fi, h["new_range"][0], read[f["path"], h["id"]], group[h["id"]], graph, f"{repo}:{sha[:8]}"))
             for fi, f in enumerate(out["files"]) for h in f["hunks"] if h.get("features") is not None]
    return hunks, out.get("feature_schema")


Where = namedtuple("Where", "file row read group graph commit")


class Graph:
    """A commit's group def->use graph, as the engine's edges between
    different groups; `reaches` walks it."""
    def __init__(self, succ):
        self.succ, self.seen = succ, {}

    def reaches(self, a, b):
        if a not in self.seen:
            seen, todo = {a}, [a]
            while todo:
                for n in self.succ.get(todo.pop(), ()):
                    if n not in seen:
                        seen.add(n)
                        todo.append(n)
            self.seen[a] = seen
        return b in self.seen[a]


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


LOADED = {}


def load(corpus, records, workers, label="should_skip"):
    memo = (label, tuple(r["key"] for r in records))
    if memo not in LOADED:
        LOADED[memo] = load_features(corpus, records, workers, label)
    return LOADED[memo]


def load_features(corpus, records, workers, label):
    """Features and `label` for `records`, each matched to an engine hunk by
    key, else by overlap; the counts of both misses and overlap matches. With
    `label` None, every head's labels: the kept records come back in place of
    the label array, for the caller to read. A record that carries the
    rationale it was labeled on (`text`) is dropped as stale when the engine
    no longer writes that rationale for its hunk."""
    commits = sorted({(r["repo"], r["sha"]) for r in records})
    vecs, texts, where, per_file, schemas = {}, {}, {}, defaultdict(list), set()
    with ThreadPoolExecutor(workers) as ex:
        for hunks, s in ex.map(lambda c: commit_features(corpus, *c), commits):
            for key, old, new, v, text, at in hunks:
                vecs[key] = v
                texts[key] = text
                where[key] = at
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
                                                       **({"stale_rationale": len(stale)} if stale else {})}, \
        [where[k] for _, k in kept]


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


Data = namedtuple("Data", "corpus schema xs s_recs rs ws s_at xg g_recs rg g_at held_out provenance workers subset",
                  defaults=[None])


class Subset:
    """A seeded training subsample for the learning curve: `frac` of the
    commits (or review snapshots), whole groups at a time, or, for the review
    labels, only `repos` randomly drawn review repos besides the held-out one.
    Evaluation rows are never dropped, only training rows."""
    def __init__(self, frac=1.0, seed=0, repos=None):
        self.frac, self.seed, self.repos = frac, seed, repos

    def keep(self, groups):
        groups = np.asarray(groups)
        uniq = np.unique(groups)
        chosen = np.random.default_rng(self.seed).permutation(uniq)[:max(1, round(self.frac * len(uniq)))]
        return np.isin(groups, chosen)

    def keep_repos(self, repos, held):
        if self.repos is None:
            return np.ones(len(repos), dtype=bool)
        others = sorted(set(repos) - {held})
        rng = np.random.default_rng([self.seed, zlib.crc32(str(held).encode())])
        return np.isin(repos, rng.choice(others, min(self.repos, len(others)), replace=False))


def train_keep(c, groups):
    """Which of the rows in commit/snapshot `groups` the run trains on."""
    return np.ones(len(groups), dtype=bool) if c.subset is None else c.subset.keep(groups)


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
    kt = train_keep(c, [a.commit for a in c.s_at])
    if extra:
        rows = list(extra_records(extra, c.corpus / "skip-sample.jsonl"))
        extra_kept = c.held_out(rows)
        e_gold = len(rows) - len(extra_kept)
        xe, ye, re_, we, eschema, e_match, e_at = load(c.corpus, extra_kept, c.workers)
        if eschema != c.schema:
            sys.exit("silver and extra feature schemas differ")
        xe = xe[:, cols]
        we = we / we.mean()
        e_weights = {"min": round(we.min(), 4), "max": round(we.max(), 4),
                     "skip_rate_raw": round(ye.mean(), 4), "skip_rate_weighted": round(np.average(ye, weights=we), 4)}
        xt, yt, rt, wt = np.vstack([xs, xe]), np.concatenate([ys, ye]), np.concatenate([rs, re_]), np.concatenate([ws, we])
        kt = np.concatenate([kt, train_keep(c, [a.commit for a in e_at])])
    xt = drop_history(xt, hist, np.random.default_rng(SEED))

    # pooled leave-one-repo-out AUROC per tree count picks the trees' size
    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    loro, curve = {"lr": np.zeros(len(ys))}, np.zeros((len(grid), len(ys)))
    for r in sorted(set(rs)):
        held, train = rs == r, (rt != r) & kt
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

    models = {k: fit(k, xt[kt], yt[kt], wt[kt]) for k in ("lr", "trees")}
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
    e_prov, e_at = None, []
    if extra:
        rows = list(extra_records(extra, c.corpus / "skip-sample.jsonl", "faithful"))
        kept = c.held_out(rows)
        xe, ye, re_, we, eschema, e_match, e_at = load(c.corpus, kept, c.workers, "faithful")
        if eschema != c.schema:
            sys.exit("silver and extra feature schemas differ")
        we = we / we.mean()
        e_prov = {"file": extra.name, "sha256": sha256(extra), "hunks": len(ye), "unfaithful": int((~ye).sum()),
                  "dropped_in_gold_commits": len(rows) - len(kept), **e_match}
    x, y = np.vstack([c.xs, xe]), np.concatenate([ys, ye])
    r, w = np.concatenate([c.rs, re_]), np.concatenate([c.ws, we])
    silver = np.arange(len(y)) < len(ys)
    kt = np.concatenate([train_keep(c, [a.commit for a in c.s_at]), train_keep(c, [a.commit for a in e_at])])
    xt = drop_history(x, hist, np.random.default_rng(SEED))

    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    loro, curve = {"lr": np.zeros(len(y))}, np.zeros((len(grid), len(y)))
    for repo in sorted(set(r)):
        held = r == repo
        train = ~held & kt
        loro["lr"][held] = fit("lr", xt[train], y[train], w[train]).predict_proba(x[held])[:, 1]
        m = fit("trees", xt[train], y[train], w[train])
        for i, n in enumerate(grid):
            curve[i, held] = m.predict_proba(x[held], num_iteration=n)[:, 1]
    loro_auroc = [roc_auc_score(y, s, sample_weight=w) for s in curve]
    best = int(np.argmax(loro_auroc))
    ship = next(i for i, v in enumerate(loro_auroc) if v >= loro_auroc[best] - TREE_TOL)
    n_trees = grid[ship]
    loro["trees"] = curve[ship]
    models = {k: fit(k, xt[kt], y[kt], w[kt]) for k in ("lr", "trees")}
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


def attention_rows(c, path, recs, at):
    """The rows of `recs` an attention file labels: their indices, levels
    (1-3), later_fix (True/False/None), and commit and position per row."""
    lab = {r["key"]: r for l in path.read_text().splitlines() if l.strip() for r in [json.loads(l)]}
    idx = [i for i, r in enumerate(recs) if r["key"] in lab]
    level = np.array([lab[recs[i]["key"]]["attention"] for i in idx])
    later = [lab[recs[i]["key"]].get("later_fix") for i in idx]
    commit = np.array([f"{recs[i]['repo']}:{recs[i]['sha'][:8]}" for i in idx])
    return np.array(idx, dtype=int), level, later, commit, [at[i] for i in idx]


def pairs(commit, level, at, free_only=False):
    """(i, j) with i before j in the same commit and different labels; with
    `free_only`, only those in different groups neither of which reaches the
    other in the def->use graph, the pairs a tiebreak can move."""
    by = defaultdict(list)
    for i, k in enumerate(commit):
        by[k].append(i)
    out = []
    for ix in by.values():
        for a in range(len(ix)):
            for b in range(a + 1, len(ix)):
                i, j = ix[a], ix[b]
                if level[i] == level[j]:
                    continue
                if free_only:
                    gi, gj = at[i].group, at[j].group
                    if gi == gj or at[i].graph.reaches(gi, gj) or at[i].graph.reaches(gj, gi):
                        continue
                out.append((i, j))
    return np.array(out, dtype=int).reshape(-1, 2)


def pairwise(score, level, pr, mask=None):
    """Share of pairs `pr` ranked as their labels are (the higher score on the
    higher level), a predicted tie half right; and the pair count."""
    if mask is not None:
        pr = pr[mask[pr[:, 0]]]
    if not len(pr):
        return None, 0
    ds = score[pr[:, 0]] - score[pr[:, 1]]
    dl = level[pr[:, 0]] - level[pr[:, 1]]
    right = np.where(ds == 0, 0.5, (np.sign(ds) == np.sign(dl)).astype(float))
    return round(float(right.mean()), 4), len(pr)


def commit_spearman(score, level, commit):
    """Mean Spearman correlation of score and label over commits with at
    least two labels and two different ones; a commit whose scores all tie
    counts 0."""
    vals = []
    for k in sorted(set(commit)):
        m = commit == k
        if m.sum() >= 2 and len(set(level[m])) >= 2:
            vals.append(spearmanr(score[m], level[m]).statistic if len(set(score[m])) >= 2 else 0.0)
    return round(float(np.mean(vals)), 4), len(vals)


def baselines(x, at, names):
    """Today's and the naive rankings, higher = read first: file position
    (file, then start row; the tiebreak the def->use order falls back on),
    hunk size (lines added + removed), rule priority (no rule in the corpus
    sets one: every pair ties) and the engine's own reading order."""
    pos = np.lexsort(([a.row for a in at], [a.file for a in at])).argsort().astype(float)
    size = x[:, names.index("lines_added")] + x[:, names.index("lines_removed")]
    return {"file_position": -pos, "hunk_size": size, "rule_priority": np.zeros(len(at)),
            "reading_order": -np.array([a.read for a in at], dtype=float)}


def rank_metrics(scores, level, commit, repos, at):
    """Per ranking: pairwise accuracy over all and over free pairs, overall
    and per repo, and the mean within-commit Spearman."""
    pr, free = pairs(commit, level, at), pairs(commit, level, at, True)
    out = {}
    for name, s in scores.items():
        acc, n = pairwise(s, level, pr)
        facc, fn = pairwise(s, level, free)
        sp, ncommits = commit_spearman(s, level, commit)
        out[name] = {"pairwise": acc, "pairs": n, "pairwise_free": facc, "free_pairs": fn,
                     "spearman": sp, "commits": ncommits,
                     "per_repo": {r: pairwise(s, level, pr, repos == r)[0] for r in sorted(set(repos))}}
    return out


def later_fix_by_attention(score, later):
    """later_fix rate per third of hunks by predicted attention, over the
    hunks whose later_fix is known."""
    known = np.array([v is not None for v in later])
    s, y = score[known], np.array([bool(v) for v in later])[known]
    cut = np.quantile(s, [1 / 3, 2 / 3])
    bins = np.digitize(s, cut)
    return {name: {"hunks": int((bins == b).sum()), "later_fix": int(y[bins == b].sum())}
            for b, name in enumerate(("low", "mid", "high"))}


def attention_gate(loro_ev, gold_ev, h):
    """`ATTENTION_GATE` for ranking `h` on the Opus-labeled `rank_metrics`."""
    base = "file_position"
    lost = sorted(k for k, v in loro_ev[h]["per_repo"].items()
                  if v is not None and loro_ev[base]["per_repo"][k] is not None and v < loro_ev[base]["per_repo"][k])
    gate = {"rule": ATTENTION_GATE, "model": h,
            "loro": {"hunch": loro_ev[h]["pairwise"], "file_position": loro_ev[base]["pairwise"],
                     "pairs": loro_ev[h]["pairs"], "losing_repos": lost},
            "gold": {"hunch": gold_ev[h]["pairwise"], "file_position": gold_ev[base]["pairwise"],
                     "pairs": gold_ev[h]["pairs"]}}
    gate["passes"] = bool(gate["loro"]["hunch"] > gate["loro"]["file_position"] and len(lost) <= 2
                          and gate["gold"]["hunch"] > gate["gold"]["file_position"])
    return gate


def train_attention(c, gold_path, silver_path):
    """The attention head: how much a hunk needs a reviewer's attention,
    from Opus's 1-3 rating, as a fractional logistic target (level-1)/2 (each
    row twice, as a 0 weighted 1-t and a 1 weighted t), so one sigmoid head
    ranks by expected level and the 2-vs-3 distinction counts. Tree count
    picked on leave-one-repo-out log-loss; logistic vs trees on its
    within-commit pairwise accuracy. The gate (`ATTENTION_GATE`) decides
    whether `--hunch` may reorder by it."""
    names = c.schema["names"]
    hist = [names.index(n) for n in HISTORY]
    si, ls, later, cs, sat = attention_rows(c, silver_path, c.s_recs, c.s_at)
    gi, lg, _, cg, gat = attention_rows(c, gold_path, c.g_recs, c.g_at)
    x, r, w = c.xs[si], c.rs[si], c.ws[si]
    xg, rg = c.xg[gi], c.rg[gi]
    t = (ls - 1) / 2

    def doubled(rows):
        xx = np.vstack([x[rows], x[rows]])
        yy = np.concatenate([np.zeros(len(rows), dtype=bool), np.ones(len(rows), dtype=bool)])
        ww = np.concatenate([w[rows] * (1 - t[rows]), w[rows] * t[rows]])
        keep = ww > 0
        return drop_history(xx, hist, np.random.default_rng(SEED))[keep], yy[keep], ww[keep]

    def loss(p, rows=slice(None)):
        p = np.clip(p, 1e-9, 1 - 1e-9)
        return float(np.average(-(t[rows] * np.log(p) + (1 - t[rows]) * np.log(1 - p)), weights=w[rows]))

    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    loro, curve = {"lr": np.zeros(len(t))}, np.zeros((len(grid), len(t)))
    for repo in sorted(set(r)):
        held = r == repo
        xt, yt, wt = doubled(np.flatnonzero(~held))
        loro["lr"][held] = fit("lr", xt, yt, wt).predict_proba(x[held])[:, 1]
        m = fit("trees", xt, yt, wt)
        for i, n in enumerate(grid):
            curve[i, held] = m.predict_proba(x[held], num_iteration=n)[:, 1]
    losses = [loss(cv) for cv in curve]
    best = int(np.argmin(losses))
    ship = next(i for i, v in enumerate(losses) if v <= losses[best] + TREE_TOL)
    n_trees = grid[ship]
    loro["trees"] = curve[ship]
    xt, yt, wt = doubled(np.arange(len(t)))
    models = {k: fit(k, xt, yt, wt) for k in ("lr", "trees")}
    proba = lambda k, x_, n=n_trees: models[k].predict_proba(x_, **({"num_iteration": n} if k == "trees" else {}))[:, 1]

    loro_ev = rank_metrics({**baselines(x, sat, names), **loro}, ls, cs, r, sat)
    shipped = "trees" if loro_ev["trees"]["pairwise"] >= loro_ev["lr"]["pairwise"] else "lr"
    gold_scores = {k: proba(k, xg) for k in models}
    gold_ev = rank_metrics({**baselines(xg, gat, names), **gold_scores}, lg, cg, rg, gat)
    if shipped == "lr":
        scaler, lr = models["lr"].named_steps.values()
        model = {"type": "logistic", "mean": [f32(v) for v in scaler.mean_],
                 "scale": [f32(v) for v in scaler.scale_], "coef": [f32(v) for v in lr.coef_[0]],
                 "intercept": f32(lr.intercept_[0])}
    else:
        trees = trimmed_trees(models["trees"].booster_, n_trees)
        both = np.vstack([c.xs, c.xg])
        export_err = float(np.abs(trees_proba(trees, both) - proba("trees", both)).max())
        if export_err > 1e-4:
            sys.exit(f"attention: trimmed trees disagree with LightGBM by {export_err:.2g}")
        model = {"type": "lightgbm", "objective": "binary", "link": "sigmoid",
                 "score": "sum of the trees' leaf_value", "split": "x[feature] <= threshold goes left",
                 "trees": trees}
    h = shipped
    gate = attention_gate(loro_ev, gold_ev, h)
    base, lost = "file_position", gate["loro"]["losing_repos"]
    provenance = {
        "label": "attention (Opus 1-3) as (level-1)/2",
        "train": {"file": silver_path.name, "sha256": sha256(silver_path), "hunks": len(t),
                  "levels": {str(k): int((ls == k).sum()) for k in (1, 2, 3)}},
        "eval": {"file": "corpus/attention.gold.jsonl", "sha256": sha256(gold_path), "hunks": len(lg),
                 "levels": {str(k): int((lg == k).sum()) for k in (1, 2, 3)}},
        "history_drop": HISTORY_DROP, "seed": SEED, "lightgbm_params": {**LGBM, "n_estimators": n_trees},
        "tree_count": {"rule": f"smallest count within {TREE_TOL} of the best pooled leave-one-repo-out "
                               f"log-loss, every {TREE_STEP} up to {TREE_MAX}",
                       "best": grid[best], "shipped": n_trees, "shipped_loro_log_loss": round(losses[ship], 4)},
    }
    selection = {"rule": "the model with the higher leave-one-repo-out within-commit pairwise accuracy, "
                         "lightgbm on a tie", "chosen": shipped}
    report = {"selection": selection, "provenance": provenance, "gate": gate,
              "metrics": {"loro": loro_ev, "gold": gold_ev},
              "loro_log_loss": {"lr": round(loss(loro["lr"]), 4), "trees": round(losses[ship], 4)},
              "later_fix_by_attention_loro": later_fix_by_attention(loro[shipped], later),
              "tree_count_curve": {"trees": grid, "loro_log_loss": [round(v, 4) for v in losses]}}
    print(f"\nattention ({len(t)} silver, {len(lg)} gold), shipping {model['type']} ({n_trees} trees)")
    print(f"{'ranking':14} {'LORO pair':>9} {'free':>6} {'rho':>6} | {'gold pair':>9} {'free':>6} {'rho':>6}")
    for k in loro_ev:
        a, g = loro_ev[k], gold_ev[k]
        print(f"{k:14} {a['pairwise']:9.3f} {a['pairwise_free'] or 0:6.3f} {a['spearman']:6.3f} | "
              f"{g['pairwise']:9.3f} {g['pairwise_free'] or 0:6.3f} {g['spearman']:6.3f}")
    print(f"pairs LORO {loro_ev[h]['pairs']} (free {loro_ev[h]['free_pairs']}), gold {gold_ev[h]['pairs']} "
          f"(free {gold_ev[h]['free_pairs']})")
    print(f"{'repo':14} {'file_pos':>8} {'hunch':>8}")
    for k, v in loro_ev[base]["per_repo"].items():
        print(f"{k:14} {v if v is not None else float('nan'):8.3f} {loro_ev[h]['per_repo'][k] or float('nan'):8.3f}"
              + ("  loses" if k in lost else ""))
    print(f"later_fix by predicted attention (LORO): {report['later_fix_by_attention_loro']}")
    print(f"attention gate: {'PASS' if gate['passes'] else 'FAIL'}")
    head = {"model": model, "columns": names, "selection": selection, "provenance": provenance,
            "policy": {"passes": gate["passes"], "rule": ATTENTION_GATE}}
    return head, report, proba(shipped, c.xg)


Review = namedtuple("Review", "x repo unit y file_pos read slices later")


@functools.cache
def review_rows(path, width):
    """The review set: features, repo, snapshot id, commented, and the file
    position and reading order rankings (higher = read first), rows grouped
    by snapshot; `slices` is each snapshot's (start, end); `later` is
    later_fix as 1/0, -1 when unknown or uncommented."""
    rows = [json.loads(l) for l in path.read_text().splitlines() if l.strip()]
    if any("read" not in r for r in rows):
        sys.exit(f"{path} lacks file/read: rerun scripts/review-label.py")
    if any(len(r["features"]) != width for r in rows):
        sys.exit(f"{path} has feature vectors of another width than the schema's {width}: rerun scripts/review-label.py")
    ids = {}
    unit = np.array([ids.setdefault(f"{r['source']}:{r['repo']}:{r['pr']}:{r['snapshot']}", len(ids)) for r in rows])
    o = np.argsort(unit, kind="stable")
    rows, unit = [rows[i] for i in o], unit[o]
    cut = np.flatnonzero(np.diff(unit)) + 1
    return Review(np.array([r["features"] for r in rows], dtype=float), np.array([r["repo"] for r in rows]), unit,
                  np.array([r["attention"] for r in rows]),
                  -np.array([r["file"] * 1e6 + r["new_range"][0] for r in rows], dtype=float),
                  -np.array([r["read"] for r in rows], dtype=float),
                  list(zip([0, *cut], [*cut, len(rows)])),
                  np.array([-1 if r["later_fix"] is None else int(r["later_fix"]) for r in rows]))


def review_metrics(scores, rv):
    """Per ranking, within each snapshot with commented and uncommented
    hunks: pairwise accuracy of commented over uncommented (a tie half right;
    pooled over pairs, per snapshot's mean, per repo), and, over those with
    more than 3 hunks, the share with a commented hunk in the top 3 and in the
    top 20%. `chance` is what a random order gets for the top-k shares."""
    mixed = [(a, b) for a, b in rv.slices if 0 < rv.y[a:b].sum() < b - a]
    big = [(a, b) for a, b in mixed if b - a > 3]
    k20 = lambda n: -(-n // 5)
    hit = lambda n, c, k: 1 - math.comb(n - c, k) / math.comb(n, k)
    chance = {"top3": round(float(np.mean([hit(b - a, rv.y[a:b].sum(), 3) for a, b in big])), 4),
              "top20": round(float(np.mean([hit(b - a, rv.y[a:b].sum(), k20(b - a)) for a, b in big])), 4)}
    out = {"chance": chance, "snapshots": len(mixed), "topk_snapshots": len(big)}
    for name, s in scores.items():
        right, pairs, macro, per = 0.0, 0, [], defaultdict(lambda: [0.0, 0])
        for a, b in mixed:
            pos, neg = s[a:b][rv.y[a:b]], s[a:b][~rv.y[a:b]]
            r = float((pos[:, None] > neg[None]).sum() + 0.5 * (pos[:, None] == neg[None]).sum())
            n = len(pos) * len(neg)
            right, pairs = right + r, pairs + n
            macro.append(r / n)
            per[rv.repo[a]][0] += r
            per[rv.repo[a]][1] += n
        top = lambda k: float(np.mean([rv.y[a:b][np.argsort(-s[a:b], kind="stable")[:k(b - a)]].any() for a, b in big]))
        out[name] = {"pairwise": round(right / pairs, 4), "pairs": pairs, "pairwise_macro": round(float(np.mean(macro)), 4),
                     "top3": round(top(lambda n: 3), 4), "top20": round(top(k20), 4),
                     "per_repo": {r: round(v[0] / v[1], 4) for r, v in sorted(per.items())}}
    return out


def review_subset(rv, keep, y):
    """`rv` restricted to the rows `keep`, labeled `y`, its snapshots re-sliced."""
    unit = rv.unit[keep]
    cut = np.flatnonzero(np.diff(unit)) + 1
    return rv._replace(x=rv.x[keep], repo=rv.repo[keep], unit=unit, y=y[keep], file_pos=rv.file_pos[keep],
                       read=rv.read[keep], later=rv.later[keep], slices=list(zip([0, *cut], [*cut, len(unit)])))


def later_fix_review(scores, rv):
    """Per ranking, how review hunks a later revision rewrote where a human
    commented (later_fix) rank within their snapshot: `over_uncommented`,
    `review_metrics` of later_fix hunks against uncommented ones; `over_commented`,
    pairwise accuracy of later_fix hunks over commented ones no later revision
    touched (does the ranking tell a fix-prompting comment from discussion);
    `thirds`, later_fix hunks per third of each snapshot of 3 or more hunks
    by the ranking's within-snapshot order."""
    fix = rv.later == 1
    quiet, talk = fix | ~rv.y, rv.later >= 0
    quiet_ev = review_metrics({k: v[quiet] for k, v in scores.items()}, review_subset(rv, quiet, fix))
    talk_ev = review_metrics({k: v[talk] for k, v in scores.items()}, review_subset(rv, talk, fix))
    out = {"later_fix": int(fix.sum()), "commented_not_fixed": int((rv.later == 0).sum()),
           "repos_with_later_fix": len(set(rv.repo[fix])), "chance": quiet_ev["chance"]}
    for name, s in scores.items():
        thirds = np.zeros((3, 2), dtype=int)
        for a, b in rv.slices:
            if b - a < 3:
                continue
            third = np.empty(b - a, dtype=int)
            third[np.argsort(-s[a:b], kind="stable")] = np.arange(b - a) * 3 // (b - a)
            np.add.at(thirds, (third, 0), 1)
            np.add.at(thirds, (third, 1), fix[a:b])
        out[name] = {"over_uncommented": {k: quiet_ev[name][k] for k in ("pairwise", "pairs", "top3", "top20")},
                     "over_commented": {k: talk_ev[name][k] for k in ("pairwise", "pairs")},
                     "thirds": {t: {"hunks": int(h), "later_fix": int(f)}
                                for t, (h, f) in zip(("first", "middle", "last"), thirds)}}
    return out


def review_gate(ev, h):
    """`REVIEW_GATE` on `review_metrics` of a leave-one-repo-out run, `h`
    against file position."""
    base = "file_position"
    lost = sorted(r for r, v in ev[h]["per_repo"].items() if v < ev[base]["per_repo"][r])
    beats = {k: ev[h][k] > ev[base][k] for k in ("pairwise", "top3", "top20")}
    return {"rule": REVIEW_GATE, "model": h, "beats_file_position": beats, "losing_repos": lost,
            "passes": bool(all(beats.values()) and len(lost) <= REVIEW_LOSING_REPOS)}


def head_scores(model, x):
    """A shipped head's score for `x`, as src/hunch.json describes it."""
    if model["type"] == "lightgbm":
        return trees_proba(model["trees"], x)
    z = (x - np.array(model["mean"])) / np.array(model["scale"])
    return 1 / (1 + np.exp(-(z @ np.array(model["coef"]) + model["intercept"])))


def run_lengths(ids):
    return [len(list(g)) for _, g in itertools.groupby(ids)]


def train_attention_ranked(c, gold_path, silver_path, review_path, shipped_model):
    """The attention head trained as a LightGBM lambdarank: Opus's levels
    (1-3, one group per commit, weighted to REVIEW_OPUS_SHARE of the review
    rows' weight) and reviewed snapshots (one group per snapshot, relevance 1
    for a hunk a human commented on, 0 for the rest). Only the order within a
    group is learned, so a PR's comment density never leaks across PRs, and
    uncommented hunks count only as lower-ranked than commented ones in the
    same snapshot. The tree count is the smallest within TREE_TOL of the best
    pooled leave-one-repo-out review pairwise accuracy. Two gates: the
    attention gate on Opus's labels, each fold also training on every review
    row, and `review_gate` over the review repos, each fold training on Opus
    and the other review repos. `shipped_model` is the head in src/hunch.json
    before this run, scored on the review set as the no-review-labels
    reference. Returns None when either gate fails."""
    names = c.schema["names"]
    hist = [names.index(n) for n in HISTORY]
    rv = review_rows(review_path, len(names))
    si, ls, _, cs, sat = attention_rows(c, silver_path, c.s_recs, c.s_at)
    gi, lg, _, cg, gat = attention_rows(c, gold_path, c.g_recs, c.g_at)
    o = np.argsort(cs, kind="stable")
    si, ls, cs, sat = si[o], ls[o], cs[o], [sat[i] for i in o]
    xo, ro, wo = c.xs[si], c.rs[si], c.ws[si]
    wo = wo / wo.mean() * REVIEW_OPUS_SHARE * len(rv.y) / len(xo)
    xg, rg = c.xg[gi], c.rg[gi]
    rng = np.random.default_rng(SEED)
    x = drop_history(np.vstack([xo, rv.x]), hist, rng)
    rel = np.concatenate([ls - 1, rv.y.astype(int)])
    w = np.concatenate([wo, np.ones(len(rv.y))])
    grp = np.concatenate([[f"o{k}" for k in cs], [f"r{u}" for u in rv.unit]])
    repo = np.concatenate([ro, rv.repo])
    nopus = len(xo)

    def train_rows(held):
        keep = np.ones(len(x), dtype=bool)
        if c.subset is not None:
            keep[:nopus] = c.subset.keep(cs)
            keep[nopus:] = c.subset.keep(rv.unit) & c.subset.keep_repos(rv.repo, held)
        return keep

    def fit_rank(m, n):
        return lightgbm.LGBMRanker(**{**LGBM, "n_estimators": n}).fit(
            x[m], rel[m], group=run_lengths(grp[m]), sample_weight=w[m])

    grid = list(range(TREE_STEP, TREE_MAX + 1, TREE_STEP))
    review_repos = sorted(set(rv.repo))

    def review_fold(r):
        mod = fit_rank((repo != r) & train_rows(r), TREE_MAX)
        held = rv.repo == r
        return held, np.array([mod.predict(rv.x[held], num_iteration=n) for n in grid])

    curve = np.zeros((len(grid), len(rv.y)))
    with ThreadPoolExecutor(c.workers) as ex:
        for held, p in ex.map(review_fold, review_repos):
            curve[:, held] = p
    pooled = [review_metrics({"m": s}, rv)["m"]["pairwise"] for s in curve]
    best = int(np.argmax(pooled))
    ship = next(i for i, v in enumerate(pooled) if v >= pooled[best] - TREE_TOL)
    n_trees = grid[ship]
    review_loro = {**{k: v for k, v in {"file_position": rv.file_pos, "reading_order": rv.read}.items()},
                   "lambdarank": curve[ship]}
    review_ev = review_metrics(review_loro, rv)
    rgate = review_gate(review_ev, "lambdarank")

    def opus_fold(r):
        held = ro == r
        mod = fit_rank(~np.concatenate([ro == r, np.zeros(len(rv.y), dtype=bool)]) & train_rows(r), n_trees)
        return held, mod.predict(xo[held])

    opus_loro = np.zeros(len(xo))
    with ThreadPoolExecutor(c.workers) as ex:
        for held, p in ex.map(opus_fold, sorted(set(ro))):
            opus_loro[held] = p
    model_all = fit_rank(train_rows(None), n_trees)
    gold_raw = model_all.predict(xg)
    loro_ev = rank_metrics({**baselines(xo, sat, names), "trees": opus_loro}, ls, cs, ro, sat)
    gold_ev = rank_metrics({**baselines(xg, gat, names), "trees": gold_raw}, lg, cg, rg, gat)
    trees = trimmed_trees(model_all.booster_, n_trees)
    both = np.vstack([c.xs, c.xg])
    export_err = float(np.abs(trees_proba(trees, both) - 1 / (1 + np.exp(-model_all.predict(both)))).max())
    if export_err > 1e-4:
        sys.exit(f"attention: trimmed trees disagree with LightGBM by {export_err:.2g}")
    model = {"type": "lightgbm", "objective": "lambdarank", "link": "sigmoid",
             "score": "sum of the trees' leaf_value; only the order within a commit means anything",
             "split": "x[feature] <= threshold goes left", "trees": trees}
    gate = attention_gate(loro_ev, gold_ev, "trees")
    shipped_review = review_metrics({"file_position": rv.file_pos, "reading_order": rv.read,
                                     "current_model": head_scores(shipped_model, rv.x)}, rv)
    passes = gate["passes"] and rgate["passes"]
    later_ev = later_fix_review({"file_position": rv.file_pos, "reading_order": rv.read,
                                 "current_model": head_scores(shipped_model, rv.x), "lambdarank": curve[ship]}, rv)
    provenance = {
        "label": "attention (Opus 1-3 as lambdarank levels) + review comments (commented over uncommented per snapshot)",
        "train": {"file": silver_path.name, "sha256": sha256(silver_path), "hunks": nopus,
                  "levels": {str(k): int((ls == k).sum()) for k in (1, 2, 3)}, "weight_share_of_review": REVIEW_OPUS_SHARE},
        "review": {"file": review_path.name, "sha256": sha256(review_path), "hunks": len(rv.y),
                   "commented": int(rv.y.sum()), "snapshots": len(rv.slices), "repos": len(review_repos),
                   "snapshots_with_both": review_ev["snapshots"]},
        "eval": {"file": "corpus/attention.gold.jsonl", "sha256": sha256(gold_path), "hunks": len(lg),
                 "levels": {str(k): int((lg == k).sum()) for k in (1, 2, 3)}},
        "reference_model": {"what": "attention head in src/hunch.json before this run, scored as current_model",
                            "sha256": hashlib.sha256(json.dumps(shipped_model, sort_keys=True).encode()).hexdigest()},
        "history_drop": HISTORY_DROP, "seed": SEED, "lightgbm_params": {**LGBM, "n_estimators": n_trees},
        "tree_count": {"rule": f"smallest count within {TREE_TOL} of the best pooled leave-one-repo-out review "
                               f"pairwise accuracy, every {TREE_STEP} up to {TREE_MAX}",
                       "best": grid[best], "shipped": n_trees, "shipped_review_pairwise": pooled[ship]},
    }
    selection = {"rule": "lightgbm lambdarank over Opus levels and review comments; no logistic alternative",
                 "chosen": "trees"}
    report = {"selection": selection, "provenance": provenance, "gate": gate, "review_gate": rgate,
              "metrics": {"loro": loro_ev, "gold": gold_ev, "review_loro": review_ev,
                          "review_shipped_without_review_labels": shipped_review},
              "later_fix_review": later_ev,
              "tree_count_curve": {"trees": grid, "review_loro_pairwise": pooled}}
    print(f"\nattention, review labels ({nopus} Opus, {len(rv.y)} review hunks in {review_ev['snapshots']} mixed "
          f"snapshots of {len(review_repos)} repos), lambdarank {n_trees} trees")
    print(f"{'review set':22} {'pairwise':>8} {'macro':>6} {'top3':>6} {'top20':>6}")
    for name, e in [*shipped_review.items(), *[(k, v) for k, v in review_ev.items() if k == "lambdarank"]]:
        if isinstance(e, dict) and "pairwise" in e:
            tag = f"{name} (LORO)" if name == "lambdarank" else name
            print(f"{tag:22} {e['pairwise']:8.3f} {e['pairwise_macro']:6.3f} {e['top3']:6.3f} {e['top20']:6.3f}")
    print(f"{'chance (top-k)':22} {'':8} {'':6} {review_ev['chance']['top3']:6.3f} {review_ev['chance']['top20']:6.3f}")
    print(f"\nlater_fix ({later_ev['later_fix']} hunks in {later_ev['repos_with_later_fix']} repos, "
          f"{later_ev['commented_not_fixed']} commented not fixed)")
    print(f"{'ranking':22} {'vs quiet':>8} {'top3':>6} {'top20':>6} {'vs talk':>8}  first/middle/last third")
    for name in ("file_position", "reading_order", "current_model", "lambdarank"):
        e = later_ev[name]
        q, t = e["over_uncommented"], e["over_commented"]
        print(f"{name:22} {q['pairwise']:8.3f} {q['top3']:6.3f} {q['top20']:6.3f} {t['pairwise']:8.3f}  "
              + "/".join(f"{v['later_fix'] / v['hunks']:.3f}" for v in e["thirds"].values()))
    print(f"{'chance (top-k)':22} {'':8} {later_ev['chance']['top3']:6.3f} {later_ev['chance']['top20']:6.3f}")
    print(f"review gate: {'PASS' if rgate['passes'] else 'FAIL'} {rgate['beats_file_position']}, "
          f"losing repos {len(rgate['losing_repos'])}")
    print(f"Opus attention gate: {'PASS' if gate['passes'] else 'FAIL'} LORO {gate['loro']['hunch']} vs "
          f"{gate['loro']['file_position']}, gold {gate['gold']['hunch']} vs {gate['gold']['file_position']}")
    head = {"model": model, "columns": names, "selection": selection, "provenance": provenance,
            "policy": {"passes": gate["passes"], "rule": ATTENTION_GATE, "review_passes": rgate["passes"],
                       "review_rule": REVIEW_GATE}}
    return (head, report, 1 / (1 + np.exp(-model_all.predict(c.xg)))) if passes else (None, report, None)


CURVE_FRACTIONS, CURVE_REPOS, CURVE_SEEDS = (0.1, 0.25, 0.5, 1.0), (10, 20, 30, 41), 3


def curve_metrics(head, report):
    """The numbers a learning curve tracks for one trained head."""
    pct = lambda v: None if v is None else round(v, 4)
    if head == "skip":
        k = "lr" if report["selection"]["chosen"] == "logistic" else "trees"
        lo, go = (report["metrics"][w][k] for w in ("leave_one_repo_out", "gold"))
        return {"loro_auroc": lo["auroc"], "loro_pr_auc": lo["pr_auc"], "gold_auroc": go["auroc"],
                "gold_pr_auc": go["pr_auc"], "policy_passes": float(report["promotion"]["gate"]["passes"])}
    if head == "faithful":
        k, g = report["selection"]["chosen"], report["gate"]
        return {"loro_auroc": report["metrics"][k]["loro"]["auroc"], "gold_auroc": report["metrics"][k]["gold"]["auroc"],
                "doubt": g["doubt"], "doubt_loro_recall": g["loro"] and g["loro"]["recall"],
                "doubt_gold_recall": g["gold"] and g["gold"]["recall"], "passes": float(g["passes"])}
    m, rv = report["metrics"], report["metrics"]["review_loro"]["lambdarank"]
    return {"opus_loro_pairwise": m["loro"]["trees"]["pairwise"], "opus_gold_pairwise": m["gold"]["trees"]["pairwise"],
            "review_pairwise": rv["pairwise"], "review_top3": rv["top3"], "review_top20": rv["top20"],
            "passes": float(report["gate"]["passes"] and report["review_gate"]["passes"])}


def learning_curve(c, a, out):
    """Retrain each head on growing shares of its training labels (and the
    review head on growing numbers of review repos), every run scored on the
    full leave-one-repo-out folds and gold, `CURVE_SEEDS` seeds per point
    (one at 100%). Results accumulate in `out` after every run."""
    previous = json.loads((ROOT / "src/hunch.json").read_text())["heads"]["attention"]["model"]
    gold_path, silver_path = ROOT / "corpus/attention.gold.jsonl", c.corpus / "attention.silver.jsonl"
    results = json.loads(out.read_text()) if out.exists() else {}
    trainers = {"skip": lambda cs: train_skip(cs, a.extra), "faithful": lambda cs: train_faithful(cs, a.extra_faithful),
                "attention": lambda cs: train_attention_ranked(cs, ROOT / "corpus/attention.gold.jsonl", silver_path,
                                                               a.extra_attention, previous)}
    points = [(h, "fraction", f) for h in a.curve_heads if h in trainers for f in CURVE_FRACTIONS]
    points += [("attention", "repos", n) for n in CURVE_REPOS] if "repos" in a.curve_heads else []
    for head, axis, v in points:
        full = v == (1.0 if axis == "fraction" else CURVE_REPOS[-1])
        runs = []
        for seed in range(1 if full else CURVE_SEEDS):
            sub = Subset(frac=v, seed=seed) if axis == "fraction" else Subset(seed=seed, repos=None if full else v)
            with contextlib.redirect_stdout(io.StringIO()):
                _, report, _ = trainers[head](c._replace(subset=sub))[:3]
            runs.append(curve_metrics(head, report))
            print(f"{head} {axis} {v} seed {seed}: {runs[-1]}", flush=True)
        agg = {k: {"mean": round(float(np.mean([r[k] for r in runs if r[k] is not None])), 4),
                   "sd": round(float(np.std([r[k] for r in runs if r[k] is not None])), 4), "runs": [r[k] for r in runs]}
               for k in runs[0]}
        results.setdefault(head if axis == "fraction" else "review_repos", {})[str(v)] = agg
        out.write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")
    for name, rows in results.items():
        print(f"\n{name}")
        for v, agg in sorted(rows.items(), key=lambda kv: float(kv[0])):
            print(f"{v:>6} " + " ".join(f"{k}={m['mean']:.3f}+-{m['sd']:.3f}" for k, m in agg.items() if m["mean"] == m["mean"]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "src/hunch.json")
    ap.add_argument("--report", type=Path, default=ROOT / "corpus/hunch-report.json")
    ap.add_argument("--parity", type=Path, default=ROOT / "tests/hunch-parity.json")
    ap.add_argument("--workers", type=int, default=os.cpu_count())
    ap.add_argument("--extra", type=Path, help="more training labels, {key, should_skip} per line")
    ap.add_argument("--extra-faithful", type=Path,
                    help="more training labels, {key, faithful, rationale, details} per line")
    ap.add_argument("--extra-attention", type=Path,
                    help="review labels for the attention head, scripts/review-label.py's output")
    ap.add_argument("--learning-curve", action="store_true",
                    help="retrain on subsets of the training labels and write --curve-out; writes no model")
    ap.add_argument("--curve-out", type=Path, default=ROOT / "corpus/hunch-curve.json")
    ap.add_argument("--curve-heads", default="skip,faithful,attention,repos",
                    help="comma list of skip, faithful, attention, repos for --learning-curve")
    a = ap.parse_args()
    a.curve_heads = a.curve_heads.split(",")
    fp = fingerprint()
    corpus = Path(os.environ.get("ORDO_CORPUS", "~/.cache/ordo-corpus")).expanduser()
    gold_path, silver_path = ROOT / "corpus/gold.jsonl", corpus / "silver.jsonl"
    read = lambda p: [json.loads(l) for l in p.read_text().splitlines() if l.strip()]
    gold_records = read(gold_path)
    gold_commits = {(r["repo"], r["sha"][:8]) for r in gold_records}
    held_out = lambda records: [r for r in records if (r["repo"], r["sha"][:8]) not in gold_commits]
    silver_records = read(silver_path)
    silver_kept = held_out(silver_records)
    xs, s_recs, rs, ws, schema, s_match, s_at = load(corpus, silver_kept, a.workers, None)
    xg, g_recs, rg, _, gschema, g_match, g_at = load(corpus, gold_records, a.workers, None)
    if gschema != schema:
        sys.exit("silver and gold feature schemas differ")
    c = Data(corpus, schema, xs, s_recs, rs, ws, s_at, xg, g_recs, rg, g_at, held_out,
             {"train": {"file": "silver.jsonl", "sha256": sha256(silver_path), "hunks": len(rs),
                        "dropped_in_gold_commits": len(silver_records) - len(silver_kept), **s_match},
              "eval": {"file": "corpus/gold.jsonl", "sha256": sha256(gold_path), "hunks": len(rg), **g_match}},
             a.workers)
    if a.learning_curve:
        return learning_curve(c, a, a.curve_out)
    skip, skip_report, skip_gold = train_skip(c, a.extra)
    faithful, faithful_report, faithful_gold = train_faithful(c, a.extra_faithful)
    attention_args = (c, ROOT / "corpus/attention.gold.jsonl", corpus / "attention.silver.jsonl")
    attention = None
    if a.extra_attention:
        previous = json.loads((ROOT / "src/hunch.json").read_text())["heads"]["attention"]["model"]
        attention, attention_report, attention_gold = train_attention_ranked(*attention_args, a.extra_attention, previous)
        if attention is None:
            print("review-label ranking fails a gate, keeping the head trained on Opus's labels only")
            ranked_report = attention_report
    if attention is None:
        attention, attention_report, attention_gold = train_attention(*attention_args)
        if a.extra_attention:
            attention_report["rejected_review_candidate"] = ranked_report
    a.out.write_text(dump({"feature_schema": schema, "fingerprint": fp,
                           "heads": {"should_skip": skip, "faithful": faithful, "attention": attention}}))
    a.parity.write_text(parity(xg, {"should_skip": skip_gold, "faithful": faithful_gold, "attention": attention_gold}))
    a.report.write_text(json.dumps({"heads": {"should_skip": skip_report, "faithful": faithful_report,
                                              "attention": attention_report}},
                                   indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
