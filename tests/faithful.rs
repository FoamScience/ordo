//! Rationales the labeled corpus caught lying (tasks-3uv.*): each test pins
//! one shape from a silver or gold hunk, reduced to the lines that mattered.
mod fixture;
use fixture::{hunks, rationales_of as rationales, run_file as one, run_json as run};
use ordo_diff::model::Category;
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
/// and the rationale listed all nine. Both paths bind `yaml` (a major version
/// is not the package name), so the line changes an import rather than adding
/// one called `v3`.
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
    assert_eq!(hs[0].rationale, "changes import yaml");
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
    let found: Vec<&ordo_diff::model::Finding> = hs
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

/// tasks-3uv.36: excalidraw ec070911 LibraryMenu.tsx added a whole
/// `useEffect(…)` and read "adds local target (6 uses)": a local inside the
/// block named as if it were the change.
#[test]
fn a_hunk_that_adds_a_whole_call_says_so() {
    let body = "  useEffect(() => {\n    const target = pick();\n    if (target) {\n      close(target);\n    }\n  }, [pick]);\n";
    let hs = hunks(
        "Menu.tsx",
        "export const Menu = memo(() => {\n  const a = 1;\n  return a;\n});\n",
        &format!("export const Menu = memo(() => {{\n  const a = 1;\n{body}  return a;\n}});\n"),
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "adds useEffect");
}

/// tasks-3uv.36: vue cd197456 and friends. A pure insertion edits nothing:
/// past a few lines it adds lines to its container, and "edits X" was judged
/// unfaithful on 13 of 15 labeled insertions of fifteen lines or more.
#[test]
fn a_large_insertion_into_a_function_adds_lines_to_it() {
    let lines: String = (0..6).map(|i| format!("    log({i});\n")).collect();
    let hs = hunks(
        "a.py",
        "def run():\n    start()\n    stop()\n",
        &format!("def run():\n    start()\n{lines}    stop()\n"),
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].rationale, "adds 6 lines to run");
}

/// …and up to three inserted lines still read as an edit: an entry in a
/// table, an item in a list.
#[test]
fn a_small_insertion_still_edits_its_container() {
    let at = |n: usize| {
        let lines: String = (0..n).map(|i| format!("    log({i})\n")).collect();
        hunks(
            "a.py",
            "def run():\n    start()\n    stop()\n",
            &format!("def run():\n    start()\n{lines}    stop()\n"),
        )[0]
        .rationale
        .clone()
    };
    assert_eq!(at(3), "edits run");
    assert_eq!(at(4), "adds 4 lines to run");
}

/// tasks-3uv.104: a pure deletion is placed by its new-side row like every
/// other hunk. Read at its old row, a deletion below a large insertion sat
/// inside it and joined its part of the change.
#[test]
fn a_deletion_below_an_insertion_is_placed_on_the_new_side() {
    let unchanged: String = (0..10).map(|i| format!("x{i} = {i}\n")).collect();
    let fresh: String = std::iter::once("def fresh():\n".to_string())
        .chain((0..28).map(|i| format!("    v{i} = {i}\n")))
        .chain(std::iter::once("    return 0\n".to_string()))
        .collect();
    let old = format!("{unchanged}def gone():\n    return 1\n");
    let new = format!("{fresh}{unchanged}");
    let out = run(json!({"changes": [{"path": "m.py", "old": old, "new": new}]}));
    assert_eq!(out.clusters.len(), 2, "{:?}", out.clusters);
}

/// tasks-3uv.167. The self-check: a rationale claim has to hold against the
/// facts the engine already has. Each case is a corpus hunk the scoreboard
/// found unfaithful, reduced to the lines that mattered.
fn rationale_at(hs: &[ordo_diff::model::HunkOut], row: usize) -> &str {
    &hs.iter()
        .find(|h| h.old_range[0] == row)
        .unwrap_or_else(|| panic!("no hunk at old row {row}: {hs:?}"))
        .rationale
}

/// cobra d1e9d85f command.go: a dropped line of a go import block was
/// "formatting only", because go imports carried the block's row, not their own.
#[test]
fn a_dropped_go_import_is_a_removal_not_formatting() {
    let hs = hunks(
        "command.go",
        "package cobra\n\nimport (\n\t\"io\"\n\t\"path/filepath\"\n\t\"sort\"\n)\n\nfunc f() { _ = filepath.Base(\"x\") }\n",
        "package cobra\n\nimport (\n\t\"io\"\n\t\"sort\"\n)\n\nfunc f() { g() }\n",
    );
    assert_eq!(rationale_at(&hs, 5), "removes import filepath");
}

/// execa 9de73355 lib/convert/readable.js: a name that left a multi-line
/// import for a line of its own moved; the residue is not "formatting only".
#[test]
fn a_name_moved_out_of_an_import_list_is_a_move() {
    let hs = hunks(
        "readable.js",
        "import {a} from './a.js';\nimport {\n\tcreateDeferred,\n\tsafeWait,\n} from './x.js';\n",
        "import {a} from './a.js';\nimport {createDeferred} from '../utils/deferred.js';\nimport {\n\tsafeWait,\n} from './x.js';\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.rationale == "moves import createDeferred"),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// ripgrep b610d1cb walk.rs: `sync::Arc` folded into `sync::{Arc, OnceLock}`.
#[test]
fn a_rust_use_folded_into_another_is_a_move() {
    let hs = hunks(
        "walk.rs",
        "use std::{\n    io,\n    sync::Arc,\n    sync::atomic::AtomicBool,\n};\n",
        "use std::{\n    io,\n    sync::atomic::AtomicBool,\n    sync::{Arc, OnceLock},\n};\n",
    );
    assert_eq!(rationale_at(&hs, 3), "moves import Arc");
}

/// excalidraw 8013eb5e actionLink.tsx: `changes import KEYS` named the name
/// the hunk kept and hid `getShortcutKey`, which it dropped.
#[test]
fn a_changed_import_names_what_left_not_what_stayed() {
    let hs = hunks(
        "actionLink.tsx",
        "import { KEYS, getShortcutKey } from \"@excalidraw/common\";\n\nexport const a = 1;\n",
        "import { KEYS } from \"@excalidraw/common\";\n\nexport const a = KEYS;\n",
    );
    assert_eq!(rationale_at(&hs, 1), "removes import getShortcutKey");
}

/// ripgrep bb8172fe glob.rs: rustfmt re-sorted the names of two imports; the
/// same names on both sides are not a change.
#[test]
fn a_sorted_import_list_is_formatting_only() {
    let hs = hunks(
        "glob.rs",
        "use std::path::{is_separator, Path};\n\nfn f() {}\n",
        "use std::path::{Path, is_separator};\n\nfn f() {}\n",
    );
    assert_eq!(rationale_at(&hs, 1), "formatting only");
}

/// a go import whose path moved to another module still binds the same name:
/// the line changed, so it is not formatting.
#[test]
fn a_changed_go_import_path_is_not_formatting() {
    let hs = hunks(
        "yaml_docs.go",
        "package doc\n\nimport (\n\t\"gopkg.in/yaml.v3\"\n)\n",
        "package doc\n\nimport (\n\t\"go.yaml.in/yaml/v3\"\n)\n",
    );
    assert_eq!(rationale_at(&hs, 4), "changes import yaml");
}

/// ripgrep be739c79 literal.rs: "and 7 more" after three names counted the
/// path words of every `use` as names; seven names are imported.
#[test]
fn an_import_list_counts_the_names_it_binds() {
    let hs = hunks(
        "literal.rs",
        "fn f() {}\n",
        "use std::cmp;\nuse std::mem;\n\nuse bstr::{BString, ByteVec};\nuse regex_syntax::hir::{self, Hir, HirKind};\n\nfn f() {}\n",
    );
    assert_eq!(
        hs[0].rationale,
        "adds import BString, ByteVec, Hir, and 4 more"
    );
}

/// junit5 5a29b878 AssertNotSame.java: an annotation added above a method the
/// file already had is not "adds assertNotSame".
#[test]
fn an_annotation_added_above_a_method_does_not_add_it() {
    let hs = hunks(
        "AssertNotSame.java",
        "class A {\n\n\tstatic void assertNotSame(Object a, Object b) {\n\t\treturn;\n\t}\n}\n",
        "class A {\n\n\t@SuppressWarnings(\"ReferenceEquality\")\n\tstatic void assertNotSame(Object a, Object b) {\n\t\treturn;\n\t}\n}\n",
    );
    assert_eq!(hs[0].rationale, "changes assertNotSame");
}

/// curl e13362c2 test_17_ssl_use.py: `@dataclass` on an existing class.
#[test]
fn a_decorator_added_to_a_class_does_not_add_it() {
    let hs = hunks(
        "test_ssl.py",
        "import os\n\n\nclass TLSDefs:\n    pass\n",
        "import os\n\n\n@dataclass\nclass TLSDefs:\n    pass\n",
    );
    assert_eq!(rationale_at(&hs, 4), "changes TLSDefs");
}

/// click 1a994adb exceptions.py: annotated attributes of a class body are the
/// class's, not locals of anything.
#[test]
fn a_class_attribute_annotation_is_not_a_local() {
    let hs = hunks(
        "exceptions.py",
        "class UsageError:\n    def show(self):\n        pass\n",
        "class UsageError:\n    param: str\n    ctx: int\n\n    def show(self):\n        pass\n",
    );
    assert!(hs.iter().all(|h| !h.rationale.contains("local")), "{hs:?}");
}

/// vue 30ba6473 release.js: two sibling blocks each declare their own
/// `pkgVersion`; the second is used twice, not four times.
#[test]
fn a_redeclared_local_counts_only_its_own_uses() {
    let hs = hunks(
        "release.js",
        "function publish() {\n  if (a) {\n    skip()\n  }\n  try {\n    run()\n  } catch (e) {\n    log(e)\n  }\n}\n",
        "function publish() {\n  if (a) {\n    const pkgVersion = `${x}`\n    log(pkgVersion)\n    all.push(pkgVersion)\n  }\n  try {\n    run()\n  } catch (e) {\n    const pkgVersion = `${y}`\n    log(pkgVersion)\n    all.push(pkgVersion)\n  }\n}\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.rationale == "adds local pkgVersion (2 uses)"),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// openfoam 2ec91f84 kOmegaSSTBase.C: the parameters of a signature being
/// retyped are declared there, not used from the function `F2` above.
#[test]
fn a_parameter_in_a_retyped_signature_is_not_a_use() {
    let f = |t: &str| {
        format!(
            "template<class M>\ntmp<{t}>\nK<M>::F2() const\n{{\n    return a;\n}}\n\n\
             template<class M>\ntmp<{t}>\nK<M>::epsilonByk\n(\n    const {t}& F1,\n    const {t}& F2\n) const\n{{\n    return b;\n}}\n"
        )
    };
    let hs = hunks("k.C", &f("volScalarField"), &f("volInternalScalarField"));
    assert!(
        hs.iter().all(|h| !h.rationale.starts_with("uses F2")),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// buildtheworld e9d91fe2 used_repos.yaml: `user` is a key many entries have;
/// naming the one file it came from is a guess.
#[test]
fn a_repeated_yaml_key_is_not_claimed_moved() {
    let out = run(json!({"changes": [
        {"path": "30.yaml",
         "old": "- name: a\n  user: pyyaml-team\n- name: b\n",
         "new": "- name: b\n"},
        {"path": "used.yaml",
         "old": "- name: c\n",
         "new": "- name: c\n- name: d\n  user: pyyaml-team\n- name: e\n  user: pyyaml-team\n"}
    ]}));
    let r = rationales(&out);
    assert!(r.iter().all(|x| !x.starts_with("moves")), "{r:?}");
}

/// curl e1450d8f-era CMake/CurlTests.c: re-indenting an `#include` in place
/// moves nothing.
#[test]
fn a_reindented_include_is_formatting_only() {
    let hs = hunks(
        "CurlTests.c",
        "#ifdef HAVE_X\n  #include <sys/types.h>\n#endif\nint main(void) { return 0; }\n",
        "#ifdef HAVE_X\n#include <sys/types.h>\n#endif\nint main(void) { return 0; }\n",
    );
    assert_eq!(rationale_at(&hs, 2), "formatting only");
}

/// curl 7e7ee16d cli_ws_write_err.c: the hunk that opens a new `#ifndef`
/// guard adds it.
#[test]
fn a_hunk_that_opens_a_guard_adds_it() {
    let func = |n: usize| {
        let body: String = (0..30).map(|i| format!("  x{n}_{i} = {i};\n")).collect();
        format!("static int f{n}(void)\n{{\n{body}  return 0;\n}}\n\n")
    };
    let new = format!(
        "#include \"first.h\"\n\n#ifndef CURL_DISABLE_WEBSOCKETS\n\n{}{}#endif\n",
        func(1),
        func(2)
    );
    let hs = hunks("cli_ws_write_err.c", "", &new);
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("edits #ifndef")), "{r:?}");
}

/// tasks-3uv.167 branch review. A return annotation sits in a signature's
/// header too, and what it names is a use.
#[test]
fn a_return_annotation_still_uses_the_class_added_above() {
    let hs = hunks(
        "m.py",
        "def f(\n    x,\n) -> int:\n    return 1\n",
        "class Foo:\n    pass\n\n\ndef f(\n    x,\n) -> Foo:\n    return 1\n",
    );
    assert_eq!(rationale_at(&hs, 3), "uses Foo, added above");
}

/// a glob binds no name: `super` and `crate` are path words
#[test]
fn a_glob_import_binds_no_path_word() {
    for use_ in ["use super::*;", "use crate::*;", "use a::b::*;"] {
        let hs = hunks("lib.rs", "fn f() {}\n", &format!("{use_}\n\nfn f() {{}}\n"));
        let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
        assert!(
            r.iter()
                .all(|x| !["super", "crate", "b"].iter().any(|w| x.ends_with(w))),
            "{use_}: {r:?}"
        );
    }
}

/// the same words in a different pairing are not a reorder
#[test]
fn swapping_names_between_use_statements_is_not_formatting() {
    let hs = hunks(
        "lib.rs",
        "use a::b;\nuse c::d;\n\nfn f() {}\n",
        "use a::d;\nuse c::b;\n\nfn f() {}\n",
    );
    assert_ne!(rationale_at(&hs, 1), "formatting only");
}

/// php scopes a variable to its function: the second assignment is the same
/// variable, so its uses are all of them
#[test]
fn a_php_variable_assigned_in_two_blocks_counts_every_use() {
    let fill = "    echo 0;\n".repeat(6);
    let old = format!("<?php\nfunction p($a) {{\n    if ($a) {{\n        skip();\n    }}\n{fill}    if (!$a) {{\n        skip();\n    }}\n}}\n");
    let new = format!("<?php\nfunction p($a) {{\n    if ($a) {{\n        $v = 1;\n        echo $v;\n        log($v);\n    }}\n{fill}    if (!$a) {{\n        $v = 2;\n        echo $v;\n        log($v);\n    }}\n}}\n");
    let hs = hunks("a.php", &old, &new);
    assert!(
        hs.iter().any(|h| h.rationale == "adds local $v (4 uses)"),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// a deleted `other::foo` line is not the import `a::foo` that is still there
#[test]
fn a_dropped_duplicate_name_is_not_a_move() {
    let hs = hunks(
        "lib.rs",
        "use std::{\n    a::foo,\n    other::foo,\n    io,\n};\n",
        "use std::{\n    a::foo,\n    io,\n};\n",
    );
    assert!(
        hs.iter().all(|h| h.rationale != "moves import foo"),
        "{:?}",
        hs.iter().map(|h| &h.rationale).collect::<Vec<_>>()
    );
}

/// A large insertion is cut at its definitions, and a decorated one is cut
/// above its decorators and again at its `def`: the piece between holds the
/// decorator alone, and that is all it adds.
fn decorated_test(n: usize, filler: usize) -> String {
    let pad: String = (0..filler).map(|i| format!("    v{i} = {i}\n")).collect();
    format!("@pytest.mark.skipif(WIN, reason=\"shell\")\ndef test_{n}(monkeypatch):\n{pad}    assert 1\n\n\n")
}

const PYTEST_OLD: &str = "import pytest\n\n\ndef test_a():\n    assert 1\n";

fn texts(hs: &[ordo_diff::model::HunkOut]) -> Vec<&str> {
    hs.iter().map(|h| h.rationale.as_str()).collect()
}

#[test]
fn decorators_of_an_added_test_are_not_the_test() {
    let new = format!(
        "{PYTEST_OLD}\n\n{}{}",
        decorated_test(1, 40),
        decorated_test(2, 40)
    );
    let hs = hunks("test_termui.py", PYTEST_OLD, &new);
    let r = texts(&hs);
    assert!(r.contains(&"adds decorator to test_2"), "{r:?}");
    assert!(r.contains(&"adds test_2"), "{r:?}");
    assert!(
        r.iter()
            .all(|t| !t.starts_with("moves") && *t != "adds cli"),
        "{r:?}"
    );
}

/// A decorator added above a definition the file already had still reads
/// "changes", not "adds decorator", however many decorators stack on it.
#[test]
fn a_decorator_added_above_an_existing_test_changes_it() {
    for stack in [1, 5] {
        let decorators: String = (0..stack)
            .map(|i| format!("@pytest.mark.parametrize(\n    \"a{i}\",\n    [1, 2],\n)\n"))
            .collect();
        let hs = hunks(
            "test_termui.py",
            PYTEST_OLD,
            &format!("import pytest\n\n\n{decorators}def test_a():\n    assert 1\n"),
        );
        assert_eq!(texts(&hs), ["changes test_a"], "{stack} decorators");
    }
}

/// openfoam b4976af6 boundBox.H: a specialisation whose only change is a
/// typo in its name has an empty body, so there is no body to match on; it is
/// a rename, not an added type.
#[test]
fn a_typo_fix_in_an_empty_bodied_type_is_a_rename() {
    let hs = hunks(
        "b.H",
        "template<class T>\nstruct scaleable : public std::false_type {};\n\n\
         template<>\nstruct scaleable<boundBox> : public std::true_type {};\n",
        "template<class T>\nstruct scaleable : public std::false_type {};\n\n\
         template<>\nstruct scalable<boundBox> : public std::true_type {};\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(r, ["renames scaleable<boundBox> → scalable<boundBox>"]);
}

/// Two empty-bodied specialisations whose names differ by a counter are two
/// types, not one respelled.
#[test]
fn a_counter_in_an_empty_bodied_type_is_not_a_typo() {
    let hs = hunks(
        "c.H",
        "struct Widget_Impl1 : public Base {};\n",
        "struct Widget_Impl2 : public Base {};\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("renames")), "{r:?}");
}

/// A short name is not replaced inside other identifiers when the body is
/// compared: `get` → `fetch` leaves `get_value` alone, so a pure rename stays
/// a pure rename.
#[test]
fn a_short_renamed_name_is_not_swapped_inside_other_names() {
    let hs = hunks(
        "g.py",
        "def get(x):\n    y = get_value(x)\n    return y + get_other(x)\n",
        "def fetch(x):\n    y = get_value(x)\n    return y + get_other(x)\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(r, ["renames get → fetch"]);
}

/// A short heading is renamed even when its letters appear inside another
/// line of the new section.
#[test]
fn a_heading_word_inside_a_line_does_not_block_a_rename() {
    let hs = hunks(
        "R.md",
        "# Doc\n\n## Fix\n\nFirst line of the notes.\nSecond line of the notes.\nThird line of the notes.\nFourth line of the notes.\n",
        "# Doc\n\n## Repair\n\nFirst line of the notes.\nSecond line of the notes.\nThird line of the notes.\nFourth line of the notes.\nA Fix was applied.\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(
        r.iter().any(|x| x.starts_with("renames Fix → Repair")),
        "{r:?}"
    );
}

/// lazy dbffad6f fragments.lua: the rename also gave the function a body it
/// did not have, which "renames F:add → M:add" alone hides.
#[test]
fn a_rename_that_also_changes_the_body_says_so() {
    let hs = hunks(
        "f.lua",
        "function F:add(plugin)\n  local id = self.next\n  self.next = id + 1\n  self.fragments[id] = plugin\n  return id\nend\n",
        "function M:add(plugin)\n  if self.plugins[plugin] then\n    return self.fragments[self.plugins[plugin]]\n  end\n  local id = self.next\n  self.next = id + 1\n  self.fragments[id] = plugin\n  return id\nend\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(
        r.iter()
            .any(|x| x.starts_with("renames F:add → M:add; edits")),
        "{r:?}"
    );
}

/// junit5 e6fa8005: `printDetail` became `appendDetail` and took new
/// parameters; the rename is not the whole of the change.
#[test]
fn a_rename_that_also_changes_the_signature_says_so() {
    let hs = hunks(
        "L.java",
        "class L {\n  private void printDetail(Style style, String detail) {\n    printf(style, detail);\n    printf(NONE, \"%n\");\n  }\n}\n",
        "class L {\n  private void appendDetail(StringBuilder output, Style style, String detail) {\n    printf(style, detail);\n    printf(NONE, \"%n\");\n  }\n}\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(
        r.iter().any(|x| x
            .starts_with("renames printDetail → appendDetail; changes signature of appendDetail")),
        "{r:?}"
    );
}

/// nvm 0d533816 tests.yml: `include` became `exclude` with different entries,
/// which inverts the matrix; two keys with some lines in common are not one
/// key renamed.
#[test]
fn a_yaml_key_with_a_different_body_is_not_renamed() {
    let hs = hunks(
        "t.yml",
        "matrix:\n  include:\n    - shell: bash\n    - shell: sh\n",
        "matrix:\n  exclude:\n    - shell: sh\n      suite: install\n    - shell: dash\n      suite: install\n    - shell: zsh\n      suite: install\n    - shell: bash\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("renames")), "{r:?}");
}

/// ripgrep bdf01f46 CHANGELOG.md: the release heading got its date and a new
/// `TBD` section went above it; `TBD` is added, not the old heading renamed.
#[test]
fn a_new_changelog_section_is_not_a_renamed_heading() {
    let hs = hunks(
        "CHANGELOG.md",
        "14.1.0 (TBD)\n============\nBug fixes:\n\n* [BUG #2891](https://example.com):\n  Fix a thing.\n",
        "TBD\n===\nUnreleased changes. Release notes have not yet been written.\n\n14.1.0 (2024-09-08)\n===================\nBug fixes:\n\n* [BUG #2891](https://example.com):\n  Fix a thing.\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.contains("→ TBD")), "{r:?}");
}
