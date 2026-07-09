//! P10 rationale-pattern tests.
use ordo::model::Input;

fn rationales(v: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().map(|h| h.rationale))
        .collect()
}

#[test]
fn p3_adds_new_def_vs_edits_existing_body() {
    // a.py: f already exists, only its body changes → "edits f"
    // b.py: g is brand new → "adds g"
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def f():\n    x = 1\n    return x\n", "new": "def f():\n    x = 2\n    return x\n" },
            { "path": "b.py", "old": "", "new": "def g():\n    return 3\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("edits f")),
        "existing body edit → edits: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.starts_with("adds g")),
        "new definition → adds: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("adds f")),
        "a pre-existing def must not be called 'adds': {rats:?}"
    );
}

#[test]
fn p4_signature_and_type_changes() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def f(a):\n    return a\n", "new": "def f(a, b):\n    return a\n" },
            { "path": "b.py", "old": "class C:\n    x = 1\n", "new": "class C(Base):\n    x = 1\n" },
            { "path": "c.py", "old": "", "new": "class D:\n    pass\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("changes signature of f")),
        "existing fn header change: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("changes type C")),
        "existing type header change: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("adds type D")),
        "new type: {rats:?}"
    );
}

#[test]
fn p5_add_import() {
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": "# x\n", "new": "import os\n# x\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("adds import os")),
        "new import: {rats:?}"
    );
}

#[test]
fn p6_test_links_to_code() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "tests/test_a.py", "old": "import a\n", "new": "import a\nassert a.helper()\n" },
            { "path": "a.py", "old": "", "new": "def helper():\n    return 1\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r == "tests helper (a.py)"),
        "test file links to code: {rats:?}"
    );
}

#[test]
fn p7_rename() {
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": "def foo():\n    return 1\n", "new": "def bar():\n    return 1\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r == "renames foo → bar"),
        "1:1 def rename: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("adds bar")),
        "a rename is not an add: {rats:?}"
    );
}

#[test]
fn p5_p7_removals() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def a():\n    return 1\ndef b():\n    return 2\n", "new": "def a():\n    return 1\n" },
            { "path": "b.py", "old": "import os\nimport sys\nx = 1\n", "new": "import os\nx = 1\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r == "removes b"),
        "deleted def: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r == "removes import sys"),
        "deleted import: {rats:?}"
    );
}
