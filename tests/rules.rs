//! Reviewing rules: the caller's own conventions, matched against the facts the
//! engine computes. Data in, deterministic annotations and ordering out.
use ordo::model::{Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn order_of(out: &Output) -> Vec<&str> {
    out.order.iter().map(|o| o.path.as_str()).collect()
}

fn hits(out: &Output, path: &str) -> Vec<String> {
    out.files
        .iter()
        .filter(|f| f.path == path)
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.rules.iter())
        .map(|r| format!("{}:{}", r.level, r.rule))
        .collect()
}

/// Two independent files, so nothing in the dependency graph constrains order.
fn two_files(rules: serde_json::Value) -> Output {
    run(serde_json::json!({
        "changes": [
            { "path": "src/app/ui.py", "old": "def a():\n    return 1\n", "new": "def a():\n    return 2\n" },
            { "path": "src/security/auth.py", "old": "def v():\n    return 1\n", "new": "def v():\n    return 2\n" },
        ],
        "options": { "rules": rules }
    }))
}

#[test]
fn a_path_rule_annotates_and_can_sort_earlier() {
    let out = two_files(serde_json::json!([
        { "name": "security-first", "when": { "path": "src/security/**" },
          "note": "security-sensitive path", "priority": 100 }
    ]));
    assert_eq!(
        order_of(&out),
        vec!["src/security/auth.py", "src/app/ui.py"]
    );
    assert_eq!(
        hits(&out, "src/security/auth.py"),
        vec!["note:security-first"]
    );
    assert!(hits(&out, "src/app/ui.py").is_empty());
}

#[test]
fn without_a_priority_the_order_is_the_engines_own() {
    let out = two_files(serde_json::json!([
        { "name": "security", "when": { "path": "src/security/**" }, "note": "n" }
    ]));
    // annotated, but file position still decides
    assert_eq!(
        order_of(&out),
        vec!["src/app/ui.py", "src/security/auth.py"]
    );
}

#[test]
fn priority_cannot_pull_a_use_ahead_of_its_definition() {
    // the guarantee that makes ordering influence safe to hand to a config file
    let out = run(serde_json::json!({
        "changes": [
            { "path": "main.py", "old": "x = 1\n", "new": "x = helper()\n" },
            { "path": "util.py", "old": "", "new": "def helper():\n    return 1\n" },
        ],
        "options": { "cross_file": true, "rules": [
            { "name": "main-first", "when": { "path": "main.py" }, "priority": 1000 }
        ]}
    }));
    assert_eq!(
        order_of(&out),
        vec!["util.py", "main.py"],
        "P2 outranks a preference"
    );
    assert_eq!(out.edges.len(), 1);
}

#[test]
fn a_rule_can_mark_a_hunk_skippable() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "vendor/lib.py", "old": "def h():\n    return 1\n",
                      "new": "def h():\n    return 2\n" }],
        "options": { "rules": [
            { "name": "vendored", "when": { "path": "vendor/**" }, "noise": true }
        ]}
    }));
    let h = &out.files[0].hunks[0];
    assert!(h.noise);
    // a rule that only sets noise still says so, or the hunk dims for no reason
    assert_eq!(h.rules[0].message, "marked skippable");
}

// ---- query rules: conventions about code shape ----

const PREFER_PATHLIB: &str = r#"((call function: (attribute
    object: (attribute object: (identifier) @mod attribute: (identifier) @sub)
    attribute: (identifier) @fn)) @call
 (#eq? @mod "os")
 (#eq? @sub "path"))"#;

fn pathlib_rule() -> serde_json::Value {
    serde_json::json!([{
        "name": "prefer-pathlib",
        "when": { "lang": "python", "query": PREFER_PATHLIB },
        "warn": "prefer pathlib.Path over os.path.*"
    }])
}

#[test]
fn a_query_rule_fires_on_the_shape_it_matches() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "u.py",
            "old": "import os\n\n\ndef p(a, b):\n    return a\n",
            "new": "import os\n\n\ndef p(a, b):\n    return os.path.join(a, b)\n" }],
        "options": { "rules": pathlib_rule() }
    }));
    assert_eq!(hits(&out, "u.py"), vec!["warn:prefer-pathlib"]);
}

#[test]
fn a_query_rule_is_a_review_signal_not_a_linter_backlog() {
    // `os.path.join` is already in the file, untouched; the change is elsewhere.
    // A linter would flag the old call; ordo must not, because this change did
    // not introduce it.
    let out = run(serde_json::json!({
        "changes": [{ "path": "u.py",
            "old": "import os\n\n\ndef old(a, b):\n    return os.path.join(a, b)\n\n\ndef new():\n    return 1\n",
            "new": "import os\n\n\ndef old(a, b):\n    return os.path.join(a, b)\n\n\ndef new():\n    return 2\n" }],
        "options": { "rules": pathlib_rule() }
    }));
    assert!(hits(&out, "u.py").is_empty(), "{:?}", hits(&out, "u.py"));
}

#[test]
fn a_query_rule_respects_its_language_condition() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "u.js", "old": "const a = 1;\n", "new": "const a = 2;\n" }],
        "options": { "rules": pathlib_rule() }
    }));
    assert!(hits(&out, "u.js").is_empty());
}

// ---- the other predicates ----

#[test]
fn predicates_match_the_facts_they_name() {
    let changes = serde_json::json!([
        { "path": "a.py", "old": "def keep():\n    return 1\n",
          "new": "def keep():\n    return 1\n\n\ndef added():\n    return 2\n" }
    ]);
    let one = |when: serde_json::Value| {
        run(serde_json::json!({
            "changes": changes,
            "options": { "rules": [{ "name": "r", "when": when, "note": "n" }] }
        }))
    };
    assert_eq!(
        hits(&one(serde_json::json!({ "lang": "python" })), "a.py"),
        vec!["note:r"]
    );
    assert!(hits(&one(serde_json::json!({ "lang": "rust" })), "a.py").is_empty());
    assert_eq!(
        hits(&one(serde_json::json!({ "defines": "added" })), "a.py"),
        vec!["note:r"]
    );
    assert!(hits(&one(serde_json::json!({ "defines": "missing" })), "a.py").is_empty());
    assert_eq!(
        hits(
            &one(serde_json::json!({ "category": "definition" })),
            "a.py"
        ),
        vec!["note:r"]
    );
    assert!(hits(&one(serde_json::json!({ "category": "import" })), "a.py").is_empty());
    assert_eq!(
        hits(&one(serde_json::json!({ "comment": false })), "a.py"),
        vec!["note:r"]
    );
    // no conditions at all: matches everything, which the rule's name should own
    assert_eq!(hits(&one(serde_json::json!({})), "a.py"), vec!["note:r"]);
}

#[test]
fn an_enclosing_kind_condition_reads_the_container() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "t.test.js",
            "old": "it(\"works\", () => {\n  ok(1);\n});\n",
            "new": "it(\"works\", () => {\n  ok(2);\n});\n" }],
        "options": { "rules": [
            { "name": "in-a-test", "when": { "enclosing_kind": "test" }, "note": "n" },
            { "name": "in-a-region", "when": { "enclosing_kind": "region" }, "note": "n" }
        ]}
    }));
    assert_eq!(hits(&out, "t.test.js"), vec!["note:in-a-test"]);
}

// ---- a rule that cannot work says so ----

#[test]
fn a_glob_that_does_not_compile_is_reported() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "x = 1\n", "new": "x = 2\n" }],
        "options": { "rules": [
            { "name": "broken", "when": { "path": "src/**/[" }, "note": "n" }
        ]}
    }));
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("broken") && p.contains("not a glob")),
        "{:?}",
        out.problems
    );
}

#[test]
fn a_query_that_does_not_compile_is_reported() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "x = 1\n", "new": "x = 2\n" }],
        "options": { "rules": [
            { "name": "bad-query", "when": { "query": "(call function: (nonexistent_node))" },
              "note": "n" }
        ]}
    }));
    assert!(
        out.problems.iter().any(|p| p.contains("bad-query")),
        "{:?}",
        out.problems
    );
}

#[test]
fn no_rules_means_no_problems_and_no_hits() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "x = 1\n", "new": "x = 2\n" }]
    }));
    assert!(out.problems.is_empty());
    assert!(out.files[0].hunks[0].rules.is_empty());
}

#[test]
fn a_query_for_one_language_is_not_reported_against_another() {
    // a Rust query legitimately fails to parse as markdown; complaining about
    // every language in the change would bury the real errors
    let rust_query = r#"((call_expression function: (field_expression
        field: (field_identifier) @m)) (#eq? @m "unwrap"))"#;
    let out = run(serde_json::json!({
        "changes": [
            { "path": "a.rs", "old": "fn f() { g(); }\n", "new": "fn f() { g().unwrap(); }\n" },
            { "path": "README.md", "old": "# T\n\nold\n", "new": "# T\n\nnew\n" },
        ],
        "options": { "rules": [
            { "name": "no-unwrap", "when": { "lang": "rust", "query": rust_query },
              "warn": "a panic takes down every consumer" }
        ]}
    }));
    assert!(out.problems.is_empty(), "{:?}", out.problems);
    assert_eq!(hits(&out, "a.rs"), vec!["warn:no-unwrap"]);
    assert!(hits(&out, "README.md").is_empty());
}

#[test]
fn a_language_less_query_that_parses_nowhere_is_still_reported() {
    // ...but a query that no grammar in the change accepts is broken, and
    // saying so is the point of reporting at all
    let out = run(serde_json::json!({
        "changes": [{ "path": "a.rs", "old": "fn f() {}\n", "new": "fn f() { g(); }\n" }],
        "options": { "rules": [
            { "name": "nonsense", "when": { "query": "(no_such_node) @x" }, "note": "n" }
        ]}
    }));
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("nonsense") && p.contains("any language")),
        "{:?}",
        out.problems
    );
}
