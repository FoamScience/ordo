//! `symbols` field tests: name + tree-sitter kind + scope identity for each
//! definition a hunk introduces (mirrors `defines`, minus imports).
use ordo::model::{Input, Symbol};

fn symbols(v: serde_json::Value) -> Vec<Symbol> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().flat_map(|h| h.symbols))
        .collect()
}

fn one(path: &str, old: &str, new: &str) -> Vec<Symbol> {
    symbols(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
}

#[test]
fn module_run_vs_method_run_are_distinct_symbols() {
    // the motivating case: same name, same kind, different scope
    let syms = one(
        "a.py",
        "",
        "def run():\n    pass\n\nclass A:\n    def run(self):\n        pass\n",
    );
    let module_run = syms
        .iter()
        .find(|s| s.name == "run" && s.scope.is_none())
        .expect("module-level run");
    let method_run = syms
        .iter()
        .find(|s| s.name == "run" && s.scope.as_deref() == Some("A"))
        .expect("A.run method");
    assert_eq!(module_run.kind, "function_definition");
    assert_eq!(method_run.kind, "function_definition");
    assert_ne!(
        (&module_run.kind, &module_run.scope),
        (&method_run.kind, &method_run.scope),
        "the two `run`s must be distinguishable by kind and/or scope"
    );
}

#[test]
fn symbol_scope_can_differ_from_hunk_enclosing() {
    // one hunk (a brand-new file) whose `enclosing` is the outer class A, but
    // a nested class B's method has its own deeper scope "A.B"
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "",
            "new": "class A:\n    class B:\n        def run(self):\n            pass\n" }]
    }))
    .unwrap();
    let out = ordo::run(inp);
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("A"));

    let run_sym = h
        .symbols
        .iter()
        .find(|s| s.name == "run")
        .expect("run method");
    assert_eq!(run_sym.scope.as_deref(), Some("A.B"));
    assert_ne!(
        run_sym.scope.as_deref(),
        h.enclosing.as_deref(),
        "a symbol's own scope is not always the hunk's `enclosing`"
    );

    let class_a = h.symbols.iter().find(|s| s.name == "A").expect("class A");
    assert_eq!(class_a.kind, "class_definition");
    assert_eq!(class_a.scope, None, "top-level def has null scope");

    let class_b = h.symbols.iter().find(|s| s.name == "B").expect("class B");
    assert_eq!(class_b.scope.as_deref(), Some("A"));
}

#[test]
fn imported_names_are_excluded_from_symbols() {
    let syms = one(
        "a.py",
        "",
        "import os\nfrom sys import path\ndef helper():\n    return os.getpid()\n",
    );
    assert!(
        syms.iter().all(|s| s.name != "os" && s.name != "path"),
        "imports must not appear in symbols: {syms:?}"
    );
    assert!(
        syms.iter()
            .any(|s| s.name == "helper" && s.kind == "function_definition"),
        "{syms:?}"
    );
}

#[test]
fn named_arrow_function_is_a_definition() {
    let syms = one("a.js", "", "export const run = () => {\n  return 1;\n};\n");
    assert!(
        syms.iter()
            .any(|s| s.name == "run" && s.kind == "arrow_function"),
        "{syms:?}"
    );
}

#[test]
fn named_function_expression_is_a_definition() {
    let syms = one("a.js", "", "const inner = function () {\n  return 1;\n};\n");
    assert!(
        syms.iter()
            .any(|s| s.name == "inner" && s.kind == "function_expression"),
        "{syms:?}"
    );
}

#[test]
fn inline_anonymous_callback_is_not_a_definition() {
    let syms = one(
        "a.js",
        "",
        "const arr = [1, 2, 3];\nconst doubled = arr.map(x => x * 2);\npromise.then(() => {});\n",
    );
    assert!(
        syms.iter().all(|s| s.kind != "arrow_function"),
        "inline callbacks must stay anonymous/transparent: {syms:?}"
    );
    assert!(
        syms.iter().all(|s| s.name != "x"),
        "an arrow's own bare parameter must never be picked up as its name: {syms:?}"
    );
}
