//! The change-level summary: one label, a behaviour verdict and two sentences,
//! by fixed rules over the hunks' facts. A wrong "preserving" is the costly
//! error, so most of the cases below are ones that must not get it.
mod fixture;
use fixture::run_json;
use ordo_diff::model::{Label, Summary, Verdict};
use serde_json::{json, Value};

fn summary(changes: Value) -> Summary {
    run_json(json!({ "changes": changes }))
        .summary
        .expect("a change with hunks has a summary")
}

fn is(changes: Value, label: Label, verdict: Verdict) -> Summary {
    let s = summary(changes);
    assert_eq!((s.label, s.verdict), (label, verdict), "{:?}", s.sentences);
    assert_eq!(s.sentences.len(), 2, "{:?}", s.sentences);
    s
}

#[test]
fn two_files_renaming_one_name_differently_are_both_renames() {
    // each file follows its own rename, whatever the other one did
    let s = is(
        json!([
            {"path": "a.py",
             "old": "def helper(p):\n    return p.strip()\n",
             "new": "def load(p):\n    return p.strip()\n"},
            {"path": "b.py",
             "old": "def helper(p):\n    return p.lower()\n",
             "new": "def read(p):\n    return p.lower()\n"}
        ]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(
        s.sentences[0],
        "Refactor: renames helper → load, helper → read."
    );
}

#[test]
fn a_rename_followed_at_its_call_site_preserves_behaviour() {
    let s = is(
        json!([
            {"path": "a.py",
             "old": "def parse_cfg(p):\n    return p.strip()\n",
             "new": "def load_cfg(p):\n    return p.strip()\n"},
            {"path": "b.py",
             "old": "from a import parse_cfg\n\ndef main():\n    return parse_cfg('x')\n",
             "new": "from a import load_cfg\n\ndef main():\n    return load_cfg('x')\n"}
        ]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(s.sentences[0], "Refactor: renames parse_cfg → load_cfg.");
    assert_eq!(
        s.sentences[1],
        "Behaviour-preserving within this change: every hunk is a rename, but the API changes for callers outside it: parse_cfg → load_cfg."
    );
}

#[test]
fn a_rename_no_outside_caller_can_reach_keeps_the_plain_wording() {
    for (path, old, new) in [
        (
            "a.rs",
            "fn parse(p: u8) -> u8 {\n    p + 1\n}\n\nfn main() {\n    parse(1);\n}\n",
            "fn load(p: u8) -> u8 {\n    p + 1\n}\n\nfn main() {\n    load(1);\n}\n",
        ),
        (
            "a.py",
            "def _parse(p):\n    return p.strip()\n\ndef main():\n    return _parse('x')\n",
            "def _load(p):\n    return p.strip()\n\ndef main():\n    return _load('x')\n",
        ),
    ] {
        let s = is(
            json!([{"path": path, "old": old, "new": new}]),
            Label::Refactor,
            Verdict::Preserving,
        );
        assert_eq!(
            s.sentences[1],
            "Behaviour-preserving: every hunk is a rename."
        );
    }
    let s = summary(json!([{"path": "a.rs",
        "old": "pub fn parse(p: u8) -> u8 {\n    p + 1\n}\n\nfn main() {\n    parse(1);\n}\n",
        "new": "pub fn load(p: u8) -> u8 {\n    p + 1\n}\n\nfn main() {\n    load(1);\n}\n"}]));
    assert!(
        s.sentences[1].ends_with("callers outside it: parse → load."),
        "{:?}",
        s.sentences
    );
    // a pub trait's methods carry no `pub` of their own
    for (vis, exposed) in [("pub ", true), ("", false)] {
        let s = summary(json!([{"path": "a.rs",
            "old": format!("{vis}trait Cfg {{\n    fn parse() -> u8 {{\n        let total = 40 + 2;\n        total\n    }}\n}}\n\nfn main<T: Cfg>() {{\n    T::parse();\n}}\n"),
            "new": format!("{vis}trait Cfg {{\n    fn load() -> u8 {{\n        let total = 40 + 2;\n        total\n    }}\n}}\n\nfn main<T: Cfg>() {{\n    T::load();\n}}\n")}]));
        assert_eq!(
            s.sentences[1].ends_with("callers outside it: parse → load."),
            exposed,
            "{:?}",
            s.sentences
        );
    }
}

#[test]
fn a_rename_that_also_rewrites_a_string_is_not_proven() {
    // the message text changed too: a string spelling the name is not a reference
    is(
        json!([{"path": "a.py",
            "old": "def parse_cfg(p):\n    return p.strip()\n\ndef main():\n    log('parse_cfg')\n    return parse_cfg('x')\n",
            "new": "def load_cfg(p):\n    return p.strip()\n\ndef main():\n    log('load_cfg')\n    return load_cfg('x')\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
}

#[test]
fn a_guard_with_a_test_is_a_fix_candidate_that_changes_behaviour() {
    let s = is(
        json!([
            {"path": "src/a.py",
             "old": "def f(x):\n    y = x + 1\n    return y\n",
             "new": "def f(x):\n    if x is None:\n        return None\n    y = x + 1\n    return y\n"},
            {"path": "tests/test_a.py",
             "old": "def test_f():\n    assert f(1) == 2\n",
             "new": "def test_f():\n    assert f(1) == 2\n\ndef test_none():\n    assert f(None) is None\n"}
        ]),
        Label::FixCandidate,
        Verdict::Changes,
    );
    assert_eq!(
        s.sentences,
        [
            "Fix candidate: a guard clause in f, with test changes in tests/test_a.py.",
            "Changes behaviour: a guard clause in f."
        ]
    );
}

#[test]
fn a_guard_alone_is_still_a_fix_candidate_and_says_no_test_was_touched() {
    let s = is(
        json!([{"path": "src/a.py",
             "old": "def f(x):\n    y = x + 1\n    return y\n",
             "new": "def f(x):\n    if x is None:\n        return None\n    y = x + 1\n    return y\n"}]),
        Label::FixCandidate,
        Verdict::Changes,
    );
    assert!(
        s.sentences[0].ends_with("with no test touched."),
        "{:?}",
        s.sentences
    );
}

#[test]
fn new_definitions_are_a_feature() {
    let s = is(
        json!([
            {"path": "src/a.py",
             "old": "def f(x):\n    return x\n",
             "new": "def f(x):\n    return x\n\ndef g(x):\n    return f(x) * 2\n"},
            {"path": "tests/test_a.py",
             "old": "def test_f():\n    assert f(1) == 1\n",
             "new": "def test_f():\n    assert f(1) == 1\n\ndef test_g():\n    assert g(1) == 2\n"}
        ]),
        Label::Feature,
        Verdict::Unknown,
    );
    assert_eq!(
        s.sentences[0],
        "Adds g, with test changes in tests/test_a.py."
    );
}

#[test]
fn edited_code_with_a_test_alongside_is_a_fix_candidate() {
    let s = is(
        json!([
            {"path": "src/a.py",
             "old": "def f(x):\n    y = x + 1\n    return y\n",
             "new": "def f(x):\n    y = x + 2\n    return y * y\n"},
            {"path": "tests/test_a.py",
             "old": "def test_f():\n    assert f(1) == 2\n",
             "new": "def test_f():\n    assert f(1) == 9\n"}
        ]),
        Label::FixCandidate,
        Verdict::Unknown,
    );
    assert_eq!(
        s.sentences[0],
        "Fix candidate: edits f, with test changes in tests/test_a.py."
    );
}

#[test]
fn edited_code_without_a_test_is_left_unlabelled_and_unknown() {
    let s = is(
        json!([{"path": "src/a.py",
             "old": "def f(x):\n    y = x + 1\n    return y\n",
             "new": "def f(x):\n    y = x + 2\n    return y * y\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
    assert_eq!(
        s.sentences,
        [
            "Edits f, with no test touched.",
            "Behaviour change not ruled out: 1 hunk edits beyond a rename, move, extraction or reformat."
        ]
    );
}

#[test]
fn a_changed_literal_changes_behaviour() {
    is(
        json!([{"path": "src/a.py",
             "old": "def f():\n    return 1\n",
             "new": "def f():\n    return 2\n"}]),
        Label::Change,
        Verdict::Changes,
    );
}

#[test]
fn docs_tests_and_configuration_are_labelled_by_their_paths() {
    let s = is(
        json!([{"path": "README.md", "old": "# A\n\ntext\n", "new": "# A\n\nmore text\n"}]),
        Label::Docs,
        Verdict::Preserving,
    );
    assert_eq!(
        s.sentences[1],
        "Behaviour-preserving: only documentation changed."
    );
    is(
        json!([{"path": "tests/test_a.py",
            "old": "def test_f():\n    assert f(1) == 2\n",
            "new": "def test_f():\n    assert f(2) == 3\n"}]),
        Label::Test,
        Verdict::Unknown,
    );
    is(
        json!([{"path": "ci.yml", "old": "svc:\n  port: 80\n", "new": "svc:\n  port: 443\n"}]),
        Label::Chore,
        Verdict::Changes,
    );
}

#[test]
fn a_dependency_list_is_not_documentation() {
    let s = summary(json!([{"path": "requirements.txt",
        "old": "requests==2.31\n", "new": "requests==2.32\n"}]));
    assert_ne!(s.label, Label::Docs, "{:?}", s.sentences);
    assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
}

#[test]
fn code_named_like_a_doc_file_is_code() {
    is(
        json!([{"path": "src/changes.py",
            "old": "def f():\n    return g(1)\n",
            "new": "def f():\n    return h(1)\n"}]),
        Label::Change,
        Verdict::Changes,
    );
}

#[test]
fn a_comment_only_code_change_is_docs() {
    is(
        json!([{"path": "src/a.py",
            "old": "def f(x):\n    # add one\n    return x + 1\n",
            "new": "def f(x):\n    # adds one to x\n    return x + 1\n"}]),
        Label::Docs,
        Verdict::Preserving,
    );
}

#[test]
fn whitespace_between_tokens_is_formatting() {
    is(
        json!([{"path": "src/a.js",
            "old": "function f(a, b) {\n  return a+b;\n}\n",
            "new": "function f(a, b) {\n  return a + b;\n}\n"}]),
        Label::Refactor,
        Verdict::Preserving,
    );
    // a call's arguments rewrapped onto one line
    is(
        json!([{"path": "src/a.rs",
            "old": "fn f() {\n    let x = g(\n        1,\n        2\n    );\n    h(x);\n}\n",
            "new": "fn f() {\n    let x = g(1, 2);\n    h(x);\n}\n"}]),
        Label::Refactor,
        Verdict::Preserving,
    );
}

#[test]
fn whitespace_that_means_something_is_not_formatting() {
    // inside a string, the text changed
    is(
        json!([{"path": "src/a.js",
            "old": "function f() {\n  return \"a  b\";\n}\n",
            "new": "function f() {\n  return \"a b\";\n}\n"}]),
        Label::Change,
        Verdict::Changes,
    );
    // inside a string that spans lines
    for changes in [
        json!([{"path": "src/a.js",
            "old": "function f() {\n  return `a\n  b  c`;\n}\n",
            "new": "function f() {\n  return `a\n  b c`;\n}\n"}]),
        json!([{"path": "src/a.py",
            "old": "def f():\n    return \"\"\"\n    select  x\n    \"\"\"\n",
            "new": "def f():\n    return \"\"\"\n    select x\n    \"\"\"\n"}]),
    ] {
        let s = summary(changes);
        assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
    }
    // a line joined onto a comment is commented out
    is(
        json!([{"path": "src/a.js",
            "old": "function f(a) {\n  g(a); // note\n  h(a);\n}\n",
            "new": "function f(a) {\n  g(a); // note h(a);\n}\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
    // a newline after `return` ends the statement
    is(
        json!([{"path": "src/a.js",
            "old": "function f(a) {\n  g(a);\n  return a;\n}\n",
            "new": "function f(a) {\n  g(a);\n  return\n  a;\n}\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
    // where indentation is syntax, a dedent moves a statement out of the loop
    is(
        json!([{"path": "src/a.py",
            "old": "def f(xs):\n    t = 0\n    for x in xs:\n        t += x\n        log(t)\n    return t\n",
            "new": "def f(xs):\n    t = 0\n    for x in xs:\n        t += x\n    log(t)\n    return t\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
    // between jsx tags a newline drops the space a space keeps
    is(
        json!([{"path": "src/a.jsx",
            "old": "const A = () => (\n  <p>\n    <b>a</b> <i>b</i>\n  </p>\n);\n",
            "new": "const A = () => (\n  <p>\n    <b>a</b>\n    <i>b</i>\n  </p>\n);\n"}]),
        Label::Change,
        Verdict::Unknown,
    );
}

#[test]
fn a_definition_moved_to_another_file_preserves_behaviour() {
    let helper = "def helper(x):\n    total = x * 2\n    return total + 1\n";
    let s = is(
        json!([
            {"path": "a.py",
             "old": format!("def main():\n    return 1\n\n\n{helper}"),
             "new": "def main():\n    return 1\n"},
            {"path": "b.py",
             "old": "def other():\n    return 2\n",
             "new": format!("def other():\n    return 2\n\n\n{helper}")}
        ]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(s.sentences[0], "Refactor: moves helper.");
}

#[test]
fn a_statement_moved_beside_a_definition_is_not_part_of_the_move() {
    let helper = "def helper(x):\n    total = x * 2\n    return total + 1\n";
    let s = summary(json!([
        {"path": "a.py",
         "old": format!("def main():\n    return 1\n\n\nregister('a')\n{helper}"),
         "new": "def main():\n    return 1\n"},
        {"path": "b.py",
         "old": "def other():\n    return 2\n",
         "new": format!("def other():\n    return 2\n\n\nregister('a')\n{helper}")}
    ]));
    assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
}

#[test]
fn a_definition_moved_within_its_file_preserves_behaviour() {
    let (a, b, c) = (
        "    def a(self, x):\n        y = x * 2\n        return y + 1\n",
        "    def b(self, x):\n        return x - 1\n",
        "    def c(self, x):\n        z = x + 3\n        return z * z\n",
    );
    let s = is(
        json!([{"path": "m.py",
            "old": format!("class K:\n{a}\n{b}\n{c}"),
            "new": format!("class K:\n{b}\n{c}\n{a}")}]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(s.sentences[0], "Refactor: reorders a.");
    assert_eq!(
        s.sentences[1],
        "Behaviour-preserving: every hunk is a reordering."
    );
    let f = "fn f() -> u8 {\n    let y = 2;\n    y + 1\n}\n";
    let g = "fn g() -> u8 {\n    0\n}\n";
    is(
        json!([{"path": "m.rs", "old": format!("{f}\n{g}"), "new": format!("{g}\n{f}")}]),
        Label::Refactor,
        Verdict::Preserving,
    );
    let unproven = |path: &str, old: String, new: String| {
        let s = summary(json!([{"path": path, "old": old, "new": new}]));
        assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
    };
    // a decorator may register in file order, and a definition that changed
    // on the way did not only move
    unproven(
        "m.py",
        format!("class K:\n    @route\n{a}\n{b}\n{c}"),
        format!("class K:\n{b}\n{c}\n    @route\n{a}"),
    );
    unproven(
        "m.py",
        format!("class K:\n{a}\n{b}\n{c}"),
        format!("class K:\n{b}\n{c}\n{}", a.replace("+ 1", "+ 2")),
    );
    // a module runs top to bottom: a function moved below a statement that
    // calls it is not defined yet when it does; a css rule's place is the cascade
    let top = |d: &str| d.lines().map(|l| &l[4..]).collect::<Vec<_>>().join("\n") + "\n";
    unproven(
        "m.py",
        format!("{}\nx = a(1)\n\n{}", top(a), top(b)),
        format!("x = a(1)\n\n{}\n{}", top(b), top(a)),
    );
    unproven(
        "m.css",
        ".a {\n  color: red;\n}\n\n.b {\n  color: blue;\n}\n".into(),
        ".b {\n  color: blue;\n}\n\n.a {\n  color: red;\n}\n".into(),
    );
    // the same lines in another container are another definition
    let f = "    fn f(&self) -> u8 {\n        let y = 2;\n        y + 1\n    }\n";
    unproven(
        "m.rs",
        format!("impl A {{\n{f}}}\n\nimpl B {{\n    fn g(&self) -> u8 {{\n        0\n    }}\n}}\n"),
        format!("impl A {{\n}}\n\nimpl B {{\n    fn g(&self) -> u8 {{\n        0\n    }}\n{f}}}\n"),
    );
}

#[test]
fn a_literal_named_by_the_constant_that_holds_it_preserves_behaviour() {
    let s = is(
        json!([{"path": "a.rs",
            "old": "fn wait() -> u64 {\n    let t = 30;\n    t\n}\n",
            "new": "const TIMEOUT: u64 = 30;\n\nfn wait() -> u64 {\n    let t = TIMEOUT;\n    t\n}\n"}]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(s.sentences[0], "Refactor: names 30 as TIMEOUT.");
    // a constant holding another value, or bound twice, proves nothing
    for consts in [
        "const TIMEOUT: u64 = 31;\n",
        "const TIMEOUT: u64 = 30;\nmod m {\n    const TIMEOUT: u64 = 5;\n}\n",
    ] {
        let s = summary(json!([{"path": "a.rs",
            "old": "fn wait() -> u64 {\n    let t = 30;\n    t\n}\n",
            "new": format!("{consts}\nfn wait() -> u64 {{\n    let t = TIMEOUT;\n    t\n}}\n")}]));
        assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
    }
    // a keyword argument spelling the name binds nothing, and a qualified
    // constant may live where the lines do not show
    for (old, new) in [
        (
            "def wait():\n    return 30\n\ncall(\n    TIMEOUT=30,\n)\n",
            "def wait():\n    return TIMEOUT\n\ncall(\n    TIMEOUT=30,\n)\n",
        ),
        (
            "TIMEOUT = 30\n\ndef wait():\n    return 30\n",
            "TIMEOUT = 30\n\ndef wait():\n    return cfg.TIMEOUT\n",
        ),
    ] {
        let s = summary(json!([{"path": "a.py", "old": old, "new": new}]));
        assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
    }
}

#[test]
fn the_review_pack_carries_the_summary() {
    let out = run_json(json!({"changes": [{"path": "src/a.py",
        "old": "def f():\n    return 1\n", "new": "def f():\n    return 2\n"}]}));
    let p = ordo_diff::pack(&out);
    assert!(
        p.contains("## summary\nlabel: change; verdict: changes\nEdits f, with no test touched.\nChanges behaviour: a value changed in f.\n"),
        "{p}"
    );
}

#[test]
fn an_extraction_preserves_behaviour_only_when_its_lines_left_the_source() {
    let old = "def main(xs):\n    print('start')\n    total = 0\n    for x in xs:\n        total += x\n    return total\n";
    let new = "def main(xs):\n    print('start')\n    return total_of(xs)\n\n\ndef total_of(xs):\n    total = 0\n    for x in xs:\n        total += x\n    return total\n";
    let s = is(
        json!([{"path": "a.py", "old": old, "new": new}]),
        Label::Refactor,
        Verdict::Preserving,
    );
    assert_eq!(s.sentences[0], "Refactor: extracts total_of.");
    // the extracted body counts differently: not the code that left
    let changed = new.replace("total += x", "total += x * 2");
    let s = summary(json!([{"path": "a.py", "old": old, "new": changed}]));
    assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
}

#[test]
fn a_renamed_config_key_is_no_rename() {
    let s = summary(json!([{"path": "ci.yml",
        "old": "build:\n  run: make\n  shell: bash\n",
        "new": "compile:\n  run: make\n  shell: bash\n"}]));
    assert_eq!(s.label, Label::Chore);
    assert_ne!(s.verdict, Verdict::Preserving, "{:?}", s.sentences);
}

#[test]
fn a_change_without_hunks_has_no_summary() {
    let out = run_json(json!({"changes": [{"path": "a.py", "old": "x = 1\n", "new": "x = 1\n"}]}));
    assert!(out.summary.is_none());
}
