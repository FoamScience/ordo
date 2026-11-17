//! hunch's feature export: off unless asked, one named column per position,
//! and git history read only inside its window.
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
            // outside: older than the window, the change itself, later work
            commit(AT - 91 * DAY, "fix ancient bug", "src/m.py"),
            commit(AT, "this change", "src/m.py"),
            commit(AT + DAY, "bugfix after", "src/m.py"),
        ]
    })));
    for h in hunks(&out) {
        assert_eq!(col(h, "has_history"), 1.0);
        assert_eq!(col(h, "churn"), 3.0);
        assert_eq!(col(h, "fixes"), 2.0, "`prefix` is not a fix");
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
