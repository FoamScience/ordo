//! scss: css's shape plus nesting. A nested rule is named by the selector it
//! compiles to, a mixin and a function by their at-rule, and `$variables`
//! link to what reads them the way custom properties do.
mod fixture;
use fixture::{hunks as one, run_json as run};

#[test]
fn a_nested_rule_is_named_by_the_selector_it_compiles_to() {
    let old = ".menu {\n  .item {\n    color: red;\n  }\n  &:hover {\n    color: red;\n  }\n}\n";
    let new = ".menu {\n  .item {\n    color: blue;\n  }\n  &:hover {\n    color: blue;\n  }\n}\n";
    let hs = one("a.scss", old, new);
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(rats, vec!["edits .menu .item", "edits .menu:hover"], "{hs:?}");
}

#[test]
fn a_parent_selector_list_is_crossed_with_the_nested_one() {
    let old = ".a, .b {\n  &-x {\n    top: 0;\n  }\n}\n";
    let new = ".a, .b {\n  &-x {\n    top: 1px;\n  }\n}\n";
    let hs = one("a.scss", old, new);
    assert_eq!(hs[0].enclosing.as_deref(), Some(".a-x, .b-x"), "{hs:?}");
}

#[test]
fn a_rule_nested_in_a_mixin_is_named_under_it() {
    let old = "@mixin btn {\n  &:hover {\n    top: 0;\n  }\n}\n";
    let new = "@mixin btn {\n  &:hover {\n    top: 1px;\n  }\n}\n";
    let hs = one("a.scss", old, new);
    assert_eq!(hs[0].enclosing.as_deref(), Some("@mixin btn &:hover"), "{hs:?}");
    assert_eq!(hs[0].rationale, "changes 0 → 1px in @mixin btn &:hover");
}

#[test]
fn keyframes_nested_in_a_rule_are_not_scoped_by_it() {
    let old = ".a {\n  @keyframes spin {\n    from {\n      top: 0;\n    }\n  }\n}\n";
    let new = ".a {\n  @keyframes spin {\n    from {\n      top: 1px;\n    }\n  }\n}\n";
    let hs = one("a.scss", old, new);
    assert_eq!(hs[0].enclosing.as_deref(), Some("@keyframes spin"), "{hs:?}");
}

#[test]
fn an_include_links_to_its_mixin() {
    let old = "@mixin other {\n  top: 0;\n}\n\n.pad { top: 0; }\n\n.btn {\n  color: red;\n}\n";
    let new = "@mixin other {\n  top: 0;\n}\n\n@mixin btn($c) {\n  color: $c;\n}\n\n.pad { top: 0; }\n\n.btn {\n  @include btn(red);\n}\n";
    let out = run(serde_json::json!({
        "changes": [{ "path": "t.scss", "old": old, "new": new }]
    }));
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: @mixin btn"]);
    let hs: Vec<_> = out.files[0].hunks.iter().collect();
    assert!(hs.iter().any(|h| h.defines == vec!["@mixin btn".to_string()]), "{hs:?}");
}

#[test]
fn a_variable_links_to_what_reads_it() {
    let old = "$a: 1px;\n\n.pad { top: 0; }\n\n.btn {\n  color: red;\n}\n";
    let new = "$a: 1px;\n$gap: 4px;\n\n.pad { top: 0; }\n\n.btn {\n  margin: $gap;\n}\n";
    let out = run(serde_json::json!({
        "changes": [{ "path": "t.scss", "old": old, "new": new }]
    }));
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: $gap"]);
}

#[test]
fn a_mixin_name_never_leaks_a_bare_identifier() {
    let out = run(serde_json::json!({
        "options": {"cross_file": true},
        "changes": [
            { "path": "a.scss", "old": ".x { top: 0; }\n", "new": ".x { @include card; }\n" },
            { "path": "b.py", "old": "def card():\n    pass\n", "new": "def card():\n    return 1\n" },
        ]
    }));
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(!hs.iter().any(|h| h.uses.contains(&"card".to_string())), "{hs:?}");
    assert!(out.edges.is_empty(), "{:?}", out.edges);
}

#[test]
fn a_namespaced_include_is_not_read_as_its_module() {
    let hs = one(
        "a.scss",
        "@use \"m\";\n.a { top: 0; }\n",
        "@use \"m\";\n.a { @include m.btn(1); }\n",
    );
    assert!(hs.iter().all(|h| !h.uses.contains(&"@mixin m".to_string())), "{hs:?}");
}
