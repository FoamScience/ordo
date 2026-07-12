//! P9 diff-input: L1 (`old` + `diff` → apply → full semantics), plus graceful
//! degradation when a diff can't be applied.
use ordo::model::Input;

fn run_pretty(v: serde_json::Value) -> String {
    let inp: Input = serde_json::from_value(v).unwrap();
    serde_json::to_string_pretty(&ordo::run(inp)).unwrap()
}

#[test]
fn l1_old_plus_diff_equals_old_new() {
    // append a second function; the added-def hunk must classify as a definition
    let old = "def f():\n    return 1\n";
    let new = "def f():\n    return 1\ndef g():\n    return 2\n";
    let diff = "@@ -2,1 +2,3 @@\n     return 1\n+def g():\n+    return 2\n";

    let a =
        run_pretty(serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "new": new }] }));
    let b = run_pretty(
        serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "diff": diff }] }),
    );

    assert_eq!(a, b, "L1 (old+diff) output must equal the old+new output");
    assert!(
        a.contains("\"category\": \"definition\""),
        "modified file got full semantics, not positional"
    );
    assert!(a.contains("\"g\""), "the added def's name is extracted");
}

#[test]
fn l1_unapplicable_diff_degrades_gracefully() {
    // context lines don't match old → apply fails → positional, must not panic
    let old = "def f():\n    return 1\n";
    let diff = "@@ -1,2 +1,2 @@\n nope\n-wrong\n+bad\n";
    let inp: Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "diff": diff }] }),
    )
    .unwrap();
    let out = ordo::run(inp);
    assert_eq!(out.schema, 1);
    assert!(
        out.files[0]
            .hunks
            .iter()
            .all(|h| h.enclosing.is_none() && h.defines.is_empty()),
        "unapplicable diff → positional (no semantics)"
    );
}

// ---- L2: opt-in full-context reconstruction ----

const FULL_CTX_DIFF: &str =
    "@@ -1,4 +1,4 @@\n def caller():\n     return helper()\n def helper():\n-    return 1\n+    return 2\n";

#[test]
fn l2_full_context_flag_gives_semantics() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }],
        "options": { "full_context": true }
    }))
    .unwrap();
    let out = ordo::run(inp);
    assert!(
        out.files[0]
            .hunks
            .iter()
            .any(|h| h.enclosing.as_deref() == Some("helper")),
        "full-context flag → modified hunk gets enclosing (full semantics): {:?}",
        out.files[0].hunks
    );
}

#[test]
fn l2_without_flag_stays_positional() {
    let inp: Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }] }),
    )
    .unwrap();
    let out = ordo::run(inp);
    assert!(
        out.files[0]
            .hunks
            .iter()
            .all(|h| h.enclosing.is_none() && h.defines.is_empty()),
        "no flag → positional (no reconstruction)"
    );
}

#[test]
fn l2_multi_hunk_with_flag_stays_positional() {
    // two hunks → not a single whole-file hunk → refuse even with the flag
    let diff = "@@ -1,1 +1,1 @@\n-a\n+A\n@@ -5,1 +5,1 @@\n-b\n+B\n";
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "m.py", "diff": diff }],
        "options": { "full_context": true }
    }))
    .unwrap();
    let out = ordo::run(inp);
    assert!(
        out.files[0].hunks.iter().all(|h| h.enclosing.is_none()),
        "multi-hunk under flag → still positional"
    );
}

// ---- L3: degraded marker ----

#[test]
fn l3_partial_diff_sets_degraded() {
    // diff only, no flag → positional → degraded flag set
    let inp: Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }] }),
    )
    .unwrap();
    let out = ordo::run(inp);
    assert!(out.files[0].degraded, "positional diff → degraded = true");
}

#[test]
fn l3_full_input_not_degraded_and_omitted() {
    let inp: Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.py", "old": "a\n", "new": "b\n" }] }),
    )
    .unwrap();
    let out = ordo::run(inp);
    assert!(!out.files[0].degraded, "old/new → not degraded");
    assert!(
        !serde_json::to_string(&out).unwrap().contains("degraded"),
        "degraded omitted from JSON when false"
    );
}
