//! P13.2: change-shape signals about the changeset as a whole, as facts rather
//! than judgments. Per-hunk signals live on `hunks[].notes`; these describe the
//! change itself.
use ordo::model::Input;

fn notes(v: serde_json::Value) -> Vec<String> {
    ordo::run(serde_json::from_value::<Input>(v).unwrap()).notes
}

fn code_change() -> serde_json::Value {
    serde_json::json!({"path": "src/app.py",
        "old": "def f():\n    return 1\n", "new": "def f():\n    return 2\n"})
}

#[test]
fn code_without_a_test_is_reported() {
    let n = notes(serde_json::json!({ "changes": [code_change()] }));
    assert!(
        n.contains(&"code changed but no test touched".to_string()),
        "{n:?}"
    );
}

#[test]
fn a_touched_test_silences_it() {
    let n = notes(serde_json::json!({"changes": [code_change(),
        {"path": "tests/test_app.py",
         "old": "def test_f():\n    pass\n", "new": "def test_f():\n    assert 1\n"}]}));
    assert!(n.is_empty(), "{n:?}");
}

#[test]
fn prose_and_config_are_not_code() {
    // a docs or CI-config change says nothing about an untested code path
    for c in [
        serde_json::json!({"path": "README.md", "old": "# A\n", "new": "# B\n"}),
        serde_json::json!({"path": "ci.yml", "old": "a: 1\n", "new": "a: 2\n"}),
    ] {
        let n = notes(serde_json::json!({ "changes": [c.clone()] }));
        assert!(n.is_empty(), "{c}: {n:?}");
    }
}

#[test]
fn a_churning_path_is_named_with_its_count() {
    let (mut old, mut new) = (String::new(), String::new());
    for i in 0..14 {
        old.push_str(&format!("def f{i}():\n    return {i}\n\n"));
        new.push_str(&format!("def f{i}():\n    return {}\n\n", i + 100));
    }
    let n = notes(serde_json::json!({"changes": [
        {"path": "src/big.py", "old": old, "new": new}]}));
    assert!(
        n.contains(&"src/big.py: 14 hunks (high churn)".to_string()),
        "{n:?}"
    );
}

#[test]
fn a_quiet_file_earns_no_churn_note() {
    let n = notes(serde_json::json!({ "changes": [code_change()] }));
    assert!(!n.iter().any(|s| s.contains("high churn")), "{n:?}");
}

#[test]
fn the_review_pack_leads_with_them() {
    let out = ordo::run(
        serde_json::from_value::<Input>(serde_json::json!({ "changes": [code_change()] })).unwrap(),
    );
    let p = ordo::pack(&out);
    let (notes_at, order_at) = (p.find("## notes"), p.find("## reading order"));
    assert!(notes_at.is_some(), "{p}");
    assert!(
        notes_at < order_at,
        "notes must precede the reading order:\n{p}"
    );
    assert!(p.contains("- code changed but no test touched"), "{p}");
}
