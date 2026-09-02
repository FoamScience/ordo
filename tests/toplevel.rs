//! Hunks that sit outside any definition — the single structural cause of a
//! bare "change" rationale. Each test here pins one container the engine now
//! recognises.
use ordo::model::{Category, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Output {
    run(serde_json::json!({ "changes": [{ "path": path, "old": old, "new": new }] }))
}

fn rationales(out: &Output) -> Vec<String> {
    out.files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.rationale.clone()))
        .collect()
}

#[test]
fn a_c_macro_is_a_definition_its_body_belongs_to() {
    let out = one(
        "m.c",
        "#define LOCK(d) \\\n  do_lock((d), 1)\n\nint f(void) { return 1; }\n",
        "#define LOCK(d) \\\n  do_lock_share((d)->s, 1)\n\nint f(void) { return 1; }\n",
    );
    // a body-only hunk is `other` (the def's own row is unchanged), the same
    // as an edit inside a function body — what matters is that it now has a
    // container to be attributed to
    let h = &out.files[0].hunks[0];
    assert_eq!(h.category, Category::Other);
    assert_eq!(h.enclosing.as_deref(), Some("LOCK"));
    // the macro's value is its body, so a body change is not a signature change
    assert_eq!(h.rationale, "edits LOCK");
}

#[test]
fn an_object_macro_is_named_too() {
    let out = one("m.h", "#define MAX 10\n", "#define MAX 20\n");
    assert_eq!(out.files[0].hunks[0].enclosing.as_deref(), Some("MAX"));
}

#[test]
fn a_cpp_macro_body_is_attributed_to_the_macro() {
    let out = one(
        "m.C",
        "#define OP(op) \\\n  boundaryFieldRef() op gf.boundaryField();\n",
        "#define OP(op) \\\n  boundaryRef() op gf.boundary();\n",
    );
    assert_eq!(out.files[0].hunks[0].enclosing.as_deref(), Some("OP"));
}

#[test]
fn a_re_export_is_module_bookkeeping_like_an_import() {
    // `export * from "./b"` forwards names defined elsewhere: it follows from
    // the real change rather than being it, exactly as an import does — so it
    // is classified as one, and carries an import's noise flag
    let out = one(
        "i.ts",
        "export * from \"./a\";\n\nexport const x = 1;\n",
        "export * from \"./a\";\nexport * from \"./b\";\n\nexport const x = 1;\n",
    );
    let h = out.files[0]
        .hunks
        .iter()
        .find(|h| h.new_range[0] == 2)
        .expect("the export hunk");
    assert_eq!(h.category, Category::Import);
    assert!(h.noise, "bookkeeping is skippable");
}

#[test]
fn the_bare_export_module_marker_is_bookkeeping_too() {
    let out = one(
        "t.d.ts",
        "type A = string;\n",
        "type A = string;\n\nexport {};\n",
    );
    let h = out.files[0].hunks.last().expect("the marker hunk");
    assert_eq!(h.category, Category::Import);
    assert!(h.noise);
}

#[test]
fn an_export_that_declares_something_is_that_declaration_not_bookkeeping() {
    let out = one(
        "d.ts",
        "export function run() {\n  return 1;\n}\n",
        "export function run() {\n  return 2;\n}\n",
    );
    assert_eq!(rationales(&out), vec!["edits run"]);
    assert!(out.files[0].dropped.is_empty());
}

// ---- test blocks: `describe`/`it`/`test` name a block the way a def does ----

#[test]
fn a_hunk_inside_a_test_is_attributed_to_that_test() {
    let out = one(
        "a.test.ts",
        "describe(\"cli\", () => {\n  it(\"parses flags\", () => {\n    expect(run([\"-v\"])).toBe(1);\n  });\n});\n",
        "describe(\"cli\", () => {\n  it(\"parses flags\", () => {\n    expect(run([\"-v\", \"-q\"])).toBe(2);\n  });\n});\n",
    );
    let h = &out.files[0].hunks[0];
    // `enclosing` carries the full path, joined with ` > ` rather than the
    // language's scope separator...
    assert_eq!(
        h.enclosing.as_deref(),
        Some("describe \"cli\" > it \"parses flags\"")
    );
    // ...while the rationale names the innermost block, which is what
    // identifies the hunk without spending the whole line on its container
    assert_eq!(h.rationale, "edits it \"parses flags\"");
}

#[test]
fn adding_a_test_reads_as_adding_it() {
    let out = one(
        "a.test.js",
        "describe(\"cli\", () => {\n  it(\"a\", () => {\n    ok();\n  });\n});\n",
        "describe(\"cli\", () => {\n  it(\"a\", () => {\n    ok();\n  });\n\n  it(\"handles empty input\", () => {\n    ok();\n  });\n});\n",
    );
    // the rationale names the innermost block: the outer `describe` is a
    // sentence the reviewer already has on screen, and spending the line on it
    // leaves no room for what changed (`enclosing` still carries the full path)
    assert_eq!(rationales(&out), vec!["adds it \"handles empty input\""]);
}

#[test]
fn a_test_name_is_never_a_symbol_and_never_seeds_an_edge() {
    // a test label is a navigation aid, not a declaration another file can
    // reference: it must stay out of `symbols` (which keys persisted review
    // marks) and out of the def→use graph
    let out = one(
        "a.test.tsx",
        "test(\"renders\", () => {\n  render(<App />);\n});\n",
        "test(\"renders\", () => {\n  render(<App name=\"x\" />);\n});\n",
    );
    let h = &out.files[0].hunks[0];
    assert!(h.symbols.is_empty(), "{:?}", h.symbols);
    assert!(out.edges.is_empty(), "{:?}", out.edges);
}

#[test]
fn runner_prefixes_and_template_names_are_recognised() {
    let out = one(
        "a.test.js",
        "test.serial(`streams ${x} bytes`, async () => {\n  await go(1);\n});\n",
        "test.serial(`streams ${x} bytes`, async () => {\n  await go(2);\n});\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("test `streams ${x} bytes`")
    );
}

#[test]
fn a_call_without_a_body_is_not_a_block() {
    // `test("name")` declares nothing to hold hunks; only a call with a
    // function argument is a block
    let out = one(
        "a.js",
        "test(\"x\");\nconst a = 1;\n",
        "test(\"x\");\nconst a = 2;\n",
    );
    assert_eq!(out.files[0].hunks[0].enclosing, None);
}

// ---- regions: a container that holds code without declaring anything ----

#[test]
fn a_conditional_compilation_block_names_the_region_it_guards() {
    let out = one(
        "e.c",
        "#ifdef CURL_DISABLE_HTTP\nint z = 1;\n#endif\n",
        "#ifdef CURL_DISABLE_HTTP\nint z = 2;\n#endif\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("#ifdef CURL_DISABLE_HTTP"));
    assert_eq!(h.enclosing_kind, Some(ordo::model::ContainerKind::Region));
    assert_eq!(h.rationale, "edits #ifdef CURL_DISABLE_HTTP");
}

#[test]
fn a_guarded_macro_does_not_swallow_the_line_after_it() {
    // `#define GUARD_H` ends at column 0 of the NEXT row, so without the
    // end-row correction the line after an include guard reads as part of the
    // macro
    let out = one(
        "h.h",
        "#ifndef GUARD_H\n#define GUARD_H\nint a = 1;\n#endif\n",
        "#ifndef GUARD_H\n#define GUARD_H\nint a = 2;\n#endif\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("#ifndef GUARD_H")
    );
}

#[test]
fn an_ifndef_reads_differently_from_an_ifdef() {
    let out = one(
        "k.c",
        "#ifndef W\nint w = 1;\n#endif\n",
        "#ifndef W\nint w = 2;\n#endif\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("#ifndef W")
    );
}

#[test]
fn an_if_expression_keeps_its_spacing() {
    let out = one(
        "g.c",
        "#if defined(A) && !defined(B)\nint z = 1;\n#endif\n",
        "#if defined(A) && !defined(B)\nint z = 2;\n#endif\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("#if defined(A) && !defined(B)")
    );
}

#[test]
fn a_region_declares_nothing_and_seeds_no_edge() {
    // CURL_DISABLE_HTTP is *tested* by the guard, never defined by it
    let out = run(serde_json::json!({ "changes": [
        { "path": "a.c", "old": "#define FLAG 1\n", "new": "#define FLAG 2\n" },
        { "path": "b.c", "old": "#ifdef FLAG\nint z = 1;\n#endif\n",
          "new": "#ifdef FLAG\nint z = 2;\n#endif\n" },
    ]}));
    let b = out.files.iter().find(|f| f.path == "b.c").unwrap();
    assert!(b.hunks[0].defines.is_empty(), "{:?}", b.hunks[0].defines);
    assert!(b.hunks[0].symbols.is_empty());
}

#[test]
fn a_documents_preamble_is_a_region_of_its_own() {
    let out = one(
        "R.md",
        "![badge](a.svg)\n\nA tool.\n\n# Install\n\nrun it\n",
        "![badge](a.svg)\n\nA better tool.\n\n# Install\n\nrun it\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("preamble"));
    // "section" is the word for a prose definition; a region names itself
    assert_eq!(h.rationale, "edits preamble");
}

#[test]
fn front_matter_is_named_as_such() {
    let out = one(
        "F.md",
        "---\nTitle: curl\nSee-also:\n  - a\n---\n\n# Name\n\ntext\n",
        "---\nTitle: curl\nSee-also:\n  - a\n  - b\n---\n\n# Name\n\ntext\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("front matter"));
    assert_eq!(
        h.enclosing_kind,
        Some(ordo::model::ContainerKind::FrontMatter)
    );
    assert_eq!(h.rationale, "edits front matter");
}

// ---- binding interiors: the lines inside a top-level literal ----

#[test]
fn a_line_inside_a_top_level_table_belongs_to_the_binding() {
    let out = one(
        "t.py",
        "ALLOWED = {\n    \"builtins\",\n    \"os\",\n}\n",
        "ALLOWED = {\n    \"builtins\",\n    \"os\",\n    \"typing_extensions\",\n}\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("ALLOWED"));
    assert_eq!(h.enclosing_kind, Some(ordo::model::ContainerKind::Binding));
    assert_eq!(h.rationale, "edits ALLOWED");
    // and the detail layer names *which* entry arrived
    assert_eq!(h.details, vec!["adds \"typing_extensions\" to ALLOWED"]);
}

#[test]
fn a_replaced_entry_reads_as_both_sides() {
    let out = one(
        "a.ts",
        "const ROUTES = [\n  \"/a\",\n  \"/b\",\n];\n",
        "const ROUTES = [\n  \"/a\",\n  \"/c\",\n];\n",
    );
    let d = &out.files[0].hunks[0].details;
    assert!(d.contains(&"adds \"/c\" to ROUTES".to_string()), "{d:?}");
    assert!(
        d.contains(&"removes \"/b\" from ROUTES".to_string()),
        "{d:?}"
    );
}

#[test]
fn a_keyed_entry_is_reported_once_by_its_key() {
    // an object property is already a member kind: it must not also be
    // reported as raw text, which would say the same thing twice
    let out = one(
        "b.js",
        "const CFG = {\n  retries: 3,\n  timeout: 10,\n};\n",
        "const CFG = {\n  retries: 5,\n  timeout: 10,\n};\n",
    );
    assert_eq!(
        out.files[0].hunks[0].details,
        vec!["changes retries in CFG"]
    );
}

#[test]
fn a_local_binding_keeps_its_function_as_container() {
    // inside a function the function is the container: a local is not a
    // top-level binding and must not shadow it
    let out = one(
        "d.py",
        "def run():\n    local = {\n        \"a\": 1,\n    }\n    return local\n",
        "def run():\n    local = {\n        \"a\": 2,\n    }\n    return local\n",
    );
    assert_eq!(out.files[0].hunks[0].enclosing.as_deref(), Some("run"));
}

#[test]
fn a_binding_container_declares_no_symbol() {
    let out = one(
        "t.py",
        "TABLE = {\n    \"a\": 1,\n}\n",
        "TABLE = {\n    \"a\": 1,\n    \"b\": 2,\n}\n",
    );
    let h = &out.files[0].hunks[0];
    assert!(h.symbols.is_empty(), "{:?}", h.symbols);
    assert!(h.defines.is_empty(), "{:?}", h.defines);
}

#[test]
fn a_one_line_binding_is_not_a_container() {
    let out = one("t.py", "X = [1, 2]\n", "X = [1, 3]\n");
    assert_eq!(out.files[0].hunks[0].enclosing, None);
}

// ---- whitespace-only hunks are formatting, not an unexplained change ----

#[test]
fn added_blank_lines_are_formatting_noise() {
    let out = one(
        "a.c",
        "int a(void) { return 1; }\nint b(void) { return 2; }\n",
        "int a(void) { return 1; }\n\n\nint b(void) { return 2; }\n",
    );
    let h = &out.files[0].hunks[0];
    assert!(h.noise, "a blank-line-only hunk is formatting");
    assert_eq!(h.rationale, "formatting only");
}

#[test]
fn removed_blank_lines_are_formatting_noise_too() {
    let out = one(
        "b.py",
        "def a():\n    return 1\n\n\n\ndef b():\n    return 2\n",
        "def a():\n    return 1\n\n\ndef b():\n    return 2\n",
    );
    assert!(out.files[0].hunks[0].noise);
}

#[test]
fn a_real_edit_is_never_called_formatting() {
    let out = one(
        "c.py",
        "def a():\n    return 1\n",
        "def a():\n    return 2\n",
    );
    assert!(!out.files[0].hunks[0].noise);
}

#[test]
fn a_rust_test_macro_names_its_test() {
    let out = one(
        "r.rs",
        "rgtest!(r428_style, |dir: Dir| {\n    dir.create(\"foo\", \"bar\");\n});\n",
        "rgtest!(r428_style, |dir: Dir| {\n    dir.create(\"foo\", \"baz\");\n});\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("rgtest! r428_style"));
    assert_eq!(h.enclosing_kind, Some(ordo::model::ContainerKind::Test));
}

#[test]
fn an_ordinary_macro_is_not_a_test_block() {
    let out = one(
        "s.rs",
        "fn main() {\n    println!(\"a\");\n}\n",
        "fn main() {\n    println!(\"b\");\n}\n",
    );
    assert_eq!(out.files[0].hunks[0].enclosing.as_deref(), Some("main"));
}

#[test]
fn a_generated_file_is_recognised_by_its_name() {
    // lazy.nvim writes `_generated.lua`; the `.generated.` infix and go's
    // `_generated.go` are the same convention
    assert!(ordo::is_generated_path("lua/lazy/community/_generated.lua"));
    assert!(ordo::is_generated_path("src/api.generated.ts"));
    assert!(ordo::is_generated_path("pkg/mock_generated.go"));
    assert!(!ordo::is_generated_path("src/generator.lua"));
    assert!(!ordo::is_generated_path("src/generated_test_helper.rs"));
}

#[test]
fn a_lua_field_assignment_is_a_binding_container() {
    let out = one(
        "c.lua",
        "M.defaults = {\n  \"rockspec\",\n  \"luarocks\",\n}\n",
        "M.defaults = {\n  \"rockspec\", -- only with rocks.enabled\n  \"luarocks\",\n}\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("M.defaults")
    );
}

#[test]
fn busted_style_lua_tests_are_blocks_too() {
    let out = one(
        "s.lua",
        "describe(\"spec\", function()\n  it(\"works\", function()\n    assert(one())\n  end)\nend)\n",
        "describe(\"spec\", function()\n  it(\"works\", function()\n    assert(two())\n  end)\nend)\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("describe \"spec\" > it \"works\"")
    );
}

#[test]
fn a_prose_section_path_is_never_shortened() {
    // a markdown path is the navigation itself: `Project > Install` must not
    // collapse to `Install`, which points at a different heading
    let out = one(
        "d.md",
        "# Project\n\n## Install\n\nold text\n",
        "# Project\n\n## Install\n\nnew text\n",
    );
    assert_eq!(
        out.files[0].hunks[0].rationale,
        "edits section Project > Install"
    );
}

#[test]
fn a_very_long_container_name_is_elided_not_left_to_crowd_the_line() {
    let name = "x".repeat(120);
    let out = one(
        "t.js",
        &format!("it(\"{name}\", () => {{\n  ok(1);\n}});\n"),
        &format!("it(\"{name}\", () => {{\n  ok(2);\n}});\n"),
    );
    let r = &out.files[0].hunks[0].rationale;
    assert!(r.ends_with('…'), "{r}");
    assert!(r.chars().count() < 80, "{r}");
}

#[test]
fn a_top_level_call_with_multi_line_arguments_is_a_container() {
    let out = one(
        "t.ts",
        "execa('unicorns', {\n\tstdio: [\n\t\t'pipe',\n\t\t'overlapped',\n\t],\n});\n",
        "execa('unicorns', {\n\tstdio: [\n\t\t'pipe',\n\t\tundefined,\n\t],\n});\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("execa('unicorns')"));
    assert_eq!(h.enclosing_kind, Some(ordo::model::ContainerKind::Call));
}

#[test]
fn a_test_block_is_named_as_a_test_not_as_a_call() {
    // both rules match the same node shape; the test-block reading is the
    // useful one and must win
    let out = one(
        "t.test.ts",
        "it(\"works\", () => {\n  expect(one()).toBe(1);\n});\n",
        "it(\"works\", () => {\n  expect(two()).toBe(2);\n});\n",
    );
    assert_eq!(
        out.files[0].hunks[0].enclosing_kind,
        Some(ordo::model::ContainerKind::Test)
    );
}

#[test]
fn a_one_line_call_is_not_a_container() {
    let out = one("t.js", "run(1);\nconst a = 1;\n", "run(2);\nconst a = 1;\n");
    assert_eq!(out.files[0].hunks[0].enclosing, None);
}

// ---- code switched off (or back on) ----

#[test]
fn commenting_code_out_says_so() {
    let out = one(
        "a.lua",
        "local function f()\n  vim.print(spec.plugins.bar)\n  return 1\nend\n",
        "local function f()\n  -- vim.print(spec.plugins.bar)\n  return 1\nend\n",
    );
    assert_eq!(out.files[0].hunks[0].rationale, "comments out code in f");
}

#[test]
fn uncommenting_code_says_so_too() {
    let out = one(
        "b.py",
        "def g():\n    # legacy = compute(1)\n    return 2\n",
        "def g():\n    legacy = compute(1)\n    return 2\n",
    );
    assert_eq!(out.files[0].hunks[0].rationale, "uncomments code in g");
}

#[test]
fn replacing_code_with_unrelated_prose_is_not_commenting_out() {
    // the commented-out test is exact: strip the markers and the two sides must
    // match. A hunk that swaps code for a *different* comment is reported as
    // what it is — documentation arriving where code left — rather than as
    // code being switched off
    let out = one(
        "c.js",
        "function h() {\n  const a = 1;\n  return a;\n}\n",
        "function h() {\n  // a totally different comment\n  return a;\n}\n",
    );
    assert_eq!(
        out.files[0].hunks[0].rationale,
        "replaces 1 line with a comment in h"
    );
}

#[test]
fn a_doc_comment_arriving_where_code_left_says_both_halves() {
    let out = one(
        "p.rs",
        "/// On Unix, an optimized check.\n#[cfg(unix)]\npub fn is_hidden(d: &D) -> bool { true }\n",
        "/// ## Windows\n",
    );
    assert_eq!(
        out.files[0].hunks[0].rationale,
        "replaces 3 lines with comments"
    );
}

#[test]
fn commenting_out_at_file_scope_needs_no_container() {
    let out = one("d.js", "run(1);\n", "// run(1);\n");
    assert_eq!(out.files[0].hunks[0].rationale, "comments out code");
}

#[test]
fn a_removed_module_constant_is_named_not_counted() {
    let out = one(
        "b.ts",
        "export const OLD = 24;\nexport function run() { return 1; }\n",
        "export function run() { return 1; }\n",
    );
    assert_eq!(out.files[0].hunks[0].rationale, "removes OLD");
}

#[test]
fn a_removed_local_is_still_part_of_editing_its_function() {
    // only file-scope bindings are named: a local's removal belongs to the
    // function it lived in, which the rationale already names
    let out = one(
        "c.py",
        "def run():\n    tmp = 1\n    return 2\n",
        "def run():\n    return 2\n",
    );
    assert!(
        !out.files[0].hunks[0].rationale.contains("removes tmp"),
        "{}",
        out.files[0].hunks[0].rationale
    );
}

#[test]
fn a_default_export_of_a_value_is_not_bookkeeping() {
    // `export default {…}` is the whole body of a config file or a component.
    // It fills the `value` field rather than `declaration`, and reading "no
    // declaration" as "bookkeeping" swept every hunk inside it out of review.
    let out = one(
        "config.ts",
        "export default {\n  a: 1,\n}\n",
        "export default {\n  a: 1,\n  b: 2,\n}\n",
    );
    assert_eq!(out.files[0].hunks.len(), 1, "{:?}", out.files[0].dropped);
    assert!(out.files[0].dropped.is_empty());
    assert_eq!(out.files[0].hunks[0].details, vec!["adds b"]);
}

#[test]
fn a_default_exported_call_keeps_its_contents_too() {
    let out = one(
        "vitest.config.ts",
        "export default defineConfig({\n  test: {\n    timeout: 10,\n  },\n})\n",
        "export default defineConfig({\n  test: {\n    timeout: 20,\n  },\n})\n",
    );
    assert_eq!(out.files[0].hunks.len(), 1, "{:?}", out.files[0].dropped);
    assert_eq!(
        out.files[0].hunks[0].enclosing.as_deref(),
        Some("defineConfig")
    );
}

#[test]
fn the_bookkeeping_forms_are_still_bookkeeping() {
    for (old, new) in [
        (
            "export * from \"./a\";\n",
            "export * from \"./a\";\nexport * from \"./b\";\n",
        ),
        ("type A = 1;\n", "type A = 1;\n\nexport {};\n"),
    ] {
        let out = one("b.ts", old, new);
        assert!(
            out.files[0]
                .hunks
                .iter()
                .any(|h| h.category == Category::Import && h.noise),
            "{old:?} -> {new:?}: {:?}",
            rationales(&out)
        );
    }
}

#[test]
fn a_file_scope_with_block_is_a_region() {
    // python and xonsh scripts put real work in a `with` at file scope; a hunk
    // inside one otherwise has no container at all
    let out = one(
        "w.py",
        "with open(p) as fh:\n    data = fh.read()\n    total = 1\n",
        "with open(p) as fh:\n    data = fh.read(4096)\n    total = 1\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.enclosing.as_deref(), Some("with open(p) as fh"));
    assert_eq!(h.rationale, "edits with open(p) as fh");
}

#[test]
fn a_with_block_inside_a_definition_leaves_the_definition_the_container() {
    let out = one(
        "w.py",
        "def f(p):\n    with open(p) as fh:\n        return fh.read()\n",
        "def f(p):\n    with open(p) as fh:\n        return fh.read(4096)\n",
    );
    assert_eq!(out.files[0].hunks[0].enclosing.as_deref(), Some("f"));
}

#[test]
fn a_new_side_of_nothing_but_blank_lines_reads_as_a_removal() {
    // a line replaced by an empty one has as little to say as a pure deletion
    let out = one(
        "w.py",
        "def f():\n    pass\n\nx = 1\ny = 2\n",
        "def f():\n    pass\n\n\ny = 2\n",
    );
    assert!(
        rationales(&out).iter().any(|r| r.contains("removes")),
        "{:?}",
        rationales(&out)
    );
}
