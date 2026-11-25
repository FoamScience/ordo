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

/// A rust attribute stays with the item it marks: a large insertion is cut
/// above the attributes, so no piece holds attribute rows alone and none
/// claims the next item's attribute as its own tail.
fn rust_body(name: &str) -> String {
    let pad: String = (0..40)
        .map(|i| format!("    let a{i} = {name}_{i}();\n"))
        .collect();
    format!("fn {name}() {{\n{pad}}}\n")
}

fn rust_test(name: &str) -> String {
    format!("#[test]\n{}", rust_body(name))
}

fn rust_hunks(changes: &[(&str, &str, &str)]) -> Vec<String> {
    let changes: Vec<_> = changes
        .iter()
        .map(|(path, old, new)| json!({"path": path, "old": old, "new": new}))
        .collect();
    rationales(&run(json!({ "changes": changes })))
}

#[test]
fn attributes_of_added_rust_tests_stay_with_them() {
    let old = rust_body("base");
    for attrs in [
        "#[test]\n",
        "#[test]\n#[ignore]\n",
        "#[cfg_attr(windows, ignore)]\n",
        "#[cfg_attr(\n    windows,\n    ignore\n)]\n",
        "#[test] // note\n",
    ] {
        let case = |name: &str| format!("{attrs}{}", rust_body(name));
        let new = format!("{old}\n{}\n{}", case("case_1"), case("case_2"));
        assert_eq!(
            rust_hunks(&[("a.rs", &old, &new)]),
            ["adds case_1", "adds case_2"],
            "{attrs:?}"
        );
    }
}

#[test]
fn attributes_of_many_one_line_tests_make_no_extra_cards() {
    let tests: String = (0..30)
        .map(|i| format!("    #[test]\n    fn t{i}() {{}}\n"))
        .collect();
    let new = format!("fn z() {{}}\n\n#[cfg(test)]\nmod tests {{\n{tests}}}\n");
    let r = rust_hunks(&[("a.rs", "fn z() {}\n", &new)]);
    assert_eq!(r.len(), 31, "{r:?}");
    assert_eq!(r[..3], ["adds tests", "adds t0", "adds t1"]);
}

#[test]
fn a_moved_rust_test_is_not_given_a_decorator() {
    let (k, q) = (rust_body("k"), rust_body("q"));
    let r = rust_hunks(&[
        ("a.rs", &format!("{k}\n{}", rust_test("case_5")), &k),
        (
            "b.rs",
            &q,
            &format!("{q}\n{}\n{}", rust_test("case_5"), rust_test("case_6")),
        ),
    ]);
    assert_eq!(
        r,
        [
            "moves case_5 to b.rs",
            "moves case_5 from a.rs",
            "adds case_6"
        ]
    );
}

#[test]
fn a_comment_between_an_attribute_and_its_item_keeps_them_together() {
    let old = rust_body("base");
    for between in ["/// doc\n", "// note\n", "/* note */\n", "/** doc */\n"] {
        let second = format!("#[test]\n{between}{}", rust_body("case_2"));
        let new = format!("{old}\n{}\n{second}", rust_test("case_1"));
        assert_eq!(
            rust_hunks(&[("a.rs", &old, &new)]),
            ["adds case_1", "adds case_2"],
            "{between:?}"
        );
    }
}

#[test]
fn derive_on_an_added_struct_stays_with_it() {
    let old = rust_body("base");
    let body: String = (0..40).map(|i| format!("    f{i}: u8,\n")).collect();
    let new = format!(
        "{old}\n#[derive(Debug)]\nstruct A {{\n{body}}}\n\n#[derive(Clone)]\nstruct B {{\n{body}}}\n"
    );
    assert_eq!(
        rust_hunks(&[("a.rs", &old, &new)]),
        ["adds type A", "adds type B"]
    );
}

#[test]
fn an_attribute_added_above_an_existing_rust_fn_keeps_its_wording() {
    let hs = hunks(
        "t.rs",
        "fn case_a() {\n    assert!(true);\n}\n",
        "#[test]\n#[ignore]\nfn case_a() {\n    assert!(true);\n}\n",
    );
    assert_eq!(texts(&hs), ["uses ignore, test"]);
}

#[test]
fn an_inner_attribute_is_not_an_item_attribute() {
    let old = rust_body("base");
    let new = format!(
        "{old}\n#![allow(dead_code)]\n{}\n{}",
        rust_body("case_1"),
        rust_body("case_2")
    );
    assert_eq!(
        rust_hunks(&[("a.rs", &old, &new)]),
        ["adds comment", "adds case_1", "adds case_2"]
    );
}

/// No grammar reads c#, so its attribute rows stay plain added lines.
const CS_OLD: &str = "class T {\n    void CaseA() {\n        Assert.True(true);\n    }\n}\n";

#[test]
fn a_csharp_attribute_is_a_plain_added_line() {
    let hs = hunks(
        "T.cs",
        CS_OLD,
        "class T {\n    [Fact]\n    void CaseA() {\n        Assert.True(true);\n    }\n}\n",
    );
    assert_eq!(texts(&hs), ["adds 1 line"]);
}

#[test]
fn a_csharp_collection_expression_line_is_a_plain_added_line() {
    let hs = hunks(
        "T.cs",
        CS_OLD,
        "class T {\n    int[] xs = [1, 2, 3];\n    void CaseA() {\n        Assert.True(true);\n    }\n}\n",
    );
    assert_eq!(texts(&hs), ["adds 1 line"]);
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

/// curl e1450d8f: a test source that merely calls the new `CURL_CSTRLEN`
/// macro uses it; nothing in it tests the macro.
#[test]
fn a_test_using_a_macro_does_not_test_it() {
    let out = run(json!({"changes": [
        {"path": "lib/curl_setup.h",
         "old": "#define A 1\n",
         "new": "#define A 1\n#define CURL_CSTRLEN(s) (sizeof(s) - 1)\n"},
        {"path": "tests/libtest/lib654.c",
         "old": "int f(void)\n{\n  return 1;\n}\n",
         "new": "int f(void)\n{\n  return CURL_CSTRLEN(\"abc\");\n}\n"}
    ]}));
    let r = rationales(&out);
    assert_eq!(
        r,
        [
            "adds CURL_CSTRLEN, used in tests/libtest/lib654.c",
            "uses CURL_CSTRLEN, added in lib/curl_setup.h"
        ]
    );
}

/// cobra 652c755d: a helper in a test file that wraps a call in `CheckErr`
/// is using the error handler, not testing it.
#[test]
fn a_test_helper_calling_a_function_does_not_test_it() {
    let out = run(json!({"changes": [
        {"path": "cobra.go",
         "old": "package cobra\n\nfunc other() {}\n",
         "new": "package cobra\n\nfunc CheckErr(msg interface{}) {\n\tif msg != nil {\n\t\tpanic(msg)\n\t}\n}\n\nfunc other() {}\n"},
        {"path": "bash_completions_test.go",
         "old": "package cobra\n\nfunc runShellCheck(s string) {\n\tcmd := exec.Command(\"bash\", \"-c\", s)\n\tcmd.Run()\n}\n",
         "new": "package cobra\n\nfunc runShellCheck(s string) {\n\tcmd := exec.Command(\"bash\", \"-c\", s)\n\tCheckErr(cmd.Run())\n}\n"}
    ]}));
    let r = rationales(&out);
    assert_eq!(
        r,
        [
            "adds CheckErr, used in bash_completions_test.go",
            "uses CheckErr, added in cobra.go"
        ]
    );
}

/// `def` adds the callee; `test` edits a test-side file that already existed
fn calls_in_test(def: (&str, &str, &str), test: (&str, &str, &str)) -> Vec<String> {
    let (dp, dold, dnew) = def;
    let (tp, told, tnew) = test;
    rationales(&run(json!({"changes": [
        {"path": dp, "old": dold, "new": dnew},
        {"path": tp, "old": told, "new": tnew}
    ]})))
}

fn assert_tests(r: &[String], name: &str) {
    assert!(
        r.iter().any(|x| x.starts_with(&format!("tests {name} "))),
        "{r:?}"
    );
}

fn assert_uses(r: &[String], name: &str, defined_in: &str, used_in: &str) {
    assert_eq!(
        r,
        [
            format!("adds {name}, used in {used_in}"),
            format!("uses {name}, added in {defined_in}")
        ]
    );
}

/// a `#[test]` fn keeps the claim whatever it is called
#[test]
fn a_rust_test_attribute_keeps_the_claim() {
    let r = calls_in_test(
        (
            "src/lib.rs",
            "pub fn other() {}\n",
            "pub fn other() {}\n\npub fn parse(x: &str) -> u32 {\n    x.len() as u32\n}\n",
        ),
        (
            "tests/cli.rs",
            "#[test]\nfn parses_flags() {\n    other();\n}\n",
            "#[test]\nfn parses_flags() {\n    other();\n    assert_eq!(parse(\"ab\"), 2);\n}\n",
        ),
    );
    assert_tests(&r, "parse");
}

/// an `it(...)` block is a test though its name is prose
#[test]
fn a_describe_it_block_keeps_the_claim() {
    let r = calls_in_test(
        (
            "src/args.ts",
            "export function other() {}\n",
            "export function other() {}\nexport function parseArgs(x: string) {\n  return x;\n}\n",
        ),
        (
            "src/args.test.ts",
            "describe('args', () => {\n  it('parses', () => {\n    other();\n  });\n});\n",
            "describe('args', () => {\n  it('parses', () => {\n    other();\n    parseArgs('a');\n  });\n});\n",
        ),
    );
    assert_tests(&r, "parseArgs");
}

/// Go `TestX`, `BenchmarkX` and `ExampleX` are tests by convention
#[test]
fn go_test_benchmark_and_example_functions_keep_the_claim() {
    for f in ["TestParse", "BenchmarkParse", "ExampleParse"] {
        let r = calls_in_test(
            (
                "parse.go",
                "package p\n\nfunc other() {}\n",
                "package p\n\nfunc other() {}\n\nfunc Parse(s string) int {\n\treturn len(s)\n}\n",
            ),
            (
                "parse_test.go",
                &format!("package p\n\nfunc {f}() {{\n\tother()\n}}\n"),
                &format!("package p\n\nfunc {f}() {{\n\tother()\n\tParse(\"ab\")\n}}\n"),
            ),
        );
        assert_tests(&r, "Parse");
    }
}

/// a helper is not a test because its name contains the letters: `contest_setup`
/// and `latest_version` call the function, they do not test it
#[test]
fn a_helper_whose_name_contains_test_does_not_test() {
    for helper in ["contest_setup", "latest_version"] {
        let r = calls_in_test(
            (
                "a.py",
                "def other():\n    return 0\n",
                "def other():\n    return 0\n\n\ndef parse(s):\n    return len(s)\n",
            ),
            (
                "tests/test_a.py",
                &format!("def {helper}():\n    return other()\n"),
                &format!("def {helper}():\n    other()\n    return parse('ab')\n"),
            ),
        );
        assert_uses(&r, "parse", "a.py", "tests/test_a.py");
    }
}

/// `macro_rules!` is a macro like a `#define`: a test calling it uses it
#[test]
fn a_test_using_a_rust_macro_does_not_test_it() {
    let r = calls_in_test(
        (
            "src/lib.rs",
            "pub fn other() {}\n",
            "pub fn other() {}\n\nmacro_rules! cstrlen {\n    ($s:expr) => {\n        $s.len()\n    };\n}\n",
        ),
        (
            "tests/cli.rs",
            "#[test]\nfn lengths() {\n    other();\n}\n",
            "#[test]\nfn lengths() {\n    other();\n    assert_eq!(cstrlen!(\"ab\"), 2);\n}\n",
        ),
    );
    assert_uses(&r, "cstrlen", "src/lib.rs", "tests/cli.rs");
}

/// without a marker, `test_` is a test's name only where the language reads it
/// so: an unmarked rust `fn test_helper` or go `testHelper` is a helper
#[test]
fn a_test_named_function_in_a_language_without_the_convention_is_a_helper() {
    let r = calls_in_test(
        (
            "src/lib.rs",
            "pub fn other() {}\n",
            "pub fn other() {}\n\npub fn parse(x: &str) -> u32 {\n    x.len() as u32\n}\n",
        ),
        (
            "tests/cli.rs",
            "fn test_helper() {\n    other();\n}\n",
            "fn test_helper() {\n    other();\n    parse(\"a\");\n}\n",
        ),
    );
    assert_uses(&r, "parse", "src/lib.rs", "tests/cli.rs");
    let r = calls_in_test(
        (
            "parse.go",
            "package p\n\nfunc other() {}\n",
            "package p\n\nfunc other() {}\n\nfunc Parse(s string) int {\n\treturn len(s)\n}\n",
        ),
        (
            "parse_test.go",
            "package p\n\nfunc testHelper() {\n\tother()\n}\n",
            "package p\n\nfunc testHelper() {\n\tother()\n\tParse(\"a\")\n}\n",
        ),
    );
    assert_uses(&r, "Parse", "parse.go", "parse_test.go");
}

/// python's unittest reads `test_x` as a test with no marker
#[test]
fn a_python_unittest_method_keeps_the_claim() {
    let r = calls_in_test(
        (
            "a.py",
            "def other():\n    return 0\n",
            "def other():\n    return 0\n\n\ndef parse(s):\n    return len(s)\n",
        ),
        (
            "tests/test_a.py",
            "class T(unittest.TestCase):\n    def test_a(self):\n        other()\n",
            "class T(unittest.TestCase):\n    def test_a(self):\n        other()\n        parse('a')\n",
        ),
    );
    assert_tests(&r, "parse");
}

/// gtest's `TEST(a, b) {…}` and boost's `BOOST_AUTO_TEST_CASE(x) {…}` parse as
/// functions named for the macro; they are tests, and a plain helper is not
#[test]
fn c_plus_plus_test_macros_keep_the_claim() {
    let def = (
        "src/parse.cc",
        "int other() { return 0; }\n",
        "int other() { return 0; }\n\nint parse(const char *s) { return 1; }\n",
    );
    for (old, new) in [
        (
            "TEST(Parse, Basic) {\n  other();\n}\n",
            "TEST(Parse, Basic) {\n  other();\n  EXPECT_EQ(parse(\"ab\"), 1);\n}\n",
        ),
        (
            "BOOST_AUTO_TEST_CASE(parses) {\n  other();\n}\n",
            "BOOST_AUTO_TEST_CASE(parses) {\n  other();\n  BOOST_CHECK(parse(\"ab\") == 1);\n}\n",
        ),
    ] {
        assert_tests(
            &calls_in_test(def, ("tests/parse_test.cc", old, new)),
            "parse",
        );
    }
    let r = calls_in_test(
        def,
        (
            "tests/parse_test.cc",
            "void check_all() {\n  other();\n}\n",
            "void check_all() {\n  other();\n  parse(\"ab\");\n}\n",
        ),
    );
    assert_uses(&r, "parse", "src/parse.cc", "tests/parse_test.cc");
}

/// lazy dbffad6f fragments.lua: `local F = {}` became `local M = {}` plus a
/// `next_id` function moved down from the top of the file. The function is
/// wholly added by the hunk, so the hunk cannot be an edit of it.
#[test]
fn a_definition_moved_into_the_hunk_is_added_not_edited() {
    assert_eq!(moved_next_id(""), ["adds next_id"]);
}

/// The same change with a second `next_id` that stays where it was: the one
/// moved into the hunk is still added by it.
#[test]
fn a_moved_definition_is_added_even_when_its_name_survives_elsewhere() {
    let survivor = "function M.wrap()\n  local function next_id()\n    return 1\n  end\n  return next_id\nend\n";
    assert_eq!(moved_next_id(survivor), ["adds next_id"]);
}

/// The rationales of the hunks defining `next_id` when the file also ends in
/// `extra`.
fn moved_next_id(extra: &str) -> Vec<String> {
    let old = format!("{}{extra}", "local M = {}\nM._fid = 0\nlocal function next_id()\n  M._fid = M._fid + 1\n  return M._fid\nend\n--- A fragment can be a dependency (dependencies) or a child (specs) of another fragment.\n---@class LazyFragments\n---@field fragments table<number, LazyFragment>\n---@field frag_stack number[]\n---@field dep_stack number[]\n---@field dirty table<number, boolean>\n---@field spec LazySpecLoader\nlocal F = {}\nfunction M.new(spec)\n  local self = setmetatable({}, { __index = F })\n  self.fragments = {}\n  self.frag_stack = {}\n  self.dep_stack = {}\n  self.dirty = {}\nend\n");
    let new = format!("{}{extra}", "--- A fragment can be a dependency (dependencies) or a child (specs) of another fragment.\n---@class LazyFragments\n---@field fragments table<number, LazyFragment>\n---@field frag_stack number[]\n---@field dep_stack number[]\n---@field dirty table<number, boolean>\nlocal M = {}\nM._fid = 0\nlocal function next_id()\n  M._fid = M._fid + 1\n  return M._fid\nend\nfunction M.new(spec)\n  local self = setmetatable({}, { __index = M })\n  self.fragments = {}\n  self.frag_stack = {}\n  self.dep_stack = {}\n  self.dirty = {}\n  self.plugins = {}\nend\n");
    hunks("f.lua", &old, &new)
        .iter()
        .filter(|h| h.defines.iter().any(|d| d == "next_id"))
        .map(|h| h.rationale.clone())
        .collect()
}

/// junit5 b5f17668 EvaluatedArgumentSet.java: `set.getName()` reaches
/// ArgumentSet's method, not the `getName` this class defines.
#[test]
fn a_java_call_on_another_object_is_not_a_use_of_a_local_method() {
    let hs = hunks(
        "E.java",
        "class E {\n\tOptional<String> getName() {\n\t\treturn this.name;\n\t}\n\n\tprivate static Optional<String> determineName(Arguments arguments) {\n\t\tif (arguments instanceof ArgumentSet set) {\n\t\t\treturn Optional.of(set.getName());\n\t\t}\n\t\treturn Optional.empty();\n\t}\n}\n",
        "class E {\n\t@Nullable\n\tString getName() {\n\t\treturn this.name;\n\t}\n\n\tprivate static @Nullable String determineName(Arguments arguments) {\n\t\tif (arguments instanceof ArgumentSet set) {\n\t\t\treturn set.getName();\n\t\t}\n\t\treturn null;\n\t}\n}\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(
        r,
        [
            "changes signature of getName",
            "changes signature of determineName",
            "edits E.determineName",
            "edits E.determineName",
        ],
        "{r:?}"
    );
}

/// openfoam 627e2f90 lduMatrix.H: a constructor prototype naming its parameter
/// `interfaceBouCoeffs` declares that name, so the accessor of the same name
/// is not used by it.
#[test]
fn a_prototype_parameter_is_not_a_use_of_a_method_of_the_same_name() {
    let hs = hunks(
        "l.H",
        "class solver\n{\n    const FieldField<Field, scalar>& interfaceBouCoeffs_;\n\n    solver\n    (\n        const word& fieldName,\n        const FieldField<Field, scalar>& interfaceBouCoeffs,\n        const dictionary& solverControls\n    );\n\n    const FieldField<Field, scalar>& interfaceBouCoeffs() const\n    {\n        return interfaceBouCoeffs_;\n    }\n};\n",
        "class solver\n{\n    const Field<Field<scalar>>& interfaceBouCoeffs_;\n\n    solver\n    (\n        const word& fieldName,\n        const Field<Field<scalar>>& interfaceBouCoeffs,\n        const dictionary& solverControls\n    );\n\n    const Field<Field<scalar>>& interfaceBouCoeffs() const\n    {\n        return interfaceBouCoeffs_;\n    }\n};\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(
        r.iter()
            .filter(|x| x.contains("interfaceBouCoeffs"))
            .collect::<Vec<_>>(),
        [&"changes signature of interfaceBouCoeffs"],
        "{r:?}"
    );
}

/// The rationale of `helper` in a class whose `run` calls it as `call(args)`;
/// `helper` gains a parameter and the call gains an argument.
fn java_helper_rationale(call: impl Fn(&str) -> String) -> String {
    let class = |params: &str, args: &str| {
        format!(
            "class Outer {{\n\tvoid helper(int a{params}) {{\n\t\twork(a);\n\t}}\n\n\tvoid run() {{\n\t\t{}\n\t}}\n}}\n",
            call(args)
        )
    };
    let hs = hunks("Outer.java", &class("", "1"), &class(", int b", "1, 2"));
    let of_helper: Vec<&str> = hs
        .iter()
        .filter(|h| h.defines.iter().any(|d| d == "helper"))
        .map(|h| h.rationale.as_str())
        .collect();
    assert_eq!(of_helper.len(), 1, "{hs:?}");
    of_helper[0].to_string()
}

const LINKED: &str = "changes signature of helper, used by Outer.run below";
const UNLINKED: &str = "changes signature of helper";

/// `this`, `super`, a type name and a qualified `this` all reach this file's
/// own `helper`.
#[test]
fn a_java_call_on_this_or_a_type_stays_linked_to_the_local_method() {
    for receiver in [
        "this.",
        "super.",
        "Outer.",
        "Outer.this.",
        "Outer.super.",
        "Outer.Inner.",
    ] {
        assert_eq!(
            java_helper_rationale(|a| format!("{receiver}helper({a});")),
            LINKED,
            "{receiver}"
        );
    }
    assert_eq!(java_helper_rationale(|a| format!("helper({a});")), LINKED);
}

/// A lowercase receiver is some other object whose type this file does not
/// say; its `helper` is not the one defined here.
#[test]
fn a_java_call_on_a_variable_is_not_linked_to_the_local_method() {
    for receiver in ["set.", "this.items.", "items.get(0)."] {
        assert_eq!(
            java_helper_rationale(|a| format!("{receiver}helper({a});")),
            UNLINKED,
            "{receiver}"
        );
    }
}

/// One plain mention is enough to link the group; chaining off a linked call
/// does not unlink it.
#[test]
fn a_bare_mention_or_a_linked_chain_keeps_the_link() {
    assert_eq!(
        java_helper_rationale(|a| format!("helper({a});\n\t\tset.helper({a});")),
        LINKED
    );
    assert_eq!(
        java_helper_rationale(|a| format!("this.helper({a}).helper({a});")),
        LINKED
    );
    assert_eq!(
        java_helper_rationale(|a| format!("set.helper({a}).helper({a});")),
        UNLINKED
    );
}

/// openfoam cd92dfdf BinaryExpression.H: `argB(args) ...` is a call in a pack
/// expansion that parses as a parameter pack, and must not read as declaring
/// `args`-style parameters: `compare` really does use `argB`.
#[test]
fn a_call_in_a_pack_expansion_is_a_use_not_a_parameter() {
    let hs = hunks(
        "b.H",
        "template<class Type>\nconst Type& argB(const Type& t)\n{\n    return t;\n}\n\ntemplate<class... Args>\nbool compare(const Args&... args)\n{\n    return op(argB(args) ...);\n}\n",
        "template<class Type>\nType& argB(Type& t)\n{\n    return t;\n}\n\ntemplate<class... Args>\nbool compare(const Args&... args)\n{\n    return op(argB(args) ..., 1);\n}\n",
    );
    let r: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(
        r[0], "changes signature of argB, used by compare below",
        "{r:?}"
    );
}

/// A definition edited in place keeps "edits", whatever else the file holds
/// under its name: overloads, a decorated or exported twin.
#[test]
fn an_edited_definition_is_not_added_because_its_name_repeats() {
    let java = hunks(
        "A.java",
        "class A {\n\tint f(int a) {\n\t\treturn a;\n\t}\n\n\tint f(String s) {\n\t\treturn 1;\n\t}\n}\n",
        "class A {\n\tint f(String s) {\n\t\treturn 2;\n\t}\n}\n",
    );
    let r: Vec<&str> = java.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("adds")), "{r:?}");

    let py = hunks(
        "k.py",
        "def helper():\n    return 0\n\n\nclass K:\n    @cache\n    def helper(self):\n        return 1\n",
        "class K:\n    @cache(1)\n    def helper(self):\n        return 1\n",
    );
    let r: Vec<&str> = py.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("adds")), "{r:?}");

    let ts = hunks(
        "k.ts",
        "export function helper() {\n  return 0;\n}\n\nexport function other() {\n  return 1;\n}\n",
        "export function other() {\n  return 2;\n}\n",
    );
    let r: Vec<&str> = ts.iter().map(|h| h.rationale.as_str()).collect();
    assert!(r.iter().all(|x| !x.starts_with("adds")), "{r:?}");
}

/// tasks-3uv.171: an import hunk that only adds names to an existing statement
/// names the arrivals, not the names the statement already had.
#[test]
fn an_import_that_only_gains_names_names_the_arrivals() {
    let cases = [
        (
            "a.py",
            "from a.b import c, d\n\nx = 1\n",
            "from a.b import c, d, e\n\nx = 1\n",
            "adds import e",
        ),
        (
            "a.py",
            "from a.b import (\n    c,\n    d,\n)\n",
            "from a.b import (\n    c,\n    e,\n    d,\n)\n",
            "adds import e",
        ),
        (
            "a.ts",
            "import { a, b } from \"./x\";\n\nexport const k = 1;\n",
            "import { a, b, c } from \"./x\";\n\nexport const k = 1;\n",
            "adds import c",
        ),
        (
            "a.ts",
            "import { a, b } from \"./x\";\n\nexport const k = 1;\n",
            "import d, { a, b, c } from \"./x\";\n\nexport const k = 1;\n",
            "adds import c, d",
        ),
        (
            "a.go",
            "package p\n\nimport (\n\t\"fmt\"\n)\n",
            "package p\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n",
            "adds import os",
        ),
        (
            "a.rs",
            "use std::{io, fmt};\n\nfn f() {}\n",
            "use std::{io, fmt, mem};\n\nfn f() {}\n",
            "adds import mem",
        ),
        (
            "a.rs",
            "use std::{\n    io,\n    fmt,\n};\n",
            "use std::{\n    io,\n    mem,\n    fmt,\n};\n",
            "adds import mem",
        ),
        (
            "A.java",
            "package p;\n\nimport java.util.List;\n\nclass A {}\n",
            "package p;\n\nimport java.util.List;\nimport java.util.Map;\n\nclass A {}\n",
            "adds import Map",
        ),
    ];
    for (path, old, new, want) in cases {
        let hs = hunks(path, old, new);
        let got: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
        assert_eq!(got, vec![want], "{path}: {old:?} -> {new:?}");
    }
}

/// tasks-3uv.179: a name is the same import only from the same module. A name
/// another statement already imports is still an arrival when this one brings
/// it from elsewhere, one that stays in its module and merely changes
/// statement is still a move, and a name that leaves for another module is
/// removed and added.
#[test]
fn an_imported_name_is_compared_with_its_own_module() {
    let cases = [
        (
            "a.py",
            "from a import d\nfrom b import z\n",
            "from a import d\nfrom b import d, z\n",
            vec!["adds import d"],
        ),
        (
            "a.py",
            "from a import c\nfrom b import (\n    d,\n)\n",
            "from a import c\nfrom b import (\n    c,\n    d,\n)\n",
            vec!["adds import c"],
        ),
        (
            "a.py",
            "from a import c\nfrom b import (\n    d,\n)\n",
            "from a import c\nfrom b import (\n    c,\n    e,\n    d,\n)\n",
            vec!["adds import c, e"],
        ),
        (
            "a.py",
            "from a import c, d\nfrom b import z\n",
            "from a import c\nfrom b import d, z\n",
            vec!["changes import d"],
        ),
        (
            "a.py",
            "from a import c, d\n",
            "from b import c, d\n",
            vec!["changes import c, d"],
        ),
        (
            "a.py",
            "from a import c, d\nimport os\nimport sys\nimport re\nfrom a import e\n",
            "from a import c\nimport os\nimport sys\nimport re\nfrom a import d, e\n",
            vec!["moves import d", "moves import d"],
        ),
        (
            "a.py",
            "from a import d\nimport os\nimport sys\nimport re\nimport json\n",
            "import os\nimport sys\nimport re\nimport json\nfrom a import d\n",
            vec!["moves import d", "moves import d"],
        ),
        (
            "a.py",
            "from a import d\nimport os\n",
            "import os\nfrom b import d\n",
            vec!["moves import d", "moves import d"],
        ),
        (
            "a.py",
            "from a import d\nfrom b import d\n",
            "from b import d\n",
            vec!["removes import d"],
        ),
        (
            "a.ts",
            "import { a } from './x';\nimport os from 'os';\n",
            "import os from 'os';\nimport { a } from './y';\n",
            vec!["moves import a", "moves import a"],
        ),
        (
            "a.py",
            "from a import d\nimport os\nimport sys\nimport re\nfrom b import z\n",
            "from a import e\nimport os\nimport sys\nimport re\nfrom b import d, z\n",
            vec!["adds import e", "moves import d"],
        ),
        (
            "a.py",
            "from a import d, e\n",
            "from a import d\nfrom b import d, e\n",
            vec!["adds import d; changes import e"],
        ),
        (
            "a.rs",
            "use std::{\n    io,\n};\n",
            "use std::{\n    io,\n    fmt::*,\n};\n",
            vec!["adds import fmt::*"],
        ),
        (
            "a.ts",
            "import { a } from \"./x\";\n",
            "import * as ns from \"./y\";\nimport { a } from \"./x\";\n",
            vec!["adds import ns"],
        ),
        (
            "a.ts",
            "import { a } from \"./x\";\n",
            "import \"./y\";\nimport { a } from \"./x\";\n",
            vec!["import"],
        ),
        (
            "a.ts",
            "export { a } from \"./x\";\n",
            "export { a, b } from \"./x\";\n",
            vec!["adds re-export b"],
        ),
    ];
    for (path, old, new, want) in cases {
        let hs = hunks(path, old, new);
        let got: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
        assert_eq!(got, want, "{path}: {old:?} -> {new:?}");
    }
}

/// An import that gains one name and loses another says both; one that only
/// reorders its names is formatting; one that changes an alias still names it.
#[test]
fn a_mixed_import_edit_says_each_part() {
    let cases = [
        (
            "a.py",
            "from a.b import c, d\n",
            "from a.b import c, e\n",
            "adds import e; removes import d",
        ),
        (
            "a.ts",
            "import { a, b } from \"./x\";\n",
            "import { a, c } from \"./x\";\n",
            "adds import c; removes import b",
        ),
        (
            "a.rs",
            "use std::{io, fmt};\n",
            "use std::{fmt, io};\n",
            "formatting only",
        ),
        (
            "a.py",
            "from a.b import c, d\n",
            "from a.b import d, c\n",
            "formatting only",
        ),
        (
            "a.py",
            "from a.b import c as x\n",
            "from a.b import c as y\n",
            "replaces import x with y",
        ),
    ];
    for (path, old, new, want) in cases {
        let hs = hunks(path, old, new);
        let got: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
        assert_eq!(got, vec![want], "{path}: {old:?} -> {new:?}");
    }
}
