//! How much of a hunk changed, as `extent` names it.
mod fixture;
use fixture::hunks;
use ordo_diff::model::Extent;

fn extent(path: &str, old: &str, new: &str) -> Vec<Option<Extent>> {
    hunks(path, old, new).iter().map(|h| h.extent).collect()
}

#[test]
fn one_sided_hunks_are_added_or_removed() {
    assert_eq!(
        extent("a.py", "x = 1\n", "x = 1\ny = 2\n"),
        vec![Some(Extent::Added)]
    );
    assert_eq!(
        extent("a.py", "x = 1\ny = 2\n", "x = 1\n"),
        vec![Some(Extent::Removed)]
    );
}

#[test]
fn blank_lines_and_reindenting_are_whitespace() {
    let old = "def f():\n    return g(a, b)\n";
    let new = "def f():\n\n    return g(a,  b)\n";
    assert_eq!(extent("a.py", old, new), vec![Some(Extent::Whitespace)]);
}

#[test]
fn a_call_rewrapped_with_a_trailing_comma_is_formatting() {
    let old = "x = compute(alpha, beta)\n";
    let new = "x = compute(\n    alpha,\n    beta,\n)\n";
    assert_eq!(extent("a.py", old, new), vec![Some(Extent::Formatting)]);
}

#[test]
fn one_argument_changed_is_an_edit() {
    let old = "x = compute(alpha, 3, verbose=True)\n";
    let new = "x = compute(alpha, 4, verbose=True)\n";
    assert_eq!(extent("a.py", old, new), vec![Some(Extent::Edited)]);
}

#[test]
fn a_year_bumped_in_a_comment_is_an_edit_not_a_rewrite() {
    // the comment is one leaf; counted by its words, one of eight changed
    let old = "# Copyright (C) 2013-2024 OpenFOAM Foundation\nx = 1\n";
    let new = "# Copyright (C) 2013-2026 OpenFOAM Foundation\nx = 1\n";
    assert_eq!(extent("a.py", old, new), vec![Some(Extent::Edited)]);
}

#[test]
fn new_statements_in_place_of_old_are_a_rewrite() {
    let old = "def f():\n    cache = build(config)\n    return cache\n";
    let new = "def f():\n    result = fetch(url, timeout=5)\n    log(result)\n";
    assert_eq!(extent("a.py", old, new), vec![Some(Extent::Rewritten)]);
}

#[test]
fn prose_counts_by_its_words() {
    let old = "# Title\n\nVersion 1.28.0 or later is required for the lookup.\n";
    let new = "# Title\n\nVersion 1.28.0 or greater is required for the lookup.\n";
    assert_eq!(extent("a.md", old, new), vec![Some(Extent::Edited)]);
}
