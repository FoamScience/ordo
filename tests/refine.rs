//! Intra-line refinement: only the part of a line that changed is reported.
use ordo::refine::Refiner;

fn lines(s: &str) -> Vec<String> {
    s.lines().map(String::from).collect()
}

/// The spans, rendered back as the substrings they cover — asserting on text
/// rather than column numbers, so a failure says what it highlighted.
fn texts(line: &str, spans: &Option<Vec<(usize, usize)>>) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    spans
        .as_ref()
        .map(|v| {
            v.iter()
                .map(|&(s, e)| chars[s..e].iter().collect())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn an_added_parameter_is_the_only_thing_highlighted() {
    let old =
        lines("fn content_matches(info: &AgentInfo, node: &AgentNode) -> bool {\n    true\n}\n");
    let new = lines(
        "fn content_matches(info: &AgentInfo, node: &AgentNode, label: Option<&str>) -> bool {\n    true\n}\n",
    );
    let r = Refiner::new("a.rs", &old, &new).expect("rust parses");
    let d = r.refine([1, 1], [1, 1]);

    assert_eq!(texts(&old[0], &d.removed[0]), Vec::<String>::new());
    assert_eq!(texts(&new[0], &d.added[0]), vec![", label: Option<&str>"]);
}

#[test]
fn a_changed_argument_highlights_both_sides() {
    let old = lines("x = compute(alpha, 3, verbose=True)\n");
    let new = lines("x = compute(alpha, 4, verbose=True)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    assert_eq!(texts(&old[0], &d.removed[0]), vec!["3"]);
    assert_eq!(texts(&new[0], &d.added[0]), vec!["4"]);
}

#[test]
fn tokens_come_from_the_grammar_not_from_splitting_on_characters() {
    // `AgentNode` -> `AgentNodeRef` is a whole identifier replaced, not a
    // three-character suffix appended: a character diff would say "Ref".
    let old = lines("fn f(node: &AgentNode) {}\n");
    let new = lines("fn f(node: &AgentNodeRef) {}\n");
    let r = Refiner::new("a.rs", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    assert_eq!(texts(&old[0], &d.removed[0]), vec!["AgentNode"]);
    assert_eq!(texts(&new[0], &d.added[0]), vec!["AgentNodeRef"]);
}

#[test]
fn an_unrelated_line_is_left_whole() {
    // nothing in common: refining would invent a relationship
    let old = lines("import os\n");
    let new = lines("total = sum(values) / len(values)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    assert_eq!(d.removed[0], None);
    assert_eq!(d.added[0], None);
}

#[test]
fn a_pure_addition_has_nothing_to_pair_with() {
    let new = lines("def a():\n    return 1\n\n\ndef b():\n    return 2\n");
    let r = Refiner::new("a.py", &lines("def a():\n    return 1\n"), &new).unwrap();
    let d = r.refine([3, 2], [4, 6]); // empty old side

    assert!(d.removed.is_empty());
    assert!(d.added.iter().all(|s| s.is_none()));
}

#[test]
fn multi_line_hunks_pair_line_by_line_in_order() {
    let old = lines("a = f(1)\nb = g(2)\nc = h(3)\n");
    let new = lines("a = f(9)\nb = g(2)\nc = h(7)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 3], [1, 3]);

    assert_eq!(texts(&new[0], &d.added[0]), vec!["9"]);
    assert_eq!(texts(&new[2], &d.added[2]), vec!["7"]);
}

#[test]
fn an_unsupported_path_refuses_rather_than_guessing() {
    assert!(Refiner::new("a.png", &lines("x\n"), &lines("y\n")).is_none());
}

#[test]
fn a_long_hunk_still_pairs_every_line() {
    // the pairing backtrack used to reconstruct the fill's choice by testing
    // accumulated f32 sums against f32::EPSILON; over enough lines the sums
    // grow past that tolerance and pairs were silently dropped
    let n = 60;
    let old: Vec<String> = (0..n).map(|i| format!("let v{i} = compute(a);")).collect();
    let new: Vec<String> = (0..n).map(|i| format!("let v{i} = compute(b);")).collect();
    let r = Refiner::new("a.rs", &old, &new).expect("rust grammar");
    let out = r.refine([1, n], [1, n]);
    let unpaired = out.removed.iter().filter(|s| s.is_none()).count();
    assert_eq!(unpaired, 0, "every line is the same line edited");
    for (i, spans) in out.added.iter().enumerate() {
        assert_eq!(texts(&new[i], spans), vec!["b".to_string()], "line {i}");
    }
}

#[test]
fn a_machine_generated_line_renders_whole() {
    // one minified line carries six figures of leaves; the LCS table is
    // (n+1)*(m+1) u32s, so it must refuse rather than allocate gigabytes
    let long = format!("var x=[{}];", vec!["1"; 40_000].join(","));
    let old = vec![long.clone()];
    let new = vec![long.replace("var x", "var y")];
    let r = Refiner::new("a.js", &old, &new).expect("javascript grammar");
    let out = r.refine([1, 1], [1, 1]);
    assert_eq!(out.added, vec![None], "too large to refine → renders whole");
}

/// The moved spans of one line, as text.
fn moved(line: &str, spans: &[Vec<(usize, usize)>], k: usize) -> Vec<String> {
    texts(line, &spans.get(k).cloned())
}

#[test]
fn swapped_arguments_read_as_moved_not_rewritten() {
    let old = lines("x = scale(width, height)\n");
    let new = lines("x = scale(height, width)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    let gone = moved(&old[0], &d.moved_removed, 0);
    let came = moved(&new[0], &d.moved_added, 0);
    assert_eq!(
        gone.len(),
        1,
        "one argument kept its place, one moved: {gone:?}"
    );
    assert_eq!(gone, came);
}

#[test]
fn a_statement_moved_down_the_hunk_is_moved_whole() {
    let old = lines("total = 0\nlog.debug(\"start\")\ncount = 0\n");
    let new = lines("total = 0\ncount = 0\nlog.debug(\"start\")\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 3], [1, 3]);

    assert_eq!(
        moved(&old[1], &d.moved_removed, 1),
        vec!["log.debug(\"start\")"]
    );
    assert_eq!(
        moved(&new[2], &d.moved_added, 2),
        vec!["log.debug(\"start\")"]
    );
    assert!(d.moved_removed[0].is_empty() && d.moved_added[0].is_empty());
}

#[test]
fn one_shared_name_between_two_rewrites_is_not_a_move() {
    let old = lines("self.cache = build(config)\n");
    let new = lines("result = fetch(self)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    assert!(
        d.moved_removed.iter().all(Vec::is_empty),
        "{:?}",
        d.moved_removed
    );
    assert!(
        d.moved_added.iter().all(Vec::is_empty),
        "{:?}",
        d.moved_added
    );
}

#[test]
fn an_edit_in_place_moves_nothing() {
    let old = lines("x = compute(alpha, 3, verbose=True)\n");
    let new = lines("x = compute(alpha, 4, verbose=True)\n");
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);

    assert_eq!(d.moved_removed, vec![Vec::<(usize, usize)>::new()]);
    assert_eq!(d.moved_added, vec![Vec::<(usize, usize)>::new()]);
}

#[test]
fn an_edited_word_in_a_comment_or_string_marks_the_word() {
    let old = lines(
        "# Copyright (C) 2013-2024 OpenFOAM Foundation\nmsg = \"requires c-ares or newer\"\n",
    );
    let new = lines(
        "# Copyright (C) 2013-2026 OpenFOAM Foundation\nmsg = \"requires c-ares or greater\"\n",
    );
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 2], [1, 2]);

    assert_eq!(texts(&new[0], &d.added[0]), vec!["2026"]);
    assert_eq!(texts(&old[1], &d.removed[1]), vec!["newer"]);
    assert_eq!(texts(&new[1], &d.added[1]), vec!["greater"]);
}

#[test]
fn a_hunk_too_scrambled_to_search_reports_no_moves() {
    // every argument reversed leaves hundreds of unmatched leaves a side; the
    // greedy search is quadratic per round, so past its cap it does not run
    let args = |rev: bool| {
        let mut v: Vec<String> = (0..300).map(|i| format!("a{i}")).collect();
        if rev {
            v.reverse();
        }
        v.join(", ")
    };
    let old = vec![format!("x = f({})", args(false))];
    let new = vec![format!("x = f({})", args(true))];
    let r = Refiner::new("a.py", &old, &new).unwrap();
    let d = r.refine([1, 1], [1, 1]);
    assert!(d.moved_added.iter().all(Vec::is_empty));
}
