//! P4: cross-file def→use. `helper` is defined in util.py but that file is
//! listed *second*; only a cross-file edge can pull its definition ahead of the
//! use in main.py (listed first).
mod fixture;
use fixture::run_json;
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
            "uses helper, added in util.py",
            "adds helper, used in main.py"
        ],
        "each side names the other file"
    );
}

#[test]
fn a_name_two_files_both_define_seeds_no_edge() {
    // nothing here says which `View` a use means, and naming one of them sends
    // the reviewer to the wrong class
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "a.H", "old": "// a\n",
              "new": "// a\nstruct A\n{\n    using View = int;\n    View at(int i) { return i; }\n};\n" },
            { "path": "b.H", "old": "// b\n",
              "new": "// b\nstruct B\n{\n    using View = long;\n    View at(int i) { return i; }\n};\n" }
        ]
    }));
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
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "grid.H", "old": "// g\n",
              "new": "// g\nstruct DonorGrid\n{\n    int key(int p) const { return p; }\n};\n" },
            { "path": "io.H", "old": "// i\n",
              "new": "// i\nvoid read()\n{\n    const char* key = lookup();\n    open(key);\n}\n" }
        ]
    }));
    assert!(
        !out.edges.iter().any(|e| e.why.contains("key")),
        "{:?}",
        out.edges
    );
    let rats: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .map(|h| h.rationale.as_str())
        .collect();
    assert!(
        !rats.iter().any(|r| r.contains("uses key, defined in")),
        "the rationale must not claim what the graph refused: {rats:?}"
    );
}

#[test]
fn a_file_scope_definition_still_reaches_another_file() {
    // the rule narrows guesses, it does not switch cross-file edges off
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
            { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
        ]
    }));
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
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" },
            { "path": "main.py", "old": "# m\n",
              "new": "# m\nclass Runner:\n    def go(self):\n        return helper()\n" }
        ]
    }));
    assert!(
        out.edges.iter().any(|e| e.why.contains("helper")),
        "{:?}",
        out.edges
    );
}

/// An alias is the name this file has; the definition keeps the name its own
/// file gave it. Matching the two on the bare text means an aliased import is
/// invisible to the graph — the identical change without `as h` gets an edge.
#[test]
fn an_aliased_import_still_reaches_its_definition() {
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "lib.py", "old": "def helper(x):\n    return x\n",
              "new": "def helper(x, y):\n    return x + y\n" },
            { "path": "use.py",
              "old": "from lib import helper as h\n\ndef run():\n    return h(1)\n",
              "new": "from lib import helper as h\n\ndef run():\n    return h(1, 2)\n" }
        ]
    }));
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
}

/// The import says which file answers for a name. Without reading it, any
/// changed file defining the same name is fair game, and the rationale asserts
/// a provenance the source contradicts.
#[test]
fn a_definer_the_import_does_not_name_is_not_the_definition() {
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "one.py", "old": "def save(x):\n    return x\n",
              "new": "def save(x, y):\n    return x + y\n" },
            { "path": "caller.py",
              "old": "from two import save\n\ndef go():\n    return save(1)\n",
              "new": "from two import save\n\ndef go():\n    return save(1, 2)\n" }
        ]
    }));
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    let rats: Vec<&String> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| &h.rationale))
        .collect();
    assert!(
        !rats
            .iter()
            .any(|r| r.contains("one.py") || r.contains("caller.py")),
        "neither file may claim the other: {rats:?}"
    );
}

/// Two files defining one name is ambiguous only until something disambiguates
/// it. The import does, so the edge the ambiguity rule used to suppress is
/// exactly the one that should be drawn.
#[test]
fn an_import_disambiguates_a_name_two_files_define() {
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "one.py", "old": "def save(x):\n    return x\n",
              "new": "def save(x, y):\n    return x + y\n" },
            { "path": "two.py", "old": "def save(x):\n    return x\n",
              "new": "def save(x, y):\n    return x - y\n" },
            { "path": "caller.py",
              "old": "from one import save\n\ndef go():\n    return save(1)\n",
              "new": "from one import save\n\ndef go():\n    return save(1, 2)\n" }
        ]
    }));
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
    let rats: Vec<&String> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| &h.rationale))
        .collect();
    assert!(
        rats.iter()
            .any(|r| r.contains("uses save, defined in one.py")),
        "{rats:?}"
    );
    // and the file the import does not name stays out of it
    assert_eq!(
        rats.iter().filter(|r| r.contains("caller.py")).count(),
        1,
        "only the imported-from file may claim the use: {rats:?}"
    );
}

/// The `alias`/`name` field pairing is a convention the grammars share, so the
/// same fix has to hold outside python — javascript spells it
/// `import { helper as h }`, rust `use path::helper as h`.
#[test]
fn an_alias_reaches_its_definition_in_every_language_that_spells_one() {
    let cases = [
        (
            "lib.js",
            "export function helper(x) { return x; }\n",
            "export function helper(x, y) { return x + y; }\n",
            "use.js",
            "import { helper as h } from './lib';\n\nfunction run() { return h(1); }\n",
            "import { helper as h } from './lib';\n\nfunction run() { return h(1, 2); }\n",
        ),
        (
            "lib.rs",
            "pub fn helper(x: i32) -> i32 { x }\n",
            "pub fn helper(x: i32, y: i32) -> i32 { x + y }\n",
            "main.rs",
            "use crate::lib::helper as h;\n\nfn run() -> i32 { h(1) }\n",
            "use crate::lib::helper as h;\n\nfn run() -> i32 { h(1, 2) }\n",
        ),
    ];
    for (dp, do_, dn, up, uo, un) in cases {
        let out = run_json(serde_json::json!({
            "changes": [
                { "path": dp, "old": do_, "new": dn },
                { "path": up, "old": uo, "new": un }
            ]
        }));
        assert_eq!(out.edges.len(), 1, "{dp}: {:?}", out.edges);
    }
}

/// `from . import helper` names the package, not a module. Reading it as one
/// leaves a module with no file name in it, which matches nothing and drops
/// every edge the name match would have found.
#[test]
fn a_package_relative_import_names_no_module_and_blocks_nothing() {
    for imp in ["from . import helper", "from .. import helper"] {
        let out = run_json(serde_json::json!({
            "changes": [
                { "path": "pkg/lib.py", "old": "def helper(x):\n    return x\n",
                  "new": "def helper(x, y):\n    return x + y\n" },
                { "path": "pkg/use.py",
                  "old": format!("{imp}\n\ndef run():\n    return helper(1)\n"),
                  "new": format!("{imp}\n\ndef run():\n    return helper(1, 2)\n") }
            ]
        }));
        assert_eq!(out.edges.len(), 1, "`{imp}`: {:?}", out.edges);
    }
}

/// tasks-3uv.14: `element.angle` names a field of whatever `element` is. A
/// top-level `angle` in another changed file is a namesake, and taking it for
/// the definition bound two unrelated commits into one cluster.
#[test]
fn a_property_access_does_not_reach_a_namesake_in_another_file() {
    let out = run_json(serde_json::json!({ "changes": [
        { "path": "math.ts", "old": "export function other() { return 1; }\n",
          "new": "export function other() { return 1; }\nexport function angle(a: number) { return a; }\n" },
        { "path": "render.ts", "old": "export const draw = (element: any) => {\n  return 0;\n};\n",
          "new": "export const draw = (element: any) => {\n  return element.angle + 1;\n};\n" }
    ]}));
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    assert_eq!(out.clusters.len(), 2, "{:?}", out.clusters);
    let r: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| &f.hunks)
        .map(|h| h.rationale.as_str())
        .collect();
    assert!(r.iter().all(|x| !x.contains("render.ts")), "{r:?}");
}

/// …and a bare mention of the same name still resolves: one plain spelling
/// anywhere in the group is enough.
#[test]
fn a_bare_mention_still_reaches_the_other_file() {
    let out = run_json(serde_json::json!({ "changes": [
        { "path": "math.ts", "old": "export function other() { return 1; }\n",
          "new": "export function other() { return 1; }\nexport function angle(a: number) { return a; }\n" },
        { "path": "render.ts", "old": "export const draw = () => {\n  return 0;\n};\n",
          "new": "export const draw = () => {\n  return angle(1);\n};\n" }
    ]}));
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: angle"], "{why:?}");
}

/// tasks-3uv.14: a doc's code fence keeps its edge — it puts the doc after the
/// code it documents — but an illustration is not participation in the change,
/// so it must not pull the doc into the code's cluster. One README mentioning
/// `Index` merged two unrelated ripgrep commits.
#[test]
fn a_doc_fence_orders_the_doc_without_joining_its_cluster() {
    let out = run_json(serde_json::json!({ "changes": [
        { "path": "config.py", "old": "def parse_cfg(p):\n    return 1\n",
          "new": "def parse_cfg(p, strict=False):\n    return 1\n" },
        { "path": "README.md", "old": "# Usage\n\n```python\nx = old_helper(1)\n```\n",
          "new": "# Usage\n\n```python\nx = parse_cfg(\"a.toml\", strict=True)\n```\n" }
    ]}));
    assert_eq!(
        out.edges.len(),
        1,
        "the ordering edge stays: {:?}",
        out.edges
    );
    assert_eq!(out.clusters.len(), 2, "{:?}", out.clusters);
}

fn two(a: (&str, &str), b: (&str, &str)) -> ordo::model::Output {
    run_json(serde_json::json!({ "changes": [
        { "path": a.0, "old": "const x0 = 0;\n", "new": format!("const x0 = 0;\n{}", a.1) },
        { "path": b.0, "old": "const y0 = 0;\n", "new": format!("const y0 = 0;\n{}", b.1) }
    ]}))
}

/// tasks-3uv.37: execa 4b2e316e. `duplex` is a member of an object literal,
/// reached only as `addProperties.duplex`; a test that says `duplex` means
/// something of its own, not the member.
#[test]
fn an_object_member_is_not_a_definition_for_another_file() {
    let out = two(
        (
            "lib/handle.js",
            "const addProperties = {\n\tduplex: ({value}) => ({stream: value}),\n\twebTransform({value}) {\n\t\treturn {stream: value};\n\t},\n};\n",
        ),
        ("test/duplex.js", "function f(duplex) {}\nf(duplex);\n"),
    );
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    let handle = out
        .files
        .iter()
        .find(|f| f.path == "lib/handle.js")
        .unwrap();
    let scopes: Vec<_> = handle.hunks[0]
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.scope.as_deref()))
        .collect();
    assert!(
        scopes.contains(&("duplex", Some("addProperties"))),
        "{scopes:?}"
    );
}

/// …but an exported object's members are what other files import, so they
/// stay at file scope and still reach a bare use.
#[test]
fn an_exported_object_member_still_reaches_another_file() {
    let out = two(
        ("lib/api.js", "module.exports = {\n\tload: () => 1,\n};\n"),
        ("main.js", "const y = load();\n"),
    );
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: load"], "{why:?}");
}

/// tasks-3uv.37: execa da7aec7a. `const writable = new Writable(…)` inside a
/// test is that test's own variable; an export called `writable` in lib/ is a
/// namesake.
#[test]
fn a_file_that_binds_the_name_itself_does_not_reach_out() {
    let out = two(
        ("lib/early.js", "export const writable = () => 1;\n"),
        (
            "test/lines.js",
            "const run = async () => {\n\tconst writable = make();\n\tawait go(writable);\n};\n",
        ),
    );
    assert!(out.edges.is_empty(), "{:?}", out.edges);
    assert_eq!(out.clusters.len(), 2, "{:?}", out.clusters);
}

fn cpp(a: (&str, &str), b: (&str, &str)) -> ordo::model::Output {
    run_json(serde_json::json!({ "changes": [
        { "path": a.0, "old": "int x0;\n", "new": format!("int x0;\n{}", a.1) },
        { "path": b.0, "old": "int y0;\n", "new": format!("int y0;\n{}", b.1) }
    ]}))
}

/// A header's prototype is a top-level `declaration`, so it looks like the
/// header binding the name itself; in c/c++ it is the link to the definition.
#[test]
fn a_header_prototype_still_reaches_its_definition() {
    let out = cpp(
        (
            "toSubFieldTemplates.C",
            "template<class T>\nint Foam::toSubField(const T& a)\n{\n    return 1;\n}\n",
        ),
        (
            "toSubField.H",
            "template<class T>\nint toSubField(const T& a);\n",
        ),
    );
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: toSubField"], "{why:?}");
}

/// `X<T>::size` is a member of X and scoped like one; `Foam::toSubField` may
/// be a namespace function, so it stays at file scope and keeps its callers.
#[test]
fn only_a_class_qualifier_scopes_an_out_of_class_definition() {
    let member = cpp(
        (
            "Field.C",
            "template<class T>\nint Foam::Field<T>::size() const\n{\n    return 1;\n}\n",
        ),
        ("user.C", "int f()\n{\n    return size();\n}\n"),
    );
    assert!(member.edges.is_empty(), "{:?}", member.edges);
    let free = cpp(
        (
            "sub.C",
            "int Foam::toSubField(int a)\n{\n    return a;\n}\n",
        ),
        ("user.C", "int f()\n{\n    return toSubField(1);\n}\n"),
    );
    let why: Vec<&str> = free.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: toSubField"], "{why:?}");
}

fn edges_of(out: &ordo::model::Output) -> Vec<&str> {
    out.edges.iter().map(|e| e.why.as_str()).collect()
}

/// `util.Trim` in go and `ns.helper()` after `import * as ns` are qualified by
/// an import: the name is that package's own, not a field of some object.
#[test]
fn a_call_qualified_by_an_import_still_reaches_its_definition() {
    let go = run_json(serde_json::json!({ "changes": [
        { "path": "util/strs.go", "old": "package util\n",
          "new": "package util\n\nfunc Trim(s string) string {\n\treturn s\n}\n" },
        { "path": "main.go", "old": "package main\n\nimport \"example.com/app/util\"\n\nfunc a() {}\n",
          "new": "package main\n\nimport \"example.com/app/util\"\n\nfunc a() {\n\tutil.Trim(\"x\")\n}\n" }
    ]}));
    assert_eq!(edges_of(&go), vec!["def→use: Trim"]);
    let js = two(
        ("lib/helper.js", "export function helper() { return 1; }\n"),
        (
            "main.js",
            "import * as ns from './lib/helper.js';\nconst y = ns.helper();\n",
        ),
    );
    assert_eq!(edges_of(&js), vec!["def→use: helper"]);
    // a default import is an object: its member is not the package's own
    let obj = two(
        ("lib/port.js", "export function port() { return 1; }\n"),
        (
            "main.js",
            "import cfg from './config.js';\nconst y = cfg.port;\n",
        ),
    );
    assert!(edges_of(&obj).is_empty(), "{:?}", obj.edges);
}

/// A local of the same name in another function does not shadow a go
/// same-package call: only a binding the use can see stays local.
#[test]
fn a_local_elsewhere_in_the_file_does_not_block_a_same_package_call() {
    let pad: String = (0..6)
        .map(|i| format!("func f{i}() int {{\n\treturn {i}0\n}}\n\n"))
        .collect();
    let old = format!("package pkg\n\nfunc other() {{\n\tx := 1\n\t_ = x\n}}\n\n{pad}func use() int {{\n\treturn 0\n}}\n");
    let new = old
        .replace("x := 1\n\t_ = x", "s := 1\n\t_ = s")
        .replace("return 0\n}\n", "return s()\n}\n");
    let out = run_json(serde_json::json!({ "changes": [
        { "path": "pkg/a.go", "old": "package pkg\n", "new": "package pkg\n\nfunc s() int {\n\treturn 1\n}\n" },
        { "path": "pkg/b.go", "old": old, "new": new }
    ]}));
    assert!(edges_of(&out).contains(&"def→use: s"), "{:?}", out.edges);
}

/// `module.exports.api = {…}` is an export like `exports.api = {…}`: its
/// members stay at file scope.
#[test]
fn a_nested_module_exports_object_stays_at_file_scope() {
    let out = two(
        (
            "lib/api.js",
            "module.exports.api = {\n\tload: () => 1,\n};\n",
        ),
        ("main.js", "const y = load();\n"),
    );
    assert_eq!(edges_of(&out), vec!["def→use: load"]);
}
