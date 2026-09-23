//! Rationales the labeled corpus caught lying (tasks-3uv.*): each test pins
//! one shape from a silver or gold hunk, reduced to the lines that mattered.
mod fixture;
use fixture::{hunks, rationales_of as rationales, run_file as one, run_json as run};
use ordo::model::Category;
use serde_json::json;

/// openfoam cb1e00ab dimensionSetI.H: `const T&` returns named every function
/// after its type, and the rename detector read the lot as `max → dimensionSet`.
#[test]
fn a_reference_return_type_does_not_rename_the_function() {
    let hs = hunks(
        "d.H",
        "inline dimensionSet max(const dimensionSet& a, const dimensionSet& b)\n{\n    return a;\n}\n\
         inline dimensionSet min(const dimensionSet& a, const dimensionSet& b)\n{\n    return b;\n}\n",
        "inline dimensionSet max(const dimensionSet& a, const dimensionSet& b)\n{\n    return a;\n}\n\
         inline const dimensionSet& min(const dimensionSet& a, const dimensionSet& b)\n{\n    return b;\n}\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].symbols[0].name, "min");
    assert_eq!(hs[0].rationale, "changes signature of min");
}

/// curl c437d28c cfilters.h: `struct Curl_easy *data` in a parameter list is a
/// struct_specifier node, and a prototype narrowing its `sockindex` parameter
/// read as "changes type Curl_easy".
#[test]
fn a_struct_mention_in_a_prototype_is_not_a_type_change() {
    let hs = hunks(
        "cf.h",
        "#ifndef CF_H\n#define CF_H\nbool Curl_conn_is_tunneling(struct connectdata *conn, int sockindex);\n#endif\n",
        "#ifndef CF_H\n#define CF_H\nbool Curl_conn_is_tunneling(struct connectdata *conn, int8_t sockindex);\n#endif\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].defines, vec!["Curl_conn_is_tunneling"]);
    assert_eq!(
        hs[0].rationale,
        "changes signature of Curl_conn_is_tunneling"
    );
}

/// curl c8df3def easy.c: `enum dupstring i;` inside a function defines nothing,
/// so deleting it must not read as moving `dupstring` to another file.
#[test]
fn a_local_enum_mention_defines_nothing() {
    let out = run(json!({"changes": [
        {"path": "easy.c",
         "old": "int f(void)\n{\n  enum dupstring i;\n  return 0;\n}\n",
         "new": "int f(void)\n{\n  return 0;\n}\n"},
        {"path": "setopt.c",
         "old": "int g(void) { return 1; }\n",
         "new": "enum dupstring {\n  STRING_A,\n  STRING_B,\n  STRING_C\n};\n\nint g(void) { return 1; }\n"}
    ]}));
    let r = rationales(&out);
    assert!(r.iter().all(|x| !x.starts_with("moves")), "{r:?}");
}

/// excalidraw 647a264a: a hunk that deletes an exported const and leaves
/// `export {};` behind is a removed public export, not import noise.
#[test]
fn deleting_an_export_is_not_import_noise() {
    let hs = hunks(
        "items.ts",
        "import { a } from \"./a\";\n\nexport const toggleTheme = {\n  ...a,\n  label: \"Toggle theme\",\n};\n",
        "export {};\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_ne!(hs[0].category, Category::Import);
    assert!(!hs[0].noise, "{}", hs[0].rationale);
}

/// cobra 117698a6 command.go: `var` to `const` on a file-scope binding changes
/// that binding; it does not "use" it.
#[test]
fn go_var_to_const_edits_the_binding() {
    let hs = hunks(
        "command.go",
        "package cobra\n\nvar defaultHelpTemplate = `help`\n",
        "package cobra\n\nconst defaultHelpTemplate = `help`\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "changes defaultHelpTemplate");
}

/// cobra 88b30ab8 yaml_docs.go: one line of a nine-line import block changed,
/// and the rationale listed all nine.
#[test]
fn go_import_block_names_only_the_line_that_changed() {
    let block = |yaml: &str| {
        format!("package doc\n\nimport (\n\t\"bytes\"\n\t\"fmt\"\n\t\"io\"\n\t\"os\"\n\t\"path/filepath\"\n\t\"sort\"\n\t\"strings\"\n\n\t\"{yaml}\"\n\n\t\"github.com/spf13/cobra\"\n)\n")
    };
    let hs = hunks(
        "yaml_docs.go",
        &block("gopkg.in/yaml.v3"),
        &block("go.yaml.in/yaml/v3"),
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "adds import v3");
}

/// buildtheworld 0858367a: a symbol this change introduces is "added", so the
/// reader does not take the definition for pre-existing code.
#[test]
fn a_use_of_a_symbol_this_change_adds_says_added() {
    let out = one(
        "b.py",
        "class R:\n    def start(self):\n        return 1\n\n\nif True:\n    r = R()\n",
        "class R:\n    def start(self):\n        return 1\n\n    def matches(self, fp):\n        return fp == 1\n\n\nif True:\n    r = R()\n    if r.matches(1):\n        print(r)\n",
    );
    let r = rationales(&out);
    assert!(r.iter().any(|x| x == "uses matches, added above"), "{r:?}");
}

/// cobra ad460ea8 args_test.go: a new helper modelled on an existing one is
/// not extracted from it — the source keeps every line it had.
#[test]
fn a_new_function_modelled_on_an_untouched_one_is_not_extracted() {
    let body = |name: &str| {
        format!("func {name}(err error, t *testing.T) {{\n\tif err == nil {{\n\t\tt.Fatal(\"Expected an error\")\n\t}}\n\tgot := err.Error()\n\tif got != \"\" {{\n\t\tt.Errorf(\"got: %q\", got)\n\t}}\n}}\n")
    };
    let old = format!("package cobra\n\n{}", body("validOnly"));
    let new = format!(
        "package cobra\n\n{}\n{}",
        body("validOnly"),
        body("noDuplicate")
    );
    let r = rationales(&one("args_test.go", &old, &new));
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].starts_with("adds noDuplicate"), "{r:?}");
    assert!(!r[0].contains("extracted"), "{r:?}");
}

/// buildtheworld f3a901f1: two files receiving the same yaml keys and values
/// name no source — the move used to point at whichever came first.
#[test]
fn an_ambiguous_move_source_is_not_claimed() {
    let doc = "---\nkind: pip\nflags: --pre --upgrade\npackages: tiled\n...\n";
    let out = run(json!({"changes": [
        {"path": "a.yaml", "old": doc, "new": ""},
        {"path": "b.yaml", "old": "", "new": doc},
        {"path": "c.yaml", "old": "", "new": doc}
    ]}));
    let r = rationales(&out);
    assert_eq!(r.len(), 3, "{r:?}");
    assert!(r.iter().all(|x| !x.contains("from a.yaml")), "{r:?}");
}

/// buildtheworld 5bd2ed1d 95-numba.yaml: a new yaml document repeats the keys
/// its neighbours have; inserted lines edit nothing.
#[test]
fn an_added_yaml_document_adds_its_keys() {
    let doc =
        |n: &str| format!("---\ndefault_branch: main\nkind: source_install\nname: {n}\n...\n");
    let hs = hunks(
        "o.yaml",
        &format!("{}{}", doc("numba"), doc("llvmlite")),
        &format!("{}{}{}", doc("numba"), doc("llvmlite"), doc("sparse")),
    );
    let last = hs.last().expect("the added document");
    assert!(last.rationale.starts_with("adds"), "{}", last.rationale);
}

/// cobra ceb39aba size-labeler.yml: a deleted file is gone; naming one of its
/// keys — or twenty-five of them — says the rest survived.
#[test]
fn a_deleted_file_says_the_file_is_gone() {
    let hs = hunks(
        ".github/workflows/w.yml",
        "name: size-labeler\non: [pull_request]\njobs:\n  label:\n    runs-on: ubuntu-latest\n",
        "",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "deletes the file");
}

/// click 051bb0f3 _termui_impl.py: deleting the tail of a class names every
/// member that left, once each.
#[test]
fn a_multi_member_deletion_names_them_all_once() {
    let hs = hunks(
        "t.py",
        "class C:\n    def a(self):\n        return 1\n\n    def b(self):\n        return 2\n\n    def c(self):\n        return 3\n",
        "class C:\n    def a(self):\n        return 1\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "removes b, c");
}

/// click 051725fa: two renames in one commit, one of them with an edited
/// docstring, and bodies identical but for that line. Both used to read as
/// unrelated deletions and additions, and the tests following them claimed
/// to be new coverage.
#[test]
fn several_renames_at_once_are_still_renames() {
    let f = |name: &str, doc: &str| {
        format!("def {name}(n):\n    \"\"\"Opens a stream.\n\n    .. deprecated:: {doc}\n    \"\"\"\n    return open(n)\n\n\n")
    };
    let old = format!(
        "{}{}",
        f("get_binary_stream", "8.5"),
        f("get_text_stream", "8.5")
    );
    let new = format!(
        "{}{}",
        f("_get_binary_stream", "8.5.0"),
        f("_get_text_stream", "8.5")
    );
    let r = rationales(&one("utils.py", &old, &new));
    for pair in [
        "get_binary_stream → _get_binary_stream",
        "get_text_stream → _get_text_stream",
    ] {
        assert!(r.iter().any(|x| x.contains(pair)), "{pair}: {r:?}");
    }
}

/// click 051725fa tests/test_testing.py: a test respelled to follow a rename
/// asserts no new coverage.
#[test]
fn a_test_following_a_rename_does_not_claim_coverage() {
    let out = run(json!({"changes": [
        {"path": "src/utils.py",
         "old": "def get_stream(name):\n    opener = streams.get(name)\n    if opener is None:\n        raise TypeError(name)\n    return opener()\n",
         "new": "def _get_stream(name):\n    opener = streams.get(name)\n    if opener is None:\n        raise TypeError(name)\n    return opener()\n"},
        {"path": "tests/test_utils.py",
         "old": "def test_stream():\n    i = get_stream(\"stdin\")\n    assert i\n",
         "new": "def test_stream():\n    i = _get_stream(\"stdin\")\n    assert i\n"}
    ]}));
    let r = rationales(&out);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r.iter().all(|x| !x.starts_with("tests ")), "{r:?}");
}

/// openfoam 08477d77 SchnerrSauer.C: `*limitedAlphal/(…)` opens a line with a
/// dereference, not a block comment, and a formula rewrite is not "replaces 2
/// lines with comments".
#[test]
fn a_wrapped_multiplication_is_not_a_comment() {
    let hs = hunks(
        "s.C",
        "scalar f()\n{\n    return cbrt(a)\n       *limitedAlphal/(1 + alphaNuc() - limitedAlphal),\n        1.0/3.0\n    );\n}\n",
        "scalar f()\n{\n    return cbrt(a)\n       *limitedAlphal/(1 + alphaNuc() - limitedAlphal)\n    );\n}\n",
    );
    assert!(!hs.is_empty());
    assert!(
        hs.iter().all(|h| !h.rationale.contains("with comments")),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// openfoam 2a156443 MRFZones.H: a method deleted under a comment header is a
/// removal; "replaces 2 lines with comments" describes the bytes, not the
/// code. Both halves are said: what left, and what stands there now.
#[test]
fn a_declaration_deleted_under_a_comment_reads_as_a_removal() {
    let hs = hunks(
        "m.H",
        "class MRFZones\n{\npublic:\n        //- Prepare for mesh update\n        virtual void preUpdateMesh();\n};\n",
        "class MRFZones\n{\npublic:\n    // Mesh changes\n};\n",
    );
    assert!(
        hs.iter().any(|h| {
            h.rationale == "removes preUpdateMesh from MRFZones, replaced by comments"
        }),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// buildtheworld dcbe6892 build_py_env.xsh: a lone `"""` closing a string
/// assignment is code; only the textual check ever called it a comment.
#[test]
fn a_closing_triple_quote_of_an_assignment_is_not_a_comment() {
    let hs = hunks(
        "b.py",
        "def f():\n    patch = \"\"\"\n--- a/x\n+++ b/x\n\"\"\"\n    return patch\n",
        "def f():\n    return None\n",
    );
    assert!(!hs.is_empty());
    assert!(
        hs.iter().all(|h| !h.rationale.contains("comment")),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// curl c437d28c: a hunk on a comment inside a go import block still names the
/// statement's imports; only a hunk on an import's own row narrows to it.
#[test]
fn a_comment_inside_an_import_block_still_names_the_imports() {
    let block = |c: &str| format!("package doc\n\nimport (\n\t\"bytes\"\n\t// {c}\n\t\"os\"\n)\n");
    let hs = hunks("d.go", &block("keep sorted"), &block("keep them sorted"));
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert!(hs[0].rationale.contains("bytes"), "{}", hs[0].rationale);
}

/// tasks-3uv.12: an added file whose import and use land in one hunk still
/// links to the definition — the import "declares" the name, and subtracting
/// it left the only shape where both sit together with nothing to point at.
#[test]
fn an_import_used_in_its_own_hunk_still_links() {
    let out = run(json!({"changes": [
        {"path": "a.py", "old": "", "new": "def foo():\n    return 1\n"},
        {"path": "b.py", "old": "", "new": "from a import foo\n\n\ndef bar():\n    return foo()\n"}
    ]}));
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
    assert_eq!(out.edges[0].why, "def→use: foo");
    assert_eq!(out.clusters.len(), 1, "{:?}", out.clusters);
}

/// tasks-3uv.15: `full_context` asserted over a context-limited patch. The
/// engine cannot see either side, so it says so instead of reading every
/// hunk as formatting.
#[test]
fn a_patch_that_is_not_full_context_is_reported_not_believed() {
    let diff = "--- a/m.c\n+++ b/m.c\n@@ -10,3 +10,3 @@ int f(void)\n   int a = 1;\n-  return a;\n+  return a + 1;\n }\n@@ -40,3 +40,3 @@ int g(void)\n   int b = 2;\n-  return b;\n+  return b + 2;\n }\n";
    let out = run(json!({
        "changes": [{"path": "m.c", "diff": diff}],
        "options": {"full_context": true}
    }));
    assert!(
        out.problems
            .iter()
            .any(|p| p.contains("full_context was asserted")),
        "{:?}",
        out.problems
    );
    let f = &out.files[0];
    assert!(f.degraded);
    assert!(
        f.hunks
            .iter()
            .all(|h| !h.noise && h.rationale != "formatting only"),
        "{:?}",
        f.hunks
            .iter()
            .map(|h| (&h.rationale, h.noise))
            .collect::<Vec<_>>()
    );
}

/// tasks-3uv.19: one construct used throughout a file earns one note and a
/// count, not a note per hunk — a labeled sample judged 25 identical
/// `raw new/delete` notes in one commit worth reading once.
#[test]
fn a_rule_stops_repeating_itself_within_a_file() {
    let pad = |j: usize| {
        (0..6)
            .map(|k| format!("// filler {j}{k}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let body = |i: usize, s: &str| format!("void f{i}() {{ {s} }}\n{}", pad(i));
    let old: Vec<String> = (0..9).map(|i| body(i, "int x = 0;")).collect();
    let new: Vec<String> = (0..9)
        .map(|i| body(i, "int* p = new int(1); delete p;"))
        .collect();
    let hs = hunks("a.cpp", &old.join("\n\n"), &new.join("\n\n"));
    assert!(hs.len() > 3, "{} hunks", hs.len());
    let found: Vec<&ordo::model::Finding> = hs
        .iter()
        .flat_map(|h| &h.findings)
        .filter(|f| f.name == "raw-new-delete")
        .collect();
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found
            .last()
            .unwrap()
            .message
            .ends_with("(and 6 more in this file)"),
        "{}",
        found.last().unwrap().message
    );
}

/// tasks-3uv.19: `any` in a test file is the tool for the job — a cast that
/// feeds a validator what the types forbid. 53 labeled findings in test
/// paths, none of them warranted.
#[test]
fn any_in_a_test_file_is_not_flagged() {
    let src = "const a: any = 1;\n";
    let named = |p: &str| {
        hunks(p, "", src)
            .iter()
            .flat_map(|h| &h.findings)
            .any(|f| f.name == "any")
    };
    assert!(named("src/app.ts"), "the rule still fires outside tests");
    for p in [
        "src/tests/app.ts",
        "src/app.test.ts",
        "packages/x/__tests__/y.ts",
    ] {
        assert!(!named(p), "{p} is a test path");
    }
}

/// tasks-3uv.34: click 333c28d7 renamed `LazyFile` to `_LazyFile` in utils.py;
/// types.py dropped the old import in one hunk and added the new one in
/// another, and the deletion read "removes import LazyFile".
#[test]
fn a_dropped_import_whose_symbol_was_renamed_reads_as_a_rename() {
    let out = run(json!({"changes": [
        {"path": "utils.py",
         "old": "def safecall(f):\n    return f\n",
         "new": "def _safecall(f):\n    return f\n"},
        {"path": "types.py",
         "old": "from .utils import safecall\nimport os\n\n\ndef go():\n    return safecall(os)\n",
         "new": "import os\nfrom .utils import _safecall\n\n\ndef go():\n    return _safecall(os)\n"}
    ]}));
    let r = rationales(&out);
    assert!(
        r.iter().any(|x| x == "renames import safecall → _safecall"),
        "{r:?}"
    );
    assert!(r.iter().all(|x| !x.contains("removes import")), "{r:?}");
}

/// tasks-3uv.34: the rename wording is evidence-gated — a dropped import with
/// no def-side rename behind it is still a removal, whatever else the file
/// imports.
#[test]
fn a_dropped_import_with_no_rename_behind_it_still_removes() {
    let out = run(json!({"changes": [
        {"path": "types.py",
         "old": "from .utils import User\nimport os\n\n\ndef go():\n    return User(os)\n",
         "new": "import os\nfrom .utils import UserProfile\n\n\ndef go():\n    return UserProfile(os)\n"}
    ]}));
    let r = rationales(&out);
    assert!(r.iter().any(|x| x == "removes import User"), "{r:?}");
}

/// tasks-3uv.33: click 61b69e96 _termui_impl.py rewrote a dispatch; the
/// splitter broke at the construct boundary, so the old lines and their
/// replacement landed in two hunks and the deletion read "removes 3 lines".
#[test]
fn a_deletion_whose_replacement_is_next_door_says_so() {
    let hs = hunks(
        "p.py",
        "def pick(win):\n    cmd = lookup(win)\n    if win:\n        return tempfile([\"more\"])\n    return pipe([\"less\"])\n\n    log(cmd)\n",
        "def pick(win):\n    cmd = lookup(win)\n\n    log(cmd)\n    use_tmp = win\n    params = cmd[1:]\n    if use_tmp:\n        return tempfile(params)\n    return pipe(params)\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.len() > 1, "the deletion and its replacement split: {r:?}");
    assert!(r.contains(&"replaces 3 lines, added below"), "{r:?}");
}

/// tasks-3uv.37: openfoam d423f755 FieldField.C. A member template of a class
/// template is two `template<…>` heads on one definition; the inner one was
/// named after the outer's first parameter, `Field`, which then made this file
/// the definer of every `Field` in the change.
#[test]
fn a_member_template_is_not_named_after_its_template_parameter() {
    let hs = hunks(
        "F.C",
        "int a;\n",
        "int a;\ntemplate<template<class> class Field, class Type>\ntemplate<class Type2>\nint FieldField<Field, Type>::NewCalculatedType(const X& ff)\n{\n    return 1;\n}\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].defines, vec!["NewCalculatedType"]);
    assert_eq!(hs[0].rationale, "adds NewCalculatedType");
}
