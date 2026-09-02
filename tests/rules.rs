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

// ---------------------------------------------------------------- limits

fn one_with(path: &str, old: &str, new: &str, rules: serde_json::Value) -> Output {
    run(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }],
        "options": { "rules": rules }
    }))
}

#[test]
fn max_params_fires_only_past_the_limit() {
    let rules = serde_json::json!([{ "name": "few-args", "when": { "max_params": 3 }, "note": "too many" }]);
    let over = one_with(
        "a.py",
        "x = 1\n",
        "x = 1\ndef f(a, b, c, d):\n    return a\n",
        rules.clone(),
    );
    assert_eq!(hits(&over, "a.py"), vec!["note:few-args"]);
    let at = one_with(
        "a.py",
        "x = 1\n",
        "x = 1\ndef f(a, b, c):\n    return a\n",
        rules,
    );
    assert!(hits(&at, "a.py").is_empty());
}

#[test]
fn max_lines_measures_the_definition_the_hunk_starts() {
    let rules =
        serde_json::json!([{ "name": "short-fns", "when": { "max_lines": 3 }, "note": "long" }]);
    let long = one_with(
        "a.py",
        "x = 1\n",
        "x = 1\ndef f():\n    a = 1\n    b = 2\n    c = 3\n    return a\n",
        rules.clone(),
    );
    assert_eq!(hits(&long, "a.py"), vec!["note:short-fns"]);
    let short = one_with("a.py", "x = 1\n", "x = 1\ndef f():\n    return 1\n", rules);
    assert!(hits(&short, "a.py").is_empty());
}

#[test]
fn max_nesting_counts_control_flow_not_definitions() {
    let rules =
        serde_json::json!([{ "name": "flat", "when": { "max_nesting": 2 }, "warn": "deep" }]);
    let deep = one_with(
        "a.py",
        "def f(x):\n    return x\n",
        "def f(x):\n    if x:\n        for i in x:\n            if i:\n                return i\n    return x\n",
        rules.clone(),
    );
    assert_eq!(hits(&deep, "a.py"), vec!["warn:flat"]);
    let ok = one_with(
        "a.py",
        "def f(x):\n    return x\n",
        "def f(x):\n    if x:\n        return 1\n    return x\n",
        rules,
    );
    assert!(hits(&ok, "a.py").is_empty());
}

#[test]
fn max_file_lines_fires_only_when_this_change_crosses_it() {
    let rules = serde_json::json!([{ "name": "file-size", "when": { "max_file_lines": 4 }, "note": "big file" }]);
    let crossing = one_with(
        "a.py",
        "a = 1\nb = 2\n",
        "a = 1\nb = 2\nc = 3\nd = 4\ne = 5\n",
        rules.clone(),
    );
    assert_eq!(hits(&crossing, "a.py"), vec!["note:file-size"]);
    // already over: a one-line edit in a 6-line file is not the moment
    let already = one_with(
        "a.py",
        "a = 1\nb = 2\nc = 3\nd = 4\ne = 5\nf = 6\n",
        "a = 9\nb = 2\nc = 3\nd = 4\ne = 5\nf = 6\n",
        rules,
    );
    assert!(hits(&already, "a.py").is_empty());
}

// ---------------------------------------------------------------- path-not

#[test]
fn path_not_excludes_third_party_code() {
    let rules = serde_json::json!([{ "name": "ours-only", "when": { "path_not": "third_party/**" }, "note": "ours" }]);
    let out = run(serde_json::json!({
        "changes": [
            { "path": "src/a.py", "old": "x = 1\n", "new": "x = 2\n" },
            { "path": "third_party/b.py", "old": "x = 1\n", "new": "x = 2\n" },
        ],
        "options": { "rules": rules }
    }));
    assert_eq!(hits(&out, "src/a.py"), vec!["note:ours-only"]);
    assert!(hits(&out, "third_party/b.py").is_empty());
}

// ---------------------------------------------------------------- recursion

#[test]
fn recursive_is_a_definition_that_calls_itself() {
    let rules = serde_json::json!([{ "name": "no-recursion", "when": { "recursive": true }, "warn": "recursion" }]);
    let rec = one_with(
        "a.c",
        "int keep;\n",
        "int keep;\nint fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }\n",
        rules.clone(),
    );
    assert_eq!(hits(&rec, "a.c"), vec!["warn:no-recursion"]);
    let plain = one_with(
        "a.c",
        "int keep;\n",
        "int keep;\nint twice(int n) { return n * 2; }\n",
        rules,
    );
    assert!(hits(&plain, "a.c").is_empty());
}

// ---------------------------------------------------------------- container members

#[test]
fn container_without_sees_the_class_the_hunk_defines_into() {
    let rules = serde_json::json!([{
        "name": "equals-needs-hashcode",
        "when": { "defines": "equals", "container_without": "hashCode" },
        "warn": "override hashCode too"
    }]);
    let old = "class P {\n    int x;\n}\n";
    let bad = one_with(
        "P.java",
        old,
        "class P {\n    int x;\n    public boolean equals(Object o) { return true; }\n}\n",
        rules.clone(),
    );
    assert_eq!(hits(&bad, "P.java"), vec!["warn:equals-needs-hashcode"]);
    let good = one_with(
        "P.java", old,
        "class P {\n    int x;\n    public boolean equals(Object o) { return true; }\n    public int hashCode() { return 1; }\n}\n",
        rules,
    );
    assert!(hits(&good, "P.java").is_empty());
}

// ---------------------------------------------------------------- introduced shapes

#[test]
fn kind_matches_a_node_the_hunk_introduces() {
    let rules = serde_json::json!([{ "name": "no-typedef", "when": { "lang": "cpp", "kind": "type_definition" }, "note": "use using" }]);
    let out = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\ntypedef int handle_t;\n",
        rules.clone(),
    );
    assert_eq!(hits(&out, "a.cpp"), vec!["note:no-typedef"]);
    let none = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nusing handle_t = int;\n",
        rules,
    );
    assert!(hits(&none, "a.cpp").is_empty());
}

#[test]
fn kind_accepts_a_list() {
    let rules = serde_json::json!([{ "name": "raw-loop", "when": { "kind": ["for_statement", "while_statement"] }, "note": "loop" }]);
    let out = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nvoid f() { while (1) { break; } }\n",
        rules,
    );
    assert_eq!(hits(&out, "a.cpp"), vec!["note:raw-loop"]);
}

#[test]
fn without_expresses_absence_of_a_named_child() {
    // `int x = 0;` carries a default_value child; `double y;` does not
    let rules = serde_json::json!([{
        "name": "uninit-member",
        "when": { "kind": "field_declaration", "without": "default_value" },
        "warn": "initialize"
    }]);
    let out = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nstruct S { int x = 0; };\n",
        rules.clone(),
    );
    assert!(hits(&out, "a.cpp").is_empty());
    let out = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nstruct S { double y; };\n",
        rules,
    );
    assert_eq!(hits(&out, "a.cpp"), vec!["warn:uninit-member"]);
}

#[test]
fn without_reads_keyword_tokens_too() {
    // `virtual` is an anonymous token — invisible to a query anchor, but a child
    let rules = serde_json::json!([{
        "name": "virtual-dtor",
        "when": { "kind": "declaration", "with": "function_declarator", "without": "virtual", "text": "~" },
        "warn": "make it virtual"
    }]);
    let plain = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nstruct A { ~A(); };\n",
        rules.clone(),
    );
    assert_eq!(hits(&plain, "a.cpp"), vec!["warn:virtual-dtor"]);
    let virt = one_with(
        "a.cpp",
        "int keep;\n",
        "int keep;\nstruct B { virtual ~B(); };\n",
        rules,
    );
    assert!(hits(&virt, "a.cpp").is_empty());
}

#[test]
fn text_not_filters_by_the_node_text() {
    let rules = serde_json::json!([{
        "name": "mutable-global",
        "when": { "lang": "python", "kind": "expression_statement", "text_not": "^[A-Z_]+ =" },
        "note": "global state"
    }]);
    let out = one_with(
        "a.py",
        "def f():\n    pass\n",
        "def f():\n    pass\ncounter = 0\n",
        rules.clone(),
    );
    assert_eq!(hits(&out, "a.py"), vec!["note:mutable-global"]);
    let constant = one_with(
        "a.py",
        "def f():\n    pass\n",
        "def f():\n    pass\nLIMIT = 0\n",
        rules,
    );
    assert!(hits(&constant, "a.py").is_empty());
}

#[test]
fn a_bad_regex_is_reported_not_ignored() {
    let rules = serde_json::json!([{ "name": "broken", "when": { "kind": "x", "text": "(" }, "note": "n" }]);
    let out = one_with("a.py", "x = 1\n", "x = 2\n", rules);
    let problems = &out.problems;
    assert!(
        problems
            .iter()
            .any(|p| p.contains("broken") && p.contains("regex")),
        "{problems:?}"
    );
}

// ---------------------------------------------------------------- imports that are paths

fn import_hit(path: &str, old: &str, new: &str, glob: &str) -> bool {
    let rules = serde_json::json!([{ "name": "imp", "when": { "imports": glob }, "note": "n" }]);
    !hits(&one_with(path, old, new, rules), path).is_empty()
}

#[test]
fn a_cpp_include_binds_its_path_so_an_imports_glob_can_see_it() {
    let (old, new) = ("#include <string>\n", "#include <string>\n#include <boost/algorithm/string.hpp>\n");
    assert!(import_hit("a.cpp", old, new, "boost/**"));
    assert!(!import_hit("a.cpp", old, new, "asio/**"));
    assert!(import_hit("a.c", "int x;\n", "#include \"util.h\"\nint x;\n", "util.h"));
    let out = one_with("a.cpp", old, new, serde_json::json!([]));
    assert_eq!(out.files[0].hunks[0].rationale, "adds import boost/algorithm/string.hpp");
}

#[test]
fn a_go_import_binds_the_package_name_code_uses() {
    let (old, new) = ("package m\n", "package m\n\nimport (\n\t\"fmt\"\n\tz \"go.uber.org/zap\"\n\t_ \"embed\"\n)\n");
    assert!(import_hit("a.go", old, new, "z"), "alias binds");
    assert!(import_hit("a.go", old, new, "fmt"), "bare path binds its last segment");
    assert!(!import_hit("a.go", old, new, "zap"), "an aliased import is known by its alias");
    assert!(!import_hit("a.go", old, new, "embed"), "a blank import binds nothing");
}
