//! hunch's feature export: off unless asked, one named column per position,
//! and git history read only inside its window; its guess: off unless asked,
//! each head the trained library's own numbers, and nothing else in the
//! output moved but the noise flags the skip policy overrides and the
//! freed groups attention reorders, each only once its gate passed.
use ordo_diff::hunch::{self, Head, Policy};
use ordo_diff::model::{HunchChange, HunkOut, Output};
use serde_json::json;
mod fixture;
use fixture::run_json as run;

const DAY: i64 = 86_400;
const AT: i64 = 1_700_000_000;

fn change() -> serde_json::Value {
    json!({
        "path": "src/m.py",
        "old": "def f(xs):\n    return 0\n",
        "new": "def f(xs):\n    t = 0\n    for x in xs:\n        if x:\n            t += x\n    return t\n"
    })
}

fn with_features(history: Option<serde_json::Value>) -> Output {
    let mut input = json!({ "changes": [change()], "options": { "features": true } });
    if let Some(h) = history {
        input["history"] = h;
    }
    run(input)
}

fn hunks(out: &Output) -> Vec<&HunkOut> {
    out.files.iter().flat_map(|f| &f.hunks).collect()
}

fn col(h: &HunkOut, name: &str) -> f64 {
    let i = hunch::names()
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("no column {name}"));
    h.features.as_ref().expect("features")[i]
}

#[test]
fn features_are_off_unless_asked() {
    let out = run(json!({ "changes": [change()] }));
    assert!(out.feature_schema.is_none());
    assert!(hunks(&out).iter().all(|h| h.features.is_none()));
    let v = serde_json::to_value(&out).unwrap();
    assert!(v.get("feature_schema").is_none());
    assert!(v["files"][0]["hunks"][0].get("features").is_none());
}

#[test]
fn every_hunk_has_one_value_per_named_column() {
    let out = with_features(None);
    let schema = out.feature_schema.as_ref().expect("schema");
    assert_eq!(schema.version, hunch::VERSION);
    assert_eq!(schema.names, hunch::names());
    assert_eq!(schema.hash, hunch::schema_hash());
    assert_eq!(schema.hash.len(), 16);
    let mut unique = schema.names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), schema.names.len(), "duplicate column names");
    for h in hunks(&out) {
        assert_eq!(h.features.as_ref().unwrap().len(), schema.names.len());
    }
}

#[test]
fn one_hot_groups_set_exactly_one_column() {
    let out = with_features(None);
    let names = hunch::names();
    for h in hunks(&out) {
        for prefix in ["category_", "extent_", "enclosing_", "template_", "lang_"] {
            let set: Vec<&String> = names
                .iter()
                .filter(|n| n.starts_with(prefix) && col(h, n) == 1.0)
                .collect();
            assert_eq!(set.len(), 1, "{prefix}: {set:?}");
        }
        assert_eq!(col(h, "lang_python"), 1.0);
    }
}

#[test]
fn readability_deltas_reach_the_vector() {
    let out = with_features(None);
    let grew = hunks(&out)
        .iter()
        .map(|h| col(h, "delta_nesting"))
        .sum::<f64>();
    assert!(grew > 0.0, "nesting grew by {grew}");
}

#[test]
fn no_history_is_zeros_and_says_so() {
    let out = with_features(None);
    for h in hunks(&out) {
        assert_eq!(col(h, "has_history"), 0.0);
        assert_eq!(col(h, "churn"), 0.0);
        assert_eq!(col(h, "fixes"), 0.0);
    }
}

#[test]
fn history_counts_only_the_window_before_the_change() {
    let commit = |time: i64, subject: &str, path: &str| json!({ "time": time, "subject": subject, "paths": [path] });
    let out = with_features(Some(json!({
        "at": AT,
        "commits": [
            commit(AT - DAY, "Fix off-by-one in f", "src/m.py"),
            commit(AT - 10 * DAY, "Revert \"speed up f\"", "src/m.py"),
            commit(AT - 20 * DAY, "add prefix option", "src/m.py"),
            commit(AT - 30 * DAY, "docs", "README.md"),
            // outside: older than the window
            commit(AT - 91 * DAY, "fix ancient bug", "src/m.py"),
        ]
    })));
    for h in hunks(&out) {
        assert_eq!(col(h, "has_history"), 1.0);
        assert_eq!(col(h, "churn"), 3.0);
        assert_eq!(col(h, "fixes"), 2.0, "`prefix` is not a fix");
    }
}

#[test]
fn an_ancestor_committed_in_the_same_second_counts() {
    // a rebased series stamps parent and child alike; the caller sends only
    // ancestors, so the engine does not second-guess them by time
    let out = with_features(Some(json!({
        "at": AT,
        "commits": [{ "time": AT, "subject": "fix parent in the series", "paths": ["src/m.py"] }]
    })));
    for h in hunks(&out) {
        assert_eq!(col(h, "churn"), 1.0);
        assert_eq!(col(h, "fixes"), 1.0);
    }
}

#[test]
fn a_file_without_commits_still_has_history() {
    let out = with_features(Some(json!({ "at": AT, "commits": [] })));
    for h in hunks(&out) {
        assert_eq!(col(h, "has_history"), 1.0);
        assert_eq!(col(h, "churn"), 0.0);
    }
}

/// tests/fixtures/hunch's outputs, by file name, as the trainer reads them.
fn fixture_outputs() -> Vec<(String, Output)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hunch");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/fixtures/hunch")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {}", dir.display());
    paths
        .into_iter()
        .map(|p| {
            let input = serde_json::from_str(&std::fs::read_to_string(&p).expect("fixture"))
                .expect("fixture json");
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, run(input))
        })
        .collect()
}

#[test]
fn the_model_was_trained_on_this_engines_features() {
    // the schema catches a renamed column, the fingerprint one whose values
    // moved under the same name
    let outputs = fixture_outputs();
    let fp = hunch::fingerprint(outputs.iter().map(|(n, o)| (n.as_str(), o)));
    if let Some(why) = hunch::misfit(Some(&fp)) {
        panic!("{why}");
    }
}

#[test]
fn the_fixtures_cover_what_the_fingerprint_guards() {
    let outputs = fixture_outputs();
    let all: Vec<&HunkOut> = outputs.iter().flat_map(|(_, o)| hunks(o)).collect();
    let any = |name: &str| all.iter().any(|h| col(h, name) != 0.0);
    for name in [
        "noise",
        "comment",
        "test_path",
        "has_history",
        "churn",
        "fixes",
        "delta_nesting",
        "delta_cognitive",
        "catalog_note",
        "rule_note",
        "rule_warn",
        "rule_verdict",
        "lang_python",
        "lang_go",
        "lang_c",
        "lang_typescript",
        "rationale_clauses",
        "rationale_commas",
        "rationale_more",
        "rationale_cut",
        "rationale_used_at",
        "rationale_added_at",
        "rationale_details",
    ] {
        assert!(any(name), "no fixture hunk sets {name}");
    }
}

#[test]
fn every_head_matches_the_trained_library() {
    // the trainer writes these: gold hunks' vectors and what LightGBM (or the
    // regression it shipped) predicted for each, per head
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hunch-parity.json");
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(p).expect("tests/hunch-parity.json"))
            .expect("json");
    assert!(rows.len() > 100, "{} rows", rows.len());
    let width = hunch::names().len();
    for (head, key) in [
        (Head::Skip, "should_skip"),
        (Head::Faithful, "faithful"),
        (Head::Attention, "attention"),
    ] {
        let worst = rows
            .iter()
            .map(|r| {
                let x: Vec<f64> = serde_json::from_value(r["x"].clone()).expect("x");
                assert_eq!(x.len(), width);
                let want = r["p"][key].as_f64().unwrap_or_else(|| panic!("p.{key}"));
                (hunch::predict(head, &x).0 - want).abs()
            })
            .fold(0.0, f64::max);
        assert!(worst <= 1e-4, "{key} off by {worst}");
    }
}

fn with_hunch(extra: serde_json::Value) -> serde_json::Value {
    let mut input = json!({ "changes": [change()], "options": { "hunch": true } });
    if let serde_json::Value::Object(m) = extra {
        input["options"].as_object_mut().unwrap().extend(m);
    }
    serde_json::to_value(run(input)).unwrap()
}

#[test]
fn the_guess_is_off_unless_asked() {
    let v = serde_json::to_value(run(json!({ "changes": [change()] }))).unwrap();
    assert!(v["files"][0]["hunks"][0].get("hunch").is_none());
}

#[test]
fn the_guess_carries_a_probability_and_its_top_features() {
    let v = with_hunch(json!({}));
    assert!(v.get("problems").is_none(), "{}", v["problems"]);
    let names = hunch::names();
    for h in v["files"][0]["hunks"].as_array().unwrap() {
        let g = &h["hunch"];
        let p = g["noise"].as_f64().expect("noise");
        assert!((0.0..=1.0).contains(&p), "{p}");
        let top = g["top"].as_array().expect("top");
        assert!(!top.is_empty() && top.len() <= 3, "{top:?}");
        let sizes: Vec<f64> = top
            .iter()
            .map(|t| {
                assert!(names.contains(&t[0].as_str().unwrap().to_string()), "{t}");
                t[1].as_f64().unwrap().abs()
            })
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
        let f = g["faithful"].as_f64().expect("faithful");
        assert!((0.0..=1.0).contains(&f), "{f}");
        let top = g["faithful_top"].as_array().expect("faithful_top");
        assert!(top.len() <= 3, "{top:?}");
        let a = g["attention"].as_f64().expect("attention");
        assert!((0.0..=1.0).contains(&a), "{a}");
        let top = g["attention_top"].as_array().expect("attention_top");
        assert!(top.len() <= 3, "{top:?}");
        // asked for the guess only, not the training columns
        assert!(h.get("features").is_none());
    }
}

/// Past both of the shipped policy's thresholds: a new file that only
/// imports and runs top-level code, which the engine hides as import noise
/// (P(skip) about 0.01), and a comment reworded at the top level of one
/// document of a busy multi-document YAML file, which it shows (about 0.98,
/// against the hide threshold of 0.9).
fn policy_input(hunch: bool) -> serde_json::Value {
    let yaml = "name: lint\nimage: python:3.12\n# runs on every push\ncommand: ruff check .\n---\n\
                name: test\nimage: python:3.12\n# the slow suite runs nightly\ncommand: pytest -q\n";
    let commits: Vec<_> = (0..20)
        .map(|i| {
            let subject = if i < 6 { "fix: a crash" } else { "ci: tidy" };
            json!({ "time": AT - (i + 1) * DAY, "subject": subject, "paths": ["ci/jobs.yaml"] })
        })
        .collect();
    json!({
        "changes": [
            {
                "path": "test/fixtures/stdin-both.js",
                "old": null,
                "new": "#!/usr/bin/env node\nimport process from 'node:process';\n\
                        import {text} from 'node:stream/consumers';\n\n\
                        const input = await text(process.stdin);\n\
                        process.stdout.write(`${input}:stdout`);\n\
                        process.stderr.write(`${input}:stderr`);\n"
            },
            {
                "path": "ci/jobs.yaml",
                "old": yaml,
                "new": yaml.replace("runs nightly", "runs every night")
            }
        ],
        "history": { "at": AT, "commits": commits },
        "options": { "hunch": hunch }
    })
}

/// `with` stripped of hunch's guesses, each noise flag the policy moved put
/// back, so it reads as the output without `--hunch`; and the moves, in
/// hunk order. Its reading order (`order`, each hunk's `order_index`) goes
/// too, when attention may move it: `attention_keeps_every_edge` checks that.
fn unmoved(mut with: serde_json::Value) -> (serde_json::Value, Vec<Option<HunchChange>>) {
    let mut moves = vec![];
    if hunch::attention_orders() {
        with.as_object_mut().unwrap().remove("order");
    }
    for f in with["files"].as_array_mut().unwrap() {
        for h in f["hunks"].as_array_mut().unwrap() {
            let h = h.as_object_mut().unwrap();
            if hunch::attention_orders() {
                h.remove("order_index");
            }
            let changed = h.remove("hunch").and_then(|g| g.get("changed").cloned());
            let changed: Option<HunchChange> = changed.map(|c| match c.as_str() {
                Some("unhid") => HunchChange::Unhid,
                Some("hid") => HunchChange::Hid,
                other => panic!("changed: {other:?}"),
            });
            if let Some(c) = changed {
                let noise = h.get("noise").and_then(|n| n.as_bool()).unwrap_or(false);
                assert_eq!(noise, c == HunchChange::Hid, "{c:?} left noise {noise}");
                if c == HunchChange::Hid {
                    h.remove("noise");
                } else {
                    h.insert("noise".into(), json!(true));
                }
            }
            moves.push(changed);
        }
    }
    (with, moves)
}

/// The output without `--hunch`, less what `unmoved` drops from the guess.
fn plain(input: serde_json::Value) -> serde_json::Value {
    let mut v = serde_json::to_value(run(input)).unwrap();
    if hunch::attention_orders() {
        v.as_object_mut().unwrap().remove("order");
        for f in v["files"].as_array_mut().unwrap() {
            for h in f["hunks"].as_array_mut().unwrap() {
                h.as_object_mut().unwrap().remove("order_index");
            }
        }
    }
    v
}

#[test]
fn the_guess_moves_nothing_else() {
    let plain = plain(json!({ "changes": [change()] }));
    let (with, moves) = unmoved(with_hunch(json!({})));
    assert_eq!(with, plain);
    assert!(moves.iter().all(Option::is_none), "{moves:?}");
}

#[test]
fn the_policy_moves_only_noise_flags() {
    let plain = plain(policy_input(false));
    let guessed = serde_json::to_value(run(policy_input(true))).unwrap();
    let p: Vec<f64> = guessed["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["hunks"].as_array().unwrap())
        .map(|h| h["hunch"]["noise"].as_f64().expect("noise"))
        .collect();
    let (with, moves) = unmoved(guessed);
    // edges, groups and every other field stay the engine's
    assert_eq!(with, plain);
    let expected = match hunch::policy() {
        Some(_) => vec![Some(HunchChange::Unhid), Some(HunchChange::Hid)],
        None => vec![None, None],
    };
    assert_eq!(
        moves,
        expected,
        "P(skip) {p:?} against {:?}",
        hunch::policy()
    );
}

fn shipped_policy(head: &str) -> serde_json::Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hunch.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(p).expect("src/hunch.json")).expect("json");
    doc["heads"][head]["policy"].clone()
}

#[test]
fn the_policy_ships_only_when_its_gate_passed() {
    let policy = shipped_policy("should_skip");
    let shipped = (policy["passes"] == true).then(|| Policy {
        unhide: policy["unhide"].as_f64().expect("unhide"),
        hide: policy["hide"].as_f64().expect("hide"),
    });
    assert_eq!(hunch::policy(), shipped);
}

#[test]
fn attention_orders_only_once_its_gate_passed() {
    assert_eq!(
        shipped_policy("attention")["passes"] == true,
        hunch::attention_orders()
    );
}

/// Each hunk id's place in `out`'s reading order.
fn places(out: &Output) -> std::collections::HashMap<String, usize> {
    out.order
        .iter()
        .enumerate()
        .map(|(i, o)| (o.hunk.clone(), i))
        .collect()
}

#[test]
fn attention_keeps_every_edge() {
    // the order with `--hunch` reads every definition before its use wherever
    // the engine's own order does: attention only reorders freed groups
    let mut inputs = feature_inputs();
    inputs.push(policy_input(false));
    for mut input in inputs {
        input["options"]["hunch"] = json!(false);
        let without = run(input.clone());
        input["options"]["hunch"] = json!(true);
        let with = run(input);
        let (a, b) = (places(&without), places(&with));
        let mut sorted: Vec<&String> = b.keys().collect();
        sorted.sort();
        let mut want: Vec<&String> = a.keys().collect();
        want.sort();
        assert_eq!(sorted, want, "the same hunks");
        for e in &without.edges {
            if a[&e.from] < a[&e.to] {
                assert!(b[&e.from] < b[&e.to], "{} -> {} reversed", e.from, e.to);
            }
        }
    }
}

/// tests/fixtures/hunch's inputs and `attention_input`'s, each with features
/// asked for.
fn feature_inputs() -> Vec<serde_json::Value> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hunch");
    let mut inputs: Vec<serde_json::Value> = fixture_outputs()
        .into_iter()
        .map(|(name, _)| {
            serde_json::from_str(&std::fs::read_to_string(dir.join(name)).unwrap()).unwrap()
        })
        .collect();
    inputs.push(attention_input(false));
    for i in &mut inputs {
        i["options"]["features"] = json!(true);
    }
    inputs
}

#[test]
fn the_first_assembly_feeds_attention_the_same_features() {
    // attention is guessed on an assembly in file-position order; that is
    // only sound while no feature reads the reading order
    let vectors = |o: &Output| -> Vec<Vec<f64>> {
        hunks(o)
            .iter()
            .map(|h| h.features.clone().expect("features"))
            .collect()
    };
    let mut reordered = false;
    for mut input in feature_inputs() {
        input["options"]["hunch"] = json!(false);
        let without = run(input.clone());
        input["options"]["hunch"] = json!(true);
        let with = run(input);
        assert_eq!(vectors(&with), vectors(&without));
        reordered |= places(&with) != places(&without);
    }
    assert_eq!(reordered, hunch::attention_orders());
}

#[test]
fn attention_cannot_pull_a_use_ahead_of_its_definition() {
    // tests/rules.rs's "P2 outranks a preference", with hunch's in place of a
    // rule's: main.py's use draws the attention, util.py still comes first
    let out = run(json!({
        "changes": [
            { "path": "main.py", "old": "x = 1\n", "new": "x = helper()\n" },
            { "path": "util.py", "old": "", "new": "def helper():\n    return 1\n" },
        ],
        "options": { "cross_file": true, "hunch": true }
    }));
    let paths: Vec<&str> = out.order.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(paths, vec!["util.py", "main.py"]);
    assert_eq!(out.edges.len(), 1);
}

/// One file of two unrelated functions: the first only reworded in a
/// comment, the second given new branching logic, which a reviewer needs to
/// read far more closely.
fn attention_input(hunch: bool) -> serde_json::Value {
    let old = "def a():\n    # the first one\n    return 1\n\n\n\n\n\n\n\n\n\n\n\n\
               def b(xs):\n    return 0\n";
    let new = "def a():\n    # the very first one\n    return 1\n\n\n\n\n\n\n\n\n\n\n\n\
               def b(xs):\n    t = 0\n    for x in xs:\n        if x > 0 and x % 2:\n            \
               t += x\n        elif x < 0:\n            t -= x\n    return t\n";
    json!({ "changes": [{ "path": "src/m.py", "old": old, "new": new }],
            "options": { "hunch": hunch } })
}

#[test]
fn attention_reads_the_freed_group_that_needs_it_first() {
    let without = run(attention_input(false));
    let with = run(attention_input(true));
    let att: Vec<f64> = with.files[0]
        .hunks
        .iter()
        .map(|h| h.hunch.as_ref().unwrap().attention)
        .collect();
    assert_eq!(att.len(), 2, "two hunks");
    assert!(att[1] > att[0], "attention {att:?}");
    let first = |o: &Output| o.order[0].hunk.clone();
    assert_eq!(first(&without), with.files[0].hunks[0].id);
    let want = if hunch::attention_orders() { 1 } else { 0 };
    assert_eq!(
        first(&with),
        with.files[0].hunks[want].id,
        "attention {att:?}"
    );
}

#[test]
fn the_faithful_guess_stays_a_shadow() {
    // its doubt gate failed, so nothing reads P(faithful) yet; a pass is the
    // cue to give `--hunch` a doubt marker
    assert_eq!(shipped_policy("faithful")["passes"], false);
}

#[test]
fn the_policy_unhides_below_one_threshold_and_hides_at_the_other() {
    let p = Some(Policy {
        unhide: 0.05,
        hide: 0.9,
    });
    assert_eq!(hunch::decide(true, 0.01, p), Some(HunchChange::Unhid));
    assert_eq!(hunch::decide(true, 0.05, p), None);
    assert_eq!(hunch::decide(true, 0.99, p), None);
    assert_eq!(hunch::decide(false, 0.9, p), Some(HunchChange::Hid));
    assert_eq!(hunch::decide(false, 0.89, p), None);
    assert_eq!(hunch::decide(false, 0.01, p), None);
}

#[test]
fn a_model_that_failed_its_gate_stays_a_shadow() {
    for noise in [true, false] {
        for p in [0.0, 0.01, 0.5, 0.95, 1.0] {
            assert_eq!(hunch::decide(noise, p, None), None);
        }
    }
}

#[test]
fn contributions_and_bias_make_up_the_log_odds() {
    // two vectors that differ in one column the model splits on: the
    // difference in log-odds is all that column's, so the bias cancels
    let base = vec![0.0; hunch::names().len()];
    let mut noisy = base.clone();
    noisy[hunch::names().iter().position(|n| n == "noise").unwrap()] = 1.0;
    let logit = |p: f64| (p / (1.0 - p)).ln();
    let (pa, ca) = hunch::predict(Head::Skip, &base);
    let (pb, cb) = hunch::predict(Head::Skip, &noisy);
    let total = |c: &[f64]| c.iter().sum::<f64>();
    let moved = (logit(pb) - logit(pa)) - (total(&cb) - total(&ca));
    assert!(moved.abs() < 1e-6, "bias moved by {moved}");
    assert!(pb > pa, "the noise flag should raise P(skip): {pa} -> {pb}");
}
