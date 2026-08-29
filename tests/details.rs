//! P15 detail-layer tests: what a hunk did to the members of its container.
use ordo::model::Input;

fn details(v: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks.into_iter().flat_map(|h| h.details))
        .collect()
}

fn one(path: &str, old: &str, new: &str) -> Vec<String> {
    details(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
}

#[test]
fn p15_add_remove_and_change_within_a_container() {
    let d = one(
        "a.rs",
        "pub enum Cli {\n    Build,\n    Test,\n    Legacy,\n}\n",
        "pub enum Cli {\n    Build,\n    Test,\n    Serve,\n    Watch,\n}\n",
    );
    assert!(d.iter().any(|x| x == "adds Serve, Watch to Cli"), "{d:?}");
    assert!(d.iter().any(|x| x == "removes Legacy from Cli"), "{d:?}");

    let d = one(
        "b.rs",
        "pub struct C {\n    pub name: String,\n    pub retries: usize,\n}\n",
        "pub struct C {\n    pub name: String,\n    pub retries: u32,\n}\n",
    );
    assert!(d.iter().any(|x| x == "changes retries in C"), "{d:?}");
}

#[test]
fn p15_generalizes_across_languages() {
    // go struct field, java enum constant, ts enum member, js object property,
    // python dict key, cpp struct field — one members table entry each
    for (path, old, new, want) in [
        (
            "a.go",
            "package m\ntype C struct {\n\tA int\n\tLegacy bool\n}\n",
            "package m\ntype C struct {\n\tA int\n\tServe bool\n}\n",
            "adds Serve",
        ),
        (
            "b.java",
            "class M {\n  enum Cli { BUILD, LEGACY }\n}\n",
            "class M {\n  enum Cli { BUILD, SERVE }\n}\n",
            "adds SERVE to M.Cli",
        ),
        (
            "c.ts",
            "export enum Cli { Build = 1, Legacy = 2 }\n",
            "export enum Cli { Build = 1, Serve = 2 }\n",
            "adds Serve to Cli",
        ),
        (
            "d.js",
            "const o = { name: \"x\", legacy: true };\n",
            "const o = { name: \"x\", serve: true };\n",
            "adds serve",
        ),
        (
            "e.py",
            "OPTS = {\n    \"name\": \"x\",\n    \"legacy\": True,\n}\n",
            "OPTS = {\n    \"name\": \"x\",\n    \"serve\": True,\n}\n",
            "adds serve",
        ),
        (
            "f.cpp",
            "struct C {\n  int a;\n  bool legacy;\n};\n",
            "struct C {\n  int a;\n  bool serve;\n};\n",
            "adds serve",
        ),
    ] {
        let d = one(path, old, new);
        assert!(
            d.iter().any(|x| x.starts_with(want)),
            "{path}: want {want:?}, got {d:?}"
        );
    }
}

#[test]
fn p15_member_sharing_a_line_with_a_change_is_not_named() {
    // the whole enum is one line, so every member falls in the hunk — only the
    // one whose own text moved counts as changed
    let d = one(
        "a.ts",
        "export enum Cli { Build = 1, Test = 2 }\n",
        "export enum Cli { Build = 1, Test = 9 }\n",
    );
    assert!(d.iter().any(|x| x == "changes Test in Cli"), "{d:?}");
    assert!(!d.iter().any(|x| x.contains("Build")), "{d:?}");
}

#[test]
fn p15_wholly_new_container_stays_silent() {
    // the rationale already says "adds type Fresh" — listing the fields it was
    // born with adds nothing
    let d = one(
        "a.rs",
        "pub fn x() {}\n",
        "pub fn x() {}\n\npub struct Fresh {\n    pub a: u8,\n    pub b: u8,\n}\n",
    );
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn p15_long_member_lists_are_capped() {
    let d = one(
        "a.rs",
        "pub enum E {\n    A,\n}\n",
        "pub enum E {\n    A,\n    B,\n    C,\n    D,\n    F,\n    G,\n}\n",
    );
    assert!(
        d.iter().any(|x| x == "adds B, C, D, and 2 more to E"),
        "{d:?}"
    );
}

#[test]
fn a_member_that_is_its_own_container_is_not_relisted() {
    // `{ run: () => {} }` makes `run` both the object member and — once an arrow
    // function counts as a definition — its own enclosing def, which read
    // "adds run to run". The rationale already says "adds run".
    let d = one(
        "o.js",
        "const opts = { name: \"x\" };\n",
        "const opts = { name: \"x\", run: () => 1 };\n",
    );
    assert!(
        !d.iter().any(|x| x.contains("run to run")),
        "a member must not be named as its own container: {d:?}"
    );
    // an ordinary member in the same shape still reports
    let d = one(
        "p.js",
        "const opts = { name: \"x\" };\n",
        "const opts = { name: \"x\", retries: 5 };\n",
    );
    assert!(
        d.iter().any(|x| x.contains("retries")),
        "an ordinary member still reports: {d:?}"
    );
}

#[test]
fn p15_keyword_arguments_attribute_to_their_own_call_not_the_enclosing_def() {
    // regression for the container-attribution bug: two distinct
    // `add_argument(...)` calls inside one `main` must not have their
    // keyword arguments reported against `main`.
    let d = one(
        "cli.py",
        "import argparse\n\ndef main():\n    ap = argparse.ArgumentParser()\n    ap.add_argument(\"--sample\", type=int, required=True)\n    return ap.parse_args()\n",
        "import argparse\n\ndef main():\n    ap = argparse.ArgumentParser()\n    ap.add_argument(\"--sample\", type=int, help=\"one sample number\")\n    ap.add_argument(\"--samples\", type=int, nargs=\"*\", default=[], help=\"many\")\n    return ap.parse_args()\n",
    );
    assert!(!d.iter().any(|x| x.contains(" to main")), "{d:?}");
    assert!(
        d.iter()
            .any(|x| x == "adds help to ap.add_argument(\"--sample\")"),
        "{d:?}"
    );
    assert!(
        d.iter()
            .any(|x| x == "removes required from ap.add_argument(\"--sample\")"),
        "{d:?}"
    );
    assert!(
        d.iter()
            .any(|x| x.starts_with("adds") && x.contains("ap.add_argument(\"--samples\")")),
        "{d:?}"
    );
}

#[test]
fn p15_call_with_no_literal_first_argument_falls_back_to_callee_alone() {
    let d = one(
        "cli.py",
        "def main():\n    foo(x, y=1)\n",
        "def main():\n    foo(x, y=1, z=2)\n",
    );
    assert!(
        d.iter().any(|x| x == "adds z to foo"),
        "container should be the bare callee name: {d:?}"
    );
}

#[test]
fn p15_struct_member_still_attributes_to_its_type() {
    // unchanged behavior: a member of a type body (not a call) keeps naming
    // the enclosing definition.
    let d = one(
        "a.rs",
        "pub struct C {\n    pub name: String,\n}\n",
        "pub struct C {\n    pub name: String,\n    pub retries: u32,\n}\n",
    );
    assert!(d.iter().any(|x| x == "adds retries to C"), "{d:?}");
}
