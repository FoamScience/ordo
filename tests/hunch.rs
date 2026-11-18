//! hunch's feature export: off unless asked, one named column per position,
//! and git history read only inside its window; its guess: off unless asked,
//! the trained library's own numbers, and nothing else in the output moved.
use ordo_diff::hunch;
use ordo_diff::model::{HunkOut, Output};
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

#[test]
fn the_guess_matches_the_trained_library() {
    // the trainer writes these: gold hunks' vectors and what LightGBM (or the
    // regression it shipped) predicted for each
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hunch-parity.json");
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(p).expect("tests/hunch-parity.json"))
            .expect("json");
    assert!(rows.len() > 100, "{} rows", rows.len());
    assert!(
        hunch::model_fits(),
        "src/hunch.json was trained on another feature schema"
    );
    let width = hunch::names().len();
    let worst = rows
        .iter()
        .map(|r| {
            let x: Vec<f64> = serde_json::from_value(r["x"].clone()).expect("x");
            assert_eq!(x.len(), width);
            (hunch::predict(&x).0 - r["p"].as_f64().expect("p")).abs()
        })
        .fold(0.0, f64::max);
    assert!(worst <= 1e-4, "off by {worst}");
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
        // asked for the guess only, not the training columns
        assert!(h.get("features").is_none());
    }
}

#[test]
fn the_guess_moves_nothing_else() {
    let strip = |mut v: serde_json::Value| {
        for h in v["files"][0]["hunks"].as_array_mut().unwrap() {
            h.as_object_mut().unwrap().remove("hunch");
        }
        v
    };
    let plain = serde_json::to_value(run(json!({ "changes": [change()] }))).unwrap();
    assert_eq!(strip(with_hunch(json!({}))), plain);
}

#[test]
fn contributions_and_bias_make_up_the_log_odds() {
    // two vectors that differ in one column the model splits on: the
    // difference in log-odds is all that column's, so the bias cancels
    let base = vec![0.0; hunch::names().len()];
    let mut noisy = base.clone();
    noisy[hunch::names().iter().position(|n| n == "noise").unwrap()] = 1.0;
    let logit = |p: f64| (p / (1.0 - p)).ln();
    let (pa, ca) = hunch::predict(&base);
    let (pb, cb) = hunch::predict(&noisy);
    let total = |c: &[f64]| c.iter().sum::<f64>();
    let moved = (logit(pb) - logit(pa)) - (total(&cb) - total(&ca));
    assert!(moved.abs() < 1e-6, "bias moved by {moved}");
    assert!(pb > pa, "the noise flag should raise P(skip): {pa} -> {pb}");
}
