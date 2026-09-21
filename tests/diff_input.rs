//! P9 diff-input: L1 (`old` + `diff` → apply → full semantics), plus graceful
//! degradation when a diff can't be applied.
mod fixture;
use fixture::run_json;
use ordo::model::SymbolChange;

fn run_pretty(v: serde_json::Value) -> String {
    serde_json::to_string_pretty(&run_json(v)).unwrap()
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
    let out =
        run_json(serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "diff": diff }] }));
    assert_eq!(out.schema, 2);
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
    let out = run_json(serde_json::json!({
        "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }],
        "options": { "full_context": true }
    }));
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
    let out =
        run_json(serde_json::json!({ "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }] }));
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
    let out = run_json(serde_json::json!({
        "changes": [{ "path": "m.py", "diff": diff }],
        "options": { "full_context": true }
    }));
    assert!(
        out.files[0].hunks.iter().all(|h| h.enclosing.is_none()),
        "multi-hunk under flag → still positional"
    );
}

// ---- L3: degraded marker ----

#[test]
fn l3_partial_diff_sets_degraded() {
    // diff only, no flag → positional → degraded flag set
    let out =
        run_json(serde_json::json!({ "changes": [{ "path": "m.py", "diff": FULL_CTX_DIFF }] }));
    assert!(out.files[0].degraded, "positional diff → degraded = true");
}

#[test]
fn l3_full_input_not_degraded_and_omitted() {
    let out = run_json(
        serde_json::json!({ "changes": [{ "path": "m.py", "old": "a\n", "new": "b\n" }] }),
    );
    assert!(!out.files[0].degraded, "old/new → not degraded");
    assert!(
        !serde_json::to_string(&out).unwrap().contains("degraded"),
        "degraded omitted from JSON when false"
    );
}

#[test]
fn a_reconstructed_old_side_reaches_the_ledger() {
    // the rebuilt sides used to stay inside the hunk builder: the symbol stage
    // then saw a file with no old definitions at all, so every pre-existing one
    // was reported as a signature change and nothing was ever a body edit
    let old = "class A:\n    def drain(self):\n        return 1\n";
    let new = "class A:\n    def drain(self):\n        return 2\n";
    let diff =
        "@@ -1,3 +1,3 @@\n class A:\n     def drain(self):\n-        return 1\n+        return 2\n";
    let out = run_json(serde_json::json!({
        "changes": [{ "path": "a.py", "diff": diff }],
        "options": { "full_context": true }
    }));
    let row = out
        .ledger
        .iter()
        .find(|l| l.name == "A.drain")
        .expect("A.drain in the ledger");
    assert_eq!(row.change, SymbolChange::Body, "its header is untouched");

    // the same change sent as old/new must say exactly the same thing
    let by_content = run_json(serde_json::json!({
        "changes": [{ "path": "a.py", "old": old, "new": new }]
    }));
    let names = |o: &ordo::model::Output| -> Vec<(String, SymbolChange)> {
        o.ledger
            .iter()
            .map(|l| (l.name.clone(), l.change))
            .collect()
    };
    assert_eq!(names(&by_content), names(&out));
}
