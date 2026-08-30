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
