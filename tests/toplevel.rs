//! Hunks that sit outside any definition — the single structural cause of a
//! bare "change" rationale. Each test here pins one container the engine now
//! recognises.
use ordo::model::{Category, DropReason, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Output {
    run(serde_json::json!({ "changes": [{ "path": path, "old": old, "new": new }] }))
}

fn rationales(out: &Output) -> Vec<String> {
    out.files.iter().flat_map(|f| f.hunks.iter().map(|h| h.rationale.clone())).collect()
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
    // the real change rather than being it, exactly as an import does
    let out = one(
        "i.ts",
        "export * from \"./a\";\n\nexport const x = 1;\n",
        "export * from \"./a\";\nexport * from \"./b\";\n\nexport const x = 1;\n",
    );
    assert!(out.files[0].hunks.is_empty(), "{:?}", rationales(&out));
    assert_eq!(out.files[0].dropped[0].reason, DropReason::Import);
}

#[test]
fn the_bare_export_module_marker_is_bookkeeping_too() {
    let out = one("t.d.ts", "type A = string;\n", "type A = string;\n\nexport {};\n");
    assert!(out.files[0].hunks.is_empty(), "{:?}", rationales(&out));
    assert_eq!(out.files[0].dropped[0].reason, DropReason::Import);
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
    // nested blocks read with ` > `, not the language's scope separator
    assert_eq!(h.enclosing.as_deref(), Some("describe \"cli\" > it \"parses flags\""));
    assert_eq!(h.rationale, "edits describe \"cli\" > it \"parses flags\"");
}

#[test]
fn adding_a_test_reads_as_adding_it() {
    let out = one(
        "a.test.js",
        "describe(\"cli\", () => {\n  it(\"a\", () => {\n    ok();\n  });\n});\n",
        "describe(\"cli\", () => {\n  it(\"a\", () => {\n    ok();\n  });\n\n  it(\"handles empty input\", () => {\n    ok();\n  });\n});\n",
    );
    assert_eq!(rationales(&out), vec!["adds describe \"cli\" > it \"handles empty input\""]);
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
