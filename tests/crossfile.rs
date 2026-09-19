//! P4: cross-file def→use. `helper` is defined in util.py but that file is
//! listed *second*; only a cross-file edge can pull its definition ahead of the
//! use in main.py (listed first).
use ordo::model::Input;

fn input(cross_file: bool) -> Input {
    let j = serde_json::json!({
        "changes": [
            { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
            { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
        ],
        "options": { "strategy": "comprehension", "cross_file": cross_file }
    });
    serde_json::from_value(j).unwrap()
}

fn pos(out: &ordo::model::Output, path: &str) -> usize {
    out.order.iter().position(|o| o.path == path).unwrap()
}

#[test]
fn cross_file_orders_def_before_use() {
    let out = ordo::run(input(true));
    assert!(
        pos(&out, "util.py") < pos(&out, "main.py"),
        "with cross_file, util.py's helper definition must precede its use in main.py; order: {:?}",
        out.order
    );
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: helper"], "exactly one cross-file edge");
}

#[test]
fn cross_file_off_keeps_input_file_order() {
    let out = ordo::run(input(false));
    assert!(
        pos(&out, "main.py") < pos(&out, "util.py"),
        "without cross_file, files keep input order; order: {:?}",
        out.order
    );
    assert!(
        out.edges.is_empty(),
        "cross_file off means no edges at all, not merely none naming helper: {:?}",
        out.edges
    );
}

#[test]
fn cross_file_rationale_names_the_other_file() {
    let out = ordo::run(input(true));
    let rats: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.rationale.as_str()))
        .collect();
    // both sides, exactly: `any` would have passed with one of the two missing
    assert_eq!(
        rats,
        vec![
            "uses helper, defined in util.py",
            "adds helper, used in main.py"
        ],
        "each side names the other file"
    );
}

#[test]
fn a_name_two_files_both_define_seeds_no_edge() {
    // nothing here says which `View` a use means, and naming one of them sends
    // the reviewer to the wrong class
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "a.H", "old": "// a\n",
                  "new": "// a\nstruct A\n{\n    using View = int;\n    View at(int i) { return i; }\n};\n" },
                { "path": "b.H", "old": "// b\n",
                  "new": "// b\nstruct B\n{\n    using View = long;\n    View at(int i) { return i; }\n};\n" }
            ]
        }))
        .unwrap(),
    );
    assert!(
        !out.edges.iter().any(|e| e.why.contains("View")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_class_member_is_not_resolved_from_another_file() {
    // `key` here is a method of DonorGrid; the `key` in the other file is a
    // local of a different type. Matching them across files needs the imports
    // and qualifications the engine does not read, so it declines to guess.
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "grid.H", "old": "// g\n",
                  "new": "// g\nstruct DonorGrid\n{\n    int key(int p) const { return p; }\n};\n" },
                { "path": "io.H", "old": "// i\n",
                  "new": "// i\nvoid read()\n{\n    const char* key = lookup();\n    open(key);\n}\n" }
            ]
        }))
        .unwrap(),
    );
    assert!(
        !out.edges.iter().any(|e| e.why.contains("key")),
        "{:?}",
        out.edges
    );
    let rats: Vec<&str> = out.files.iter().flat_map(|f| f.hunks.iter()).map(|h| h.rationale.as_str()).collect();
    assert!(
        !rats.iter().any(|r| r.contains("uses key, defined in")),
        "the rationale must not claim what the graph refused: {rats:?}"
    );
}

#[test]
fn a_file_scope_definition_still_reaches_another_file() {
    // the rule narrows guesses, it does not switch cross-file edges off
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
                { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
            ]
        }))
        .unwrap(),
    );
    assert!(
        out.edges.iter().any(|e| e.why.contains("helper")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_use_side_scope_does_not_block_the_edge() {
    // it is the *definition's* scope that decides whether a name resolves
    // across files: a file-scope `helper` still reaches a use that happens to
    // sit inside a class
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" },
                { "path": "main.py", "old": "# m\n",
                  "new": "# m\nclass Runner:\n    def go(self):\n        return helper()\n" }
            ]
        }))
        .unwrap(),
    );
    assert!(
        out.edges.iter().any(|e| e.why.contains("helper")),
        "{:?}",
        out.edges
    );
}
