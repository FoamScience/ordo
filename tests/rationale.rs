//! P10 rationale-pattern tests.
use ordo::model::Input;

fn rationales(v: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().map(|h| h.rationale))
        .collect()
}

#[test]
fn p3_adds_new_def_vs_edits_existing_body() {
    // a.py: f already exists, only its body changes → "edits f"
    // b.py: g is brand new → "adds g"
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def f():\n    x = 1\n    return x\n", "new": "def f():\n    x = 2\n    return x\n" },
            { "path": "b.py", "old": "", "new": "def g():\n    return 3\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("edits f")),
        "existing body edit → edits: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.starts_with("adds g")),
        "new definition → adds: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("adds f")),
        "a pre-existing def must not be called 'adds': {rats:?}"
    );
}

#[test]
fn p4_signature_and_type_changes() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def f(a):\n    return a\n", "new": "def f(a, b):\n    return a\n" },
            { "path": "b.py", "old": "class C:\n    x = 1\n", "new": "class C(Base):\n    x = 1\n" },
            { "path": "c.py", "old": "", "new": "class D:\n    pass\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("changes signature of f")),
        "existing fn header change: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("changes type C")),
        "existing type header change: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("adds type D")),
        "new type: {rats:?}"
    );
}

#[test]
fn p5_an_added_import_is_named_and_marked_skippable() {
    // a pure-import add is noise, not nothing: it never leads the reading order
    // and seeds no edge, but a new dependency is worth seeing arrive
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": "# x\n", "new": "import os\n# x\n" } ]
    }));
    assert_eq!(rats, vec!["adds import os"]);
}

#[test]
fn p6_test_links_to_code() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "tests/test_a.py", "old": "import a\n", "new": "import a\nassert a.helper()\n" },
            { "path": "a.py", "old": "", "new": "def helper():\n    return 1\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r == "tests helper (a.py)"),
        "test file links to code: {rats:?}"
    );
}

#[test]
fn p7_rename() {
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": "def foo():\n    return 1\n", "new": "def bar():\n    return 1\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r == "renames foo → bar"),
        "1:1 def rename: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("adds bar")),
        "a rename is not an add: {rats:?}"
    );
}

#[test]
fn p5_p7_removals() {
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def a():\n    return 1\ndef b():\n    return 2\n", "new": "def a():\n    return 1\n" },
            { "path": "b.py", "old": "import os\nimport sys\nx = 1\n", "new": "import os\nx = 1\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r == "removes b"),
        "deleted def: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r == "removes import sys"),
        "deleted import: {rats:?}"
    );
}

#[test]
fn p11_unnamed_defs_contribute_no_enclosing_segment() {
    // A def with no name of its own (a lua lambda, a c++ anonymous `namespace {`,
    // markdown content before the first heading) is transparent: its contents
    // nest under the nearest *named* def, which is what a reviewer can navigate
    // to. Previously this collapsed runs of `<anonymous>` into one, still
    // leaving `outer.<anonymous>` in user-visible wording.
    let old = "local function outer()\n  reg(function()\n    inner(function()\n      x = 1\n    end)\n  end)\nend\n";
    let new = "local function outer()\n  reg(function()\n    inner(function()\n      x = 2\n    end)\n  end)\nend\n";
    let inp: ordo::model::Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.lua", "old": old, "new": new }] }),
    )
    .unwrap();
    let out = ordo::run(inp);
    let enc: Vec<_> = out.files[0]
        .hunks
        .iter()
        .filter_map(|h| h.enclosing.clone())
        .collect();
    assert!(
        enc.iter().any(|e| e == "outer"),
        "nests under the nearest named def: {enc:?}"
    );
    assert!(
        !enc.iter().any(|e| e.contains("<anonymous>")),
        "no placeholder segment: {enc:?}"
    );
}

#[test]
fn p11_decorated_defs_name_the_wrapped_def() {
    // a decorated class, a decorated method, and stacked decorators — none
    // should yield "<anonymous>" or a doubled segment like "C.C".
    let old = "\
@dataclass
class C:
    x: int
    def method(self):
        return self.x

class W:
    @property
    def foo(self):
        return 1

    @a
    @b
    def g(self):
        return 2
";
    let new = "\
@dataclass
class C:
    x: int
    def method(self):
        return self.x + 1

class W:
    @property
    def foo(self):
        return 100

    @a
    @b
    def g(self):
        return 200
";
    let inp: Input = serde_json::from_value(
        serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "new": new }] }),
    )
    .unwrap();
    let enc: Vec<_> = ordo::run(inp).files[0]
        .hunks
        .iter()
        .filter_map(|h| h.enclosing.clone())
        .collect();
    assert!(!enc.iter().any(|e| e.contains("<anonymous>")), "{enc:?}");
    assert!(enc.iter().any(|e| e == "C.method"), "{enc:?}");
    assert!(enc.iter().any(|e| e == "W.foo"), "{enc:?}");
    assert!(enc.iter().any(|e| e == "W.g"), "{enc:?}");
}

#[test]
fn p11_multi_rename_by_body() {
    // two renames (bodies unchanged) + one genuinely new def, in one file
    let old = "def alpha():\n    return 111\ndef beta():\n    return 222\n";
    let new = "def gamma():\n    return 111\ndef delta():\n    return 222\ndef epsilon():\n    return 999\n";
    let rats =
        rationales(serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "new": new }] }));
    assert!(
        rats.iter().any(|r| r == "renames alpha → gamma"),
        "rename 1: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r == "renames beta → delta"),
        "rename 2: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.starts_with("adds epsilon")),
        "genuine new def: {rats:?}"
    );
}

#[test]
fn p12_move_detection() {
    // helper's body leaves a.py and reappears in b.py (unchanged) → a move, not add+remove
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def helper():\n    return 42\ndef keep():\n    return 1\n", "new": "def keep():\n    return 1\n" },
            { "path": "b.py", "old": "# b\n", "new": "# b\ndef helper():\n    return 42\n" }
        ]
    }));
    assert!(
        rats.iter().any(|r| r == "moves helper from a.py"),
        "move-in on target: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r == "moves helper to b.py"),
        "move-out on source: {rats:?}"
    );
    assert!(
        !rats
            .iter()
            .any(|r| r.contains("removes helper") || r.starts_with("adds helper")),
        "not add+remove: {rats:?}"
    );
}

#[test]
fn p12_noise_formatting_and_generated() {
    // whitespace-only body change → formatting-only noise
    let a = ordo::run(serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "def f():\n    return  1\n", "new": "def f():\n    return 1\n" }]
    })).unwrap());
    assert!(
        a.files[0].hunks.iter().all(|h| h.noise),
        "formatting hunk flagged noise"
    );
    assert!(
        a.files[0]
            .hunks
            .iter()
            .any(|h| h.rationale == "formatting only"),
        "formatting rationale: {:?}",
        a.files[0]
            .hunks
            .iter()
            .map(|h| &h.rationale)
            .collect::<Vec<_>>()
    );

    // generated/lockfile path → noise regardless of content
    let b = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [{ "path": "package-lock.json", "old": "{}\n", "new": "{ \"a\": 1 }\n" }]
        }))
        .unwrap(),
    );
    assert!(
        b.files[0].hunks.iter().all(|h| h.noise),
        "generated hunk flagged noise"
    );
    assert!(
        b.files[0]
            .hunks
            .iter()
            .any(|h| h.rationale == "generated file"),
        "generated rationale"
    );
}

#[test]
fn p12_clusters_split_and_connected() {
    // two unrelated files → two independent parts
    let split = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "a.py", "old": "# a\n", "new": "# a\ndef x():\n    return 1\nq = x()\n" },
                { "path": "b.py", "old": "# b\n", "new": "# b\ndef z():\n    return 2\nr = z()\n" }
            ]
        }))
        .unwrap(),
    );
    assert_eq!(
        split.clusters.len(),
        2,
        "unrelated files split: {:?}",
        split.clusters
    );
    let total: usize = split.files.iter().map(|f| f.hunks.len()).sum();
    assert_eq!(
        split.clusters.iter().map(|c| c.len()).sum::<usize>(),
        total,
        "clusters partition all hunks"
    );

    // cross-file def→use links into one part
    let connected = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
                { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
            ]
        }))
        .unwrap(),
    );
    assert_eq!(
        connected.clusters.len(),
        1,
        "cross-file link → one part: {:?}",
        connected.clusters
    );
}

#[test]
fn containment_edge_joins_nested_def_cluster_without_merging_groups() {
    // outer's own body changes (x = 1 → 100) in one hunk, inner's signature
    // changes (adds a param) in another, no def→use edge between them (the
    // call site is untouched) — the only thing linking the two is that
    // `inner` nests inside `outer`. A containment edge should still land
    // them in one cluster (ClusterChanges: containment feeds connected
    // components) while keeping them as two distinct groups (no flattening
    // to the outermost def).
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "a.py",
                  "old": "def outer(spec):\n    x = 1\n    spacer = 9\n    def inner(y):\n        z = 2\n        return y\n    return inner(spec)\n",
                  "new": "def outer(spec):\n    x = 100\n    spacer = 9\n    def inner(y, w):\n        z = 2\n        return y\n    return inner(spec)\n" }
            ]
        }))
        .unwrap(),
    );
    let hunks = &out.files[0].hunks;
    assert_eq!(hunks.len(), 2, "two independent line hunks: {hunks:?}");

    let outer_group = &hunks
        .iter()
        .find(|h| h.enclosing.as_deref() == Some("outer"))
        .unwrap()
        .group;
    let inner_group = &hunks
        .iter()
        .find(|h| h.enclosing.as_deref() == Some("outer.inner"))
        .unwrap()
        .group;
    assert_ne!(
        outer_group, inner_group,
        "nested def keeps its own group, not flattened into outer's: {hunks:?}"
    );

    assert!(
        out.edges
            .iter()
            .any(|e| e.why.contains("encloses") && e.why.contains("inner")),
        "expected a containment edge naming inner: {:?}",
        out.edges
    );
    assert_eq!(
        out.clusters.len(),
        1,
        "containment joins outer and inner into one cluster: {:?}",
        out.clusters
    );
}

#[test]
fn sibling_methods_of_a_changed_class_are_not_fused() {
    // click 8a1b1a33-shaped case: three sibling methods of one class change
    // independently (no shared def/use, and the class body itself is
    // untouched, so there's no "Context" group for a containment edge to
    // anchor on) — an outermost-def key would wrongly fuse them; they must
    // stay in three separate groups and clusters.
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "ctx.py",
                  "old": "class Context:\n    def __init__(self, info):\n        self.p = info\n    def __enter__(self):\n        return self\n    def scope(self, x):\n        return x\n",
                  "new": "class Context:\n    def __init__(self, info: int):\n        self.p = info\n    def __enter__(self) -> \"Context\":\n        return self\n    def scope(self, x, y):\n        return x\n" }
            ]
        }))
        .unwrap(),
    );
    let hunks = &out.files[0].hunks;
    assert_eq!(
        hunks.len(),
        3,
        "three independent signature hunks: {hunks:?}"
    );

    let groups: std::collections::HashSet<&String> = hunks.iter().map(|h| &h.group).collect();
    assert_eq!(
        groups.len(),
        3,
        "three sibling methods must not share a group: {hunks:?}"
    );
    assert_eq!(
        out.clusters.len(),
        3,
        "no containment anchor (class body untouched) → no fused cluster: {:?}",
        out.clusters
    );
}

#[test]
fn p12_pack_renders_sections() {
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
                { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
            ]
        }))
        .unwrap(),
    );
    let p = ordo::pack(&out);
    assert!(p.contains("# ordo review pack"), "header:\n{p}");
    assert!(p.contains("## reading order"), "order section");
    assert!(p.contains("## dependencies"), "edges section");
    assert!(p.contains("util.py:L2"), "locates util helper");
}

#[test]
fn p13_def_smells_size_and_params() {
    let body: String = (0..65).map(|i| format!("    v{i} = {i}\n")).collect();
    let big = format!("def big():\n{body}");
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "a.py", "old": "", "new": big },
                { "path": "b.py", "old": "", "new": "def f(a, b, c, d, e, f, g):\n    return a\n" }
            ]
        }))
        .unwrap(),
    );
    let notes: Vec<String> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().flat_map(|h| h.notes.clone()))
        .collect();
    assert!(
        notes.iter().any(|n| n.starts_with("large definition")),
        "large-def note: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n == "7 params"),
        "param-bloat note: {notes:?}"
    );
}

#[test]
fn p14_metaclass_advisories() {
    let out = ordo::run(serde_json::from_value(serde_json::json!({
        "changes": [
            { "path": "reg.py", "old": "", "new": "class Registry(type):\n    def __init__(cls, name, bases, ns):\n        pass\n" },
            { "path": "meta.py", "old": "", "new": "class Meta(type):\n    def __new__(mcs, name, bases, ns):\n        return type.__new__(mcs, name, bases, ns)\n" },
            { "path": "use.py", "old": "", "new": "class Widget(metaclass=Meta):\n    pass\n" }
        ]
    })).unwrap());
    let advs: Vec<(String, String, bool)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks.iter().flat_map(move |h| {
                h.advisories
                    .iter()
                    .map(move |a| (f.path.clone(), a.construct.clone(), a.verdict))
            })
        })
        .collect();
    assert!(
        advs.iter()
            .any(|(p, c, v)| p == "reg.py" && c == "metaclass" && *v),
        "register-only metaclass → downgrade verdict: {advs:?}"
    );
    assert!(
        advs.iter()
            .any(|(p, c, v)| p == "meta.py" && c == "metaclass" && !*v),
        "__new__ metaclass → advisory only: {advs:?}"
    );
    assert!(
        advs.iter()
            .any(|(p, c, v)| p == "use.py" && c == "metaclass" && !*v),
        "metaclass usage → advisory: {advs:?}"
    );
}

#[test]
fn p14_catalog() {
    fn advs(path: &str, code: &str) -> Vec<(String, bool)> {
        let out = ordo::run(
            serde_json::from_value(serde_json::json!({
                "changes": [{ "path": path, "old": "", "new": code }]
            }))
            .unwrap(),
        );
        out.files
            .iter()
            .flat_map(|f| {
                f.hunks.iter().flat_map(|h| {
                    h.advisories
                        .iter()
                        .map(|a| (a.construct.clone(), a.verdict))
                })
            })
            .collect()
    }
    let py = advs(
        "a.py",
        "def f(x=[]):\n    return x\ntry:\n    g()\nexcept:\n    pass\nr = eval('1')\n",
    );
    assert!(
        py.iter().any(|(c, v)| c == "mutable-default-arg" && *v),
        "mutable default: {py:?}"
    );
    assert!(
        py.iter().any(|(c, v)| c == "bare-except" && *v),
        "bare except: {py:?}"
    );
    assert!(
        py.iter().any(|(c, v)| c == "eval/exec" && !*v),
        "eval: {py:?}"
    );

    let rs = advs(
        "a.rs",
        "fn f(x: i32) {\n    unsafe {\n        let _y: u32 = std::mem::transmute(x);\n    }\n}\n",
    );
    assert!(rs.iter().any(|(c, _)| c == "unsafe"), "rust unsafe: {rs:?}");
    assert!(
        rs.iter().any(|(c, _)| c == "transmute"),
        "transmute: {rs:?}"
    );

    let js = advs("a.js", "with (obj) { x = 1 }\nvar r = eval('1')\n");
    assert!(js.iter().any(|(c, v)| c == "with" && *v), "with: {js:?}");
    assert!(js.iter().any(|(c, _)| c == "eval"), "js eval: {js:?}");

    let go = advs(
        "a.go",
        "package m\nfunc f(x any) {\n    _ = reflect.TypeOf(x)\n    _ = unsafe.Pointer(nil)\n}\n",
    );
    assert!(go.iter().any(|(c, _)| c == "reflect"), "reflect: {go:?}");
    assert!(go.iter().any(|(c, _)| c == "unsafe"), "go unsafe: {go:?}");
}

#[test]
fn p14_batch2() {
    fn advs(path: &str, code: &str) -> Vec<(String, bool)> {
        let out = ordo::run(
            serde_json::from_value(serde_json::json!({
                "changes": [{ "path": path, "old": "", "new": code }]
            }))
            .unwrap(),
        );
        out.files
            .iter()
            .flat_map(|f| {
                f.hunks.iter().flat_map(|h| {
                    h.advisories
                        .iter()
                        .map(|a| (a.construct.clone(), a.verdict))
                })
            })
            .collect()
    }
    let py = advs("svc.py", "def check(x):\n    assert x > 0\nC = type('C', (), {})\ntry:\n    f()\nexcept ValueError:\n    pass\n");
    assert!(
        py.iter().any(|(c, v)| c == "assert-validation" && *v),
        "assert: {py:?}"
    );
    assert!(
        py.iter().any(|(c, _)| c == "dynamic-type"),
        "dynamic type: {py:?}"
    );
    assert!(
        py.iter().any(|(c, v)| c == "empty-catch" && *v),
        "py empty catch: {py:?}"
    );

    let rs = advs("a.rs", "static mut COUNT: u32 = 0;\n");
    assert!(
        rs.iter().any(|(c, v)| c == "static-mut" && *v),
        "static mut: {rs:?}"
    );

    let js = advs("a.js", "try { f() } catch (e) {}\n");
    assert!(
        js.iter().any(|(c, v)| c == "empty-catch" && *v),
        "js empty catch: {js:?}"
    );
    let ts = advs("a.ts", "let x: any = 1;\n");
    assert!(ts.iter().any(|(c, _)| c == "any"), "ts any: {ts:?}");

    let go = advs("svc.go", "package m\nfunc f() {\n    panic(\"x\")\n}\n");
    assert!(go.iter().any(|(c, _)| c == "panic"), "go panic: {go:?}");

    let c = advs(
        "a.c",
        "int f() {\n    goto done;\ndone:\n    return 0;\n}\n",
    );
    assert!(c.iter().any(|(c, _)| c == "goto"), "c goto: {c:?}");

    let cpp = advs(
        "a.cpp",
        "int f(char* p) {\n    return *reinterpret_cast<int*>(p);\n}\n",
    );
    assert!(
        cpp.iter().any(|(c, _)| c == "reinterpret_cast"),
        "cpp reinterpret_cast: {cpp:?}"
    );

    let java = advs("A.java", "class A {\n    void f() throws Exception {\n        try { g(); } catch (Exception e) {}\n        java.lang.reflect.Method m = null;\n        m.setAccessible(true);\n    }\n}\n");
    assert!(
        java.iter().any(|(c, v)| c == "empty-catch" && *v),
        "java empty catch: {java:?}"
    );
    assert!(
        java.iter().any(|(c, _)| c == "reflection"),
        "java reflection: {java:?}"
    );
}

#[test]
fn p16_extraction_from_present_def() {
    // read_input's body is extracted from order (which still exists as a wrapper);
    // bodies overlap but are not identical, and order is NOT removed → not a rename.
    let old = "fn order() {\n    let mut buf = String::new();\n    io::stdin().read_to_string(&mut buf).unwrap();\n    log::debug!(\"got input\");\n    let parsed = serde_json::from_str(&buf).unwrap();\n    emit(run(parsed));\n}\n";
    let new = "fn read_input() -> Input {\n    let mut buf = String::new();\n    io::stdin().read_to_string(&mut buf).unwrap();\n    log::debug!(\"got input\");\n    serde_json::from_str(&buf).unwrap()\n}\nfn order() {\n    emit(run(read_input()));\n}\n";
    let rats =
        rationales(serde_json::json!({ "changes": [{ "path": "m.rs", "old": old, "new": new }] }));
    assert!(
        rats.iter()
            .any(|r| r == "adds read_input, extracted from order"),
        "extraction detected: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r == "adds read_input"),
        "must not read as a plain add: {rats:?}"
    );
}

#[test]
fn p16_multi_extraction_from_same_source_is_grouped() {
    // read_input and check_input are both extracted from order in one hunk;
    // the shared provenance must be named once, not repeated per symbol.
    // Reused lines are nested an extra indent level in `old` so the raw text
    // diff never marks them literally unchanged (which would split them into
    // separate hunks) while the whitespace-normalized body-overlap detector
    // (extract::symbol_bodies) still recognizes the relocation.
    let old = "def order():\n    if True:\n        buf = read_stdin()\n        log_debug(buf)\n        parsed = parse(buf)\n    if True:\n        validate(parsed)\n        emit(run(parsed))\n        finalize(parsed)\n";
    let new = "def read_input():\n    buf = read_stdin()\n    log_debug(buf)\n    parsed = parse(buf)\n    return parsed\n\ndef check_input():\n    validate(parsed)\n    emit(run(parsed))\n    finalize(parsed)\n\ndef order():  # dispatch\n    parsed = read_input()\n    check_input()\n";
    let rats =
        rationales(serde_json::json!({ "changes": [{ "path": "m.py", "old": old, "new": new }] }));
    assert!(
        rats.iter()
            .any(|r| r.contains("adds check_input, read_input, extracted from order")),
        "grouped extraction: {rats:?}"
    );
    assert!(
        !rats
            .iter()
            .any(|r| r.matches("extracted from order").count() > 1),
        "provenance must not repeat: {rats:?}"
    );
}

#[test]
fn placeholder_defs_suppressed() {
    // `_`-bound / anonymous closures carry no navigational signal → dropped from wording.
    let rats = rationales(serde_json::json!({ "changes": [{
        "path": "m.lua",
        "old": "local x = 1\n",
        "new": "local x = 1\n_ = function() return 1 end\nlocal t = { run = function() return 2 end }\n",
    }]}));
    // P17 composes binding wording with def-side wording (see order.rs
    // rationale_for): `t` is a genuine new local binding alongside `run`, so
    // it's named too, not just "adds run" in isolation.
    assert!(
        rats.iter().any(|r| r.starts_with("adds run")),
        "named def kept: {rats:?}"
    );
    assert!(
        !rats
            .iter()
            .any(|r| r.contains("adds _") || r.contains("<anonymous>")),
        "placeholders dropped: {rats:?}"
    );
}

#[test]
fn p14_deep_python_cpp() {
    fn advs(path: &str, code: &str) -> Vec<(String, bool)> {
        let out = ordo::run(
            serde_json::from_value(serde_json::json!({
                "changes": [{ "path": path, "old": "", "new": code }]
            }))
            .unwrap(),
        );
        out.files
            .iter()
            .flat_map(|f| {
                f.hunks.iter().flat_map(|h| {
                    h.advisories
                        .iter()
                        .map(|a| (a.construct.clone(), a.verdict))
                })
            })
            .collect()
    }
    let has = |v: &[(String, bool)], c: &str, verdict: bool| {
        v.iter().any(|(k, w)| k == c && *w == verdict)
    };

    let py = advs(
        "a.py",
        "import os, subprocess, pickle\nclass C:\n    def __eq__(self, o):\n        return True\n    def __del__(self):\n        pass\nos.system(x)\nsubprocess.run(x, shell=True)\npickle.loads(b)\n",
    );
    assert!(has(&py, "eq-without-hash", true), "eq/hash: {py:?}");
    assert!(has(&py, "del-finalizer", true), "__del__: {py:?}");
    assert!(has(&py, "os-system", false), "os.system: {py:?}");
    assert!(has(&py, "shell-injection", true), "shell=True: {py:?}");
    assert!(has(&py, "pickle", false), "pickle: {py:?}");

    // __eq__ + __hash__ present → no advisory
    let ok = advs(
        "b.py",
        "class D:\n    def __eq__(self, o):\n        return True\n    def __hash__(self):\n        return 0\n",
    );
    assert!(
        !ok.iter().any(|(c, _)| c == "eq-without-hash"),
        "hash present → quiet: {ok:?}"
    );

    let cpp = advs(
        "a.cpp",
        "#define SQ(x) ((x)*(x))\nvoid f(){ char b[4]; strcpy(b,s); int* p = new int(3); free(malloc(8)); int y=(int)3.0; auto q=reinterpret_cast<long>(p); auto r=const_cast<int*>(p); delete p; }\n",
    );
    assert!(has(&cpp, "unsafe-str-fn", true), "strcpy: {cpp:?}");
    assert!(has(&cpp, "raw-new-delete", false), "new/delete: {cpp:?}");
    assert!(has(&cpp, "manual-memory", false), "malloc: {cpp:?}");
    assert!(has(&cpp, "c-style-cast", false), "c-cast: {cpp:?}");
    assert!(has(&cpp, "const-cast", false), "const_cast: {cpp:?}");
    assert!(has(&cpp, "function-macro", false), "macro: {cpp:?}");

    // using namespace std: advisory always, verdict only in a header
    let src = advs("a.cpp", "using namespace std;\n");
    assert!(
        has(&src, "using-namespace-std", false),
        "in .cpp → no verdict: {src:?}"
    );
    let hdr = advs("a.hpp", "using namespace std;\n");
    assert!(
        has(&hdr, "using-namespace-std", true),
        "in header → verdict: {hdr:?}"
    );
}

#[test]
fn p14_derived_python_cpp() {
    fn advs(path: &str, code: &str) -> Vec<(String, bool)> {
        let out = ordo::run(
            serde_json::from_value(serde_json::json!({
                "changes": [{ "path": path, "old": "z", "new": code }]
            }))
            .unwrap(),
        );
        out.files
            .iter()
            .flat_map(|f| {
                f.hunks.iter().flat_map(|h| {
                    h.advisories
                        .iter()
                        .map(|a| (a.construct.clone(), a.verdict))
                })
            })
            .collect()
    }
    let has = |v: &[(String, bool)], c: &str, verdict: bool| {
        v.iter().any(|(k, w)| k == c && *w == verdict)
    };

    let py = advs(
        "a.py",
        "import asyncio, yaml, functools\nfrom contextlib import suppress\nclass C:\n    def __enter__(self): return self\n    @functools.lru_cache\n    def m(self): return 1\n    def __getattribute__(self, n): return 1\nasync def h():\n    time.sleep(1)\ncur.execute(f'select {x}')\nrequests.get(u, verify=False)\nyaml.load(data)\nasyncio.create_task(bg())\nwith suppress(Exception):\n    pass\n",
    );
    for c in [
        ("blocking-in-async", true),
        ("lru-cache-on-method", true),
        ("sql-injection", true),
        ("tls-no-verify", true),
        ("yaml-load", true),
        ("fire-and-forget-task", true),
        ("half-context-manager", true),
        ("getattribute-override", false),
        ("broad-suppress", false),
    ] {
        assert!(has(&py, c.0, c.1), "python {}: {py:?}", c.0);
    }
    // negatives
    let pyn = advs("b.py", "class C:\n    def __enter__(self): return self\n    def __exit__(self, *a): pass\ncur.execute('select 1', p)\nyaml.load(d, Loader=SafeLoader)\nx = asyncio.create_task(bg())\n");
    assert!(
        !pyn.iter().any(|(c, _)| c == "half-context-manager"
            || c == "sql-injection"
            || c == "yaml-load"
            || c == "fire-and-forget-task"),
        "py negatives: {pyn:?}"
    );

    let cpp = advs(
        "a.cpp",
        "struct T { ~T() { throw 1; } };\nbool operator&&(T a, T b) { return true; }\nvoid f() {\n  volatile int v = 0;\n  auto g = [&]() { return v; };\n  std::memcpy(p, q, 8);\n  system(cmd);\n  alloca(64);\n  rand();\n  setjmp(buf);\n  auto d = dynamic_cast<T*>(pp);\n  try { g(); } catch (std::exception e) {}\n}",
    );
    for c in [
        ("throw-in-destructor", true),
        ("operator-logical", true),
        ("setjmp-longjmp", true),
        ("volatile", false),
        ("lambda-ref-capture", false),
        ("mem-family", false),
        ("shell-exec", false),
        ("alloca", false),
        ("non-reentrant", false),
        ("dynamic-cast", false),
        ("catch-by-value", false),
    ] {
        assert!(has(&cpp, c.0, c.1), "cpp {}: {cpp:?}", c.0);
    }
    // negatives
    let cppn = advs("b.cpp", "void ok() { throw 1; }\nvoid f() {\n  auto a = [=]() { return 1; };\n  auto b = [&x]() { return x; };\n  try { f(); } catch (const std::exception& e) {}\n  try { f(); } catch (int e) {}\n}");
    assert!(
        !cppn.iter().any(|(c, _)| c == "throw-in-destructor"
            || c == "lambda-ref-capture"
            || c == "catch-by-value"),
        "cpp negatives: {cppn:?}"
    );
}

#[test]
fn rust_const_and_static_are_definitions() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [ { "path": "a.rs", "old": "pub fn go() -> u64 {\n    1\n}\n", "new": "const WINDOW_MINS: u64 = 24 * 60;\nstatic NAME: &str = \"x\";\n\npub fn go() -> u64 {\n    WINDOW_MINS\n}\n" } ]
    }))
    .unwrap();
    let out = ordo::run(inp);
    let hunks = &out.files[0].hunks;
    let const_hunk = &hunks[0];
    assert_eq!(const_hunk.category, ordo::model::Category::Definition);
    assert_eq!(const_hunk.defines, vec!["NAME", "WINDOW_MINS"]);
    assert!(!const_hunk.uses.contains(&"WINDOW_MINS".to_string()));
    assert!(
        const_hunk.rationale.starts_with("adds"),
        "const/static def → adds: {}",
        const_hunk.rationale
    );
    let use_hunk = &hunks[1];
    assert!(use_hunk.uses.contains(&"WINDOW_MINS".to_string()));
    assert!(
        out.edges
            .iter()
            .any(|e| e.from == const_hunk.id && e.to == use_hunk.id),
        "expected def→use edge from const hunk to consumer: {:?}",
        out.edges
    );
}

#[test]
fn java_field_is_a_definition() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [ { "path": "A.java", "old": "class A {\n  int go() {\n    return 1;\n  }\n}\n", "new": "class A {\n  private static final int WINDOW = 1440;\n\n  int go() {\n    return WINDOW;\n  }\n}\n" } ]
    }))
    .unwrap();
    let out = ordo::run(inp);
    let hunks = &out.files[0].hunks;
    let field_hunk = &hunks[0];
    assert_eq!(field_hunk.category, ordo::model::Category::Definition);
    assert_eq!(field_hunk.defines, vec!["WINDOW"]);
    assert!(!field_hunk.uses.contains(&"WINDOW".to_string()));
    let use_hunk = &hunks[1];
    assert!(use_hunk.uses.contains(&"WINDOW".to_string()));
    assert!(
        out.edges
            .iter()
            .any(|e| e.from == field_hunk.id && e.to == use_hunk.id),
        "expected def→use edge from java field hunk to consumer: {:?}",
        out.edges
    );
}

#[test]
fn rationale_long_symbol_list_is_capped() {
    // a whole-new file where every top-level def lands in one hunk — worst
    // case for an uncapped "adds a, b, c, ..." rationale.
    let new = (0..20)
        .map(|i| format!("def fn_{i}():\n    pass\n"))
        .collect::<Vec<_>>()
        .join("\n");
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": "", "new": new } ]
    }));
    assert_eq!(rats.len(), 1);
    let r = &rats[0];
    assert!(r.starts_with("adds fn_0, fn_1, fn_10, and 17 more"), "{r}");
    assert!(
        r.len() < 80,
        "rationale should stay short: {} chars: {r}",
        r.len()
    );
}

#[test]
fn comment_only_gets_comment_wording_code_edit_unaffected() {
    // a top-level comment inserted before f: no enclosing def, no defines/uses
    // → used to fall through to the bare "change" fallback; should now say so.
    // g's body edit is ordinary code and must keep its usual "edits g" wording.
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py",
            "old": "def f():\n    return 1\n\ndef g():\n    return 2\n",
            "new": "# note about f\ndef f():\n    return 1\n\ndef g():\n    return 3\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r == "adds comment"),
        "top-level comment insert → adds comment: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r == "edits g"),
        "ordinary body edit inside g is unaffected: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r == "change"),
        "no hunk should fall back to the bare 'change': {rats:?}"
    );
}

#[test]
fn comment_only_inside_definition_names_container() {
    // a comment-only edit inside f used to be reported as "edits f" (as if the
    // code changed); it must now say it's a comment edit and still name f.
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py",
            "old": "def f():\n    # note\n    return 1\n",
            "new": "def f():\n    # updated note\n    return 1\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r == "edits comment in f"),
        "comment-only edit inside a def names it: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r == "edits f"),
        "must not be worded as a code edit: {rats:?}"
    );
}

#[test]
fn comment_and_code_together_is_not_comment_wording() {
    // the comment line and the return line change in the same contiguous
    // hunk — real code changed too, so the comment-only wording must not fire.
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py",
            "old": "def f():\n    # note\n    return 1\n",
            "new": "def f():\n    # updated note\n    return 2\n" } ]
    }));
    assert!(
        rats.iter().any(|r| r == "edits f"),
        "mixed comment+code hunk keeps ordinary code-edit wording: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("comment")),
        "must not be described as a comment change: {rats:?}"
    );
}

#[test]
fn python_multiline_docstring_only_change_is_a_comment_change() {
    // interior prose lines of a multi-line docstring match no `#`-style
    // textual prefix — the tree-sitter path (a `string` node that's the
    // sole child of the first `expression_statement` in f's body) is what
    // recognizes this as a comment change, not a code change.
    let old =
        "def f():\n    \"\"\"\n    Old line one.\n    Old line two.\n    \"\"\"\n    return 1\n";
    let new =
        "def f():\n    \"\"\"\n    New line one.\n    New line two.\n    \"\"\"\n    return 1\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": old, "new": new } ]
    }));
    assert!(
        rats.iter().any(|r| r == "edits comment in f"),
        "multi-line docstring edit is a comment change: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r == "edits f"),
        "must not be worded as a code edit: {rats:?}"
    );
}

#[test]
fn python_docstring_and_code_together_is_not_comment_wording() {
    // the docstring line and the return line change in the same contiguous
    // hunk — real code changed too, so the comment-only wording must not fire.
    let old = "def f():\n    \"\"\"Old text.\"\"\"\n    return 1\n";
    let new = "def f():\n    \"\"\"New text.\"\"\"\n    return 2\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": old, "new": new } ]
    }));
    assert!(
        rats.iter().any(|r| r == "edits f"),
        "mixed docstring+code hunk keeps ordinary code-edit wording: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("comment")),
        "must not be described as a comment change: {rats:?}"
    );
}

#[test]
fn only_comments_keeps_just_the_comment_hunks_with_a_consistent_order() {
    // a.py mixes a comment-only hunk (f's docstring) with a code hunk (g's
    // body); b.py is pure code. With only_comments on, only f's hunk survives,
    // it carries comment: true, and `order` names exactly that one hunk.
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "a.py",
                  "old": "def f():\n    \"\"\"old.\"\"\"\n    return 1\n\ndef g():\n    return 2\n",
                  "new": "def f():\n    \"\"\"new.\"\"\"\n    return 1\n\ndef g():\n    return 3\n" },
                { "path": "b.py", "old": "def h():\n    return 1\n", "new": "def h():\n    return 2\n" }
            ],
            "options": { "only_comments": true }
        }))
        .unwrap(),
    );
    let all_hunks: Vec<&ordo::model::HunkOut> =
        out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert_eq!(
        all_hunks.len(),
        1,
        "only the comment hunk remains: {all_hunks:?}"
    );
    assert!(
        all_hunks[0].comment,
        "surviving hunk is marked comment: true"
    );
    assert_eq!(out.order.len(), 1, "order lists exactly that one hunk");
    assert_eq!(out.order[0].hunk, all_hunks[0].id);
}

#[test]
fn comment_field_is_set_on_exactly_the_comment_hunks() {
    // without only_comments, every hunk survives; `comment` distinguishes the
    // docstring-only hunk in f from the ordinary body edit in g.
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [ { "path": "a.py",
                "old": "def f():\n    \"\"\"old.\"\"\"\n    return 1\n\ndef g():\n    return 2\n",
                "new": "def f():\n    \"\"\"new.\"\"\"\n    return 1\n\ndef g():\n    return 3\n" } ]
        }))
        .unwrap(),
    );
    let f = &out.files[0];
    assert_eq!(f.hunks.len(), 2);
    let by_range = |lo: usize| f.hunks.iter().find(|h| h.new_range[0] == lo).unwrap();
    assert!(by_range(2).comment, "f's docstring hunk is comment: true");
    assert!(!by_range(6).comment, "g's body hunk is comment: false");
}

#[test]
fn unsupported_extension_flagged() {
    let out = ordo::run(
        serde_json::from_value(serde_json::json!({
            "changes": [
                { "path": "logo.gif", "old": "abc", "new": "abd" },
                { "path": "a.py", "old": "x = 1\n", "new": "x = 2\n" }
            ]
        }))
        .unwrap(),
    );
    let gif = out.files.iter().find(|f| f.path == "logo.gif").unwrap();
    let py = out.files.iter().find(|f| f.path == "a.py").unwrap();
    assert!(gif.unsupported, "no grammar → unsupported: true");
    assert!(!py.unsupported, "known language → unsupported: false");
}

#[test]
fn rust_impl_blocks_name_their_type_not_a_lifetime_or_trait() {
    // `impl<'s> Worker<'s>` used to name itself `<anonymous>` (its first named
    // child is the lifetime list), and `impl Display for Work` named itself
    // after the *trait* (the first identifier it found). Both found by the
    // corpus sweep over ripgrep.
    let rats = rationales(serde_json::json!({
        "changes": [
            { "path": "a.rs", "old": "struct W;\n",
              "new": "struct W;\nimpl<'s> Worker<'s> {\n    fn go(&self) -> u8 { 1 }\n}\n" },
            { "path": "b.rs", "old": "struct W;\n",
              "new": "struct W;\nimpl Display for Work {\n    fn fmt(&self) -> u8 { 2 }\n}\n" }
        ]
    }));
    assert!(
        !rats.iter().any(|r| r.contains("<anonymous>")),
        "impl with generics must not be anonymous: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("Worker")),
        "generic impl names its type: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("Work")) && !rats.iter().any(|r| r.contains("Display")),
        "trait impl names the type, not the trait: {rats:?}"
    );
}

#[test]
fn a_qualified_name_split_across_lines_is_joined() {
    // OpenFOAM's house style wraps a qualified name at the `::`, which put a
    // newline into `enclosing`, `defines` and the rationale — the last of which
    // is contractually one line. Found by the corpus sweep.
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "a.C",
            "old": "namespace Foam { namespace kt { } }\n",
            "new": "namespace Foam { namespace kt {\nFoam::scalar Foam::kt::\nSchaeffer::nu\n(\n    int a\n) const\n{\n    return 1;\n}\n} }\n" }]
    }));
    assert!(
        !rats.iter().any(|r| r.contains('\n')),
        "a rationale must never span lines: {rats:?}"
    );
    assert!(
        rats.iter().any(|r| r.contains("kt::Schaeffer::nu")),
        "the wrapped name is joined, not truncated: {rats:?}"
    );
}

#[test]
fn several_extraction_sources_name_the_relation_once() {
    // Grouping was per source but not across sources, so two sources each said
    // "extracted from" — the same shape as the multi-scope binding case. Found
    // by raising curl's corpus cap.
    let old = "\
def alpha():
    part_one = 1
    part_two = 2
    return part_one + part_two


def beta():
    other_one = 3
    other_two = 4
    return other_one + other_two
";
    let new = "\
def helper_a():
    part_one = 1
    part_two = 2
    return part_one + part_two


def helper_b():
    other_one = 3
    other_two = 4
    return other_one + other_two


def alpha():
    return helper_a()


def beta():
    return helper_b()
";
    let rats = rationales(serde_json::json!({
        "changes": [{ "path": "m.py", "old": old, "new": new }]
    }));
    for r in &rats {
        assert!(
            r.matches(", extracted from ").count() <= 1,
            "the relation must be named once however many sources: {r}"
        );
    }
}

#[test]
fn tail_fragment_of_a_multiline_import_is_an_import_not_a_bare_change() {
    // only the closing "} from '...'" line changes — the import node's own
    // start row is several lines above the hunk, so a row-based check that
    // only recognizes an import by its *start* row would miss this hunk
    // entirely and fall through to a bare "change".
    let old = "import {\n\ta,\n\tb,\n} from './x';\n";
    let new = "import {\n\ta,\n\tb,\n} from './y';\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.ts", "old": old, "new": new } ]
    }));
    assert_eq!(
        rats,
        vec!["changes import a, b"],
        "a fragment of an import is still an import hunk"
    );
}

#[test]
fn middle_fragment_of_a_multiline_import_list_is_an_import_not_a_bare_change() {
    // a name inserted in the middle of a parenthesized `from x import (...)`
    // list — its row sits well after the import statement's own start row.
    let old = "from a.b import (\n    c,\n    d,\n)\n";
    let new = "from a.b import (\n    c,\n    e,\n    d,\n)\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": old, "new": new } ]
    }));
    // the statement's whole name list, since that is what the import now binds
    assert_eq!(
        rats,
        vec!["changes import c, d, e"],
        "a fragment of an import is still an import hunk"
    );
}

#[test]
fn python_block_comment_string_outside_first_statement_position_is_a_comment() {
    // the "attribute docstring" convention (Sphinx/attrs): a bare triple-quoted
    // string immediately after the module-level assignment it documents, not
    // in first-statement position, so the old first-statement-only rule missed
    // it and this hunk fell through to a bare "change".
    let old = "import os\n\nN = 5\n\"\"\"Old note about N.\nMore old detail.\"\"\"\n\ndef f():\n    return N\n";
    let new = "import os\n\nN = 5\n\"\"\"New note about N.\nMore old detail.\"\"\"\n\ndef f():\n    return N\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": old, "new": new } ]
    }));
    assert!(
        rats.iter().any(|r| r.contains("comment")),
        "a trailing attribute docstring reads as a comment change: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r == "change"),
        "must not fall through to the bare fallback: {rats:?}"
    );
}

#[test]
fn multiline_data_string_assigned_to_a_variable_is_not_a_comment() {
    // a bare string statement right after an assignment is the docstring
    // convention this widening targets — but a string that IS the assignment's
    // own right-hand side (SQL, an HTML template, …) is data, not documentation,
    // and must not be swept in by the same rule.
    let old = "QUERY = \"\"\"SELECT *\nFROM t\nWHERE x = 1\"\"\"\n";
    let new = "QUERY = \"\"\"SELECT *\nFROM t\nWHERE x = 2\"\"\"\n";
    let rats = rationales(serde_json::json!({
        "changes": [ { "path": "a.py", "old": old, "new": new } ]
    }));
    assert!(
        !rats.iter().any(|r| r.contains("comment")),
        "a data string assigned to a variable must not be called a comment: {rats:?}"
    );
}
