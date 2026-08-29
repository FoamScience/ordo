//! Standing bounds on `rationale`, which is contractually a ONE-LINE string.
//!
//! Two separate regressions produced multi-hundred-character rationales by
//! emitting one fully-formed fragment per symbol and joining them: first the
//! uncapped name lists (2215 chars, 91 names), later the binding provenance
//! (831 chars, one `no uses in this file` phrase per constant). Each was fixed
//! in isolation, and neither fix was protected by a test that would catch the
//! same shape arriving through new wording.
//!
//! These assertions are shape-based, not snapshot-based: they say a rationale
//! stays short and never repeats a provenance phrase, whatever the wording.
use ordo::model::Input;

/// Generous next to real output (the widest observed across two corpora is
/// ~356, from legitimately distinct move/extract fragments) and far below
/// either regression.
const MAX_RATIONALE: usize = 240;

/// A provenance phrase describes where a group of symbols came from or went.
/// Repeating one within a single rationale means per-symbol fragments were
/// joined instead of grouped — the exact defect both regressions had.
const PROVENANCE: &[&str] = &[
    ", extracted from ",
    ", no uses in ",
    ", used at L",
    ", defined in ",
    ", used in ",
    ", moves ",
];

fn rationales(v: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().map(|h| h.rationale))
        .collect()
}

fn check(label: &str, rats: &[String]) {
    for r in rats {
        assert!(
            r.chars().count() <= MAX_RATIONALE,
            "{label}: rationale is {} chars, over the {MAX_RATIONALE} bound — \
             a name or fragment list is uncapped:\n  {r}",
            r.chars().count()
        );
        for phrase in PROVENANCE {
            let n = r.matches(phrase).count();
            assert!(
                n <= 1,
                "{label}: {phrase:?} appears {n}x in one rationale — per-symbol \
                 fragments were joined instead of grouped:\n  {r}",
            );
        }
        assert!(
            !r.contains('\n'),
            "{label}: rationale spans multiple lines:\n  {r}"
        );
    }
}

#[test]
fn a_whole_new_file_of_definitions_stays_short() {
    // the original 2215-char case: every symbol in a new file lands in one hunk
    let body: String = (0..40)
        .map(|i| format!("def fn_{i}(a, b):\n    return a + b\n\n"))
        .collect();
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "", "new": body }]
    }));
    check("new file, 40 defs", &rats);
}

#[test]
fn many_module_level_bindings_with_no_uses_stay_short() {
    // the 831-char case: one "no uses in this file" phrase per constant
    let body: String = (0..30)
        .map(|i| format!("CONST_NUMBER_{i} = {i}\n"))
        .collect();
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "consts.py", "old": "", "new": body }]
    }));
    check("30 unused module constants", &rats);
}

#[test]
fn many_used_bindings_stay_short() {
    // the 701-char case: one "used at L.." phrase per binding
    let mut body = String::new();
    for i in 0..25 {
        body.push_str(&format!("VALUE_NUMBER_{i} = {i}\n"));
    }
    body.push_str("TOTAL = [\n");
    for i in 0..25 {
        body.push_str(&format!("    VALUE_NUMBER_{i},\n"));
    }
    body.push_str("]\n");
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "vals.py", "old": "", "new": body }]
    }));
    check("25 used module constants", &rats);
}

#[test]
fn many_locals_in_one_function_stay_short() {
    let mut body = String::from("def run(spec):\n");
    for i in 0..25 {
        body.push_str(&format!("    local_number_{i} = spec.value\n"));
    }
    body.push_str("    return 0\n");
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "f.py", "old": "def run(spec):\n    return 0\n", "new": body }]
    }));
    check("25 unused locals", &rats);
}

#[test]
fn many_imports_stay_short() {
    let body: String = (0..30)
        .map(|i| format!("use std::collections::module_{i};\n"))
        .collect();
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "a.rs", "old": "", "new": format!("{body}fn go() {{}}\n") }]
    }));
    check("30 imports", &rats);
}

#[test]
fn many_rust_consts_stay_short() {
    let body: String = (0..30)
        .map(|i| format!("pub const LIMIT_NUMBER_{i}: usize = {i};\n"))
        .collect();
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "a.rs", "old": "", "new": body }]
    }));
    check("30 rust consts", &rats);
}

#[test]
fn many_markdown_sections_stay_short() {
    let body: String = (0..30)
        .map(|i| format!("## Section number {i}\n\nSome prose here.\n\n"))
        .collect();
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "doc.md", "old": "# Title\n", "new": format!("# Title\n\n{body}") }]
    }));
    check("30 markdown sections", &rats);
}

#[test]
fn a_mixed_change_stays_short() {
    // definitions, constants, locals and comments in one hunk — the shapes that
    // compose into a single rationale via "; "
    let new = "\
CONST_A = 1
CONST_B = 2


def alpha(x):
    # explain
    scratch_one = x
    scratch_two = x
    return scratch_one


def beta(y):
    return alpha(y)
";
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "m.py", "old": "", "new": new }]
    }));
    check("mixed change", &rats);
}

#[test]
fn recorded_golden_output_stays_within_bounds() {
    // the fixtures are small, so this guards recorded output rather than shape —
    // it fails if a regenerated golden ever bakes in an unbounded rationale
    for entry in std::fs::read_dir("tests/golden").expect("golden dir") {
        let dir = entry.expect("golden entry").path();
        let f = dir.join("expected.json");
        if !f.exists() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&f).expect("read golden"))
                .expect("parse golden");
        let rats: Vec<String> = v["files"]
            .as_array()
            .expect("files")
            .iter()
            .flat_map(|fl| fl["hunks"].as_array().expect("hunks"))
            .map(|h| h["rationale"].as_str().expect("rationale").to_string())
            .collect();
        check(&format!("golden {}", dir.display()), &rats);
    }
}

#[test]
fn many_long_names_across_several_fragments_stay_within_bounds() {
    // Found by raising click's corpus cap: each fragment was name-capped, but
    // three capped fragments joined still ran to 359 chars because python test
    // names run past 60 characters. The bound has to hold on the JOIN, not just
    // on each part.
    let long =
        |i: usize| format!("test_flag_group_competition_envvar_prefix_and_unset_default_map_{i}");
    let mut old = String::from("def helper():\n    return 1\n\n");
    let mut new = String::from("def helper():\n    return 2\n\n");
    for i in 0..12 {
        new.push_str(&format!("def {}():\n    return helper()\n\n", long(i)));
    }
    old.push_str("def existing_one_with_a_fairly_long_name_too():\n    return 0\n");
    new.push_str("def existing_one_with_a_fairly_long_name_too():\n    return 9\n");
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "tests/test_options.py", "old": old, "new": new }]
    }));
    check("12 long test names + edits", &rats);
}
