//! css: a rule set is a definition named by its whole selector list, sigils
//! kept — that punctuation is what makes a css symbol unable to collide with a
//! code one in the cross-file union. A `--custom-property` and its `var(--x)`
//! are the one def→use pair a stylesheet has.
use ordo::model::{HunkOut, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    run(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .files
    .into_iter()
    .flat_map(|f| f.hunks)
    .collect()
}

#[test]
fn a_rule_set_is_named_by_its_whole_selector_list() {
    let hs = one(
        "a.css",
        ".btn {\n  color: red;\n}\n",
        ".btn, .btn-primary {\n  color: red;\n}\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.defines.contains(&".btn, .btn-primary".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_custom_property_links_to_the_var_that_reads_it() {
    let old = ":root {\n  --a: 1;\n}\n\n.filler { z-index: 0; }\n\n.btn {\n  color: red;\n}\n";
    let new =
        ":root {\n  --a: 1;\n  --brand: #0af;\n}\n\n.filler { z-index: 0; }\n\n.btn {\n  color: var(--brand);\n}\n";
    let out = run(serde_json::json!({
        "changes": [{ "path": "t.css", "old": old, "new": new }]
    }));
    assert!(
        out.edges.iter().any(|e| e.why.contains("--brand")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_selector_never_leaks_a_bare_identifier() {
    // `.card` wraps a plain `identifier`, which IDENT_KINDS matches — without
    // the guard a stylesheet seeds `card` into the union symbol table every
    // other file is ordered against, and draws an edge to a python function
    let out = run(serde_json::json!({
        "options": {"cross_file": true},
        "changes": [
            { "path": "a.css", "old": ".x { color: red; }\n", "new": ".card, .title { color: blue; }\n" },
            { "path": "b.py", "old": "def card():\n    pass\n", "new": "def card():\n    return 1\n" },
        ]
    }));
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(
        !hs.iter().any(|h| h.uses.contains(&"card".to_string())),
        "{hs:?}"
    );
    assert!(out.edges.is_empty(), "{:?}", out.edges);
}

#[test]
fn a_media_query_is_a_region_not_a_definition() {
    let hs = one(
        "a.css",
        "@media (min-width: 700px) {\n  .btn { padding: 4px; }\n}\n",
        "@media (min-width: 700px) {\n  .btn { padding: 12px; }\n}\n",
    );
    // the rule inside is the definition; the query is the container
    assert!(
        hs.iter().any(|h| h.enclosing.as_deref() == Some(".btn")
            || h.enclosing.as_deref() == Some("@media (min-width: 700px)")),
        "{hs:?}"
    );
}

#[test]
fn a_declaration_is_a_member_of_its_rule() {
    let hs = one(
        "a.css",
        ".btn {\n  color: red;\n  padding: 4px;\n}\n",
        ".btn {\n  color: blue;\n  padding: 4px;\n}\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.details.contains(&"changes color in .btn".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_declaration_edit_is_not_a_change_to_the_selector() {
    // a `rule_set` labels no body field; without the `block` fallback the
    // header is the whole rule and this reads as a change to `.btn` itself
    let hs = one(
        "a.css",
        ".btn {\n  color: red;\n}\n",
        ".btn {\n  color: blue;\n}\n",
    );
    assert!(hs.iter().any(|h| h.rationale == "edits .btn"), "{hs:?}");
}
