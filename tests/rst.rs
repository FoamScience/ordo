//! reStructuredText: a def is a section, as in markdown. The grammar keeps
//! sections flat, so a section's span and its parents come from the headings
//! around it, levelled by the order their adornment styles first appear.
mod fixture;
use fixture::hunks as one;

const DOC: &str = "\
=======
Project
=======

Intro.

Install
=======

Run it.

From source
-----------

Clone.

Usage
=====

Call it.
";

#[test]
fn an_edit_is_read_against_its_section_path() {
    let new = DOC.replace("Clone.", "Clone and build.");
    let hs = one("a.rst", DOC, &new);
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(
        hs[0].enclosing.as_deref(),
        Some("Project > Install > From source")
    );
}

#[test]
fn a_section_ends_at_the_next_heading_of_its_level() {
    let new = DOC.replace("Call it.", "Call it twice.");
    let hs = one("a.rst", DOC, &new);
    assert_eq!(
        hs[0].enclosing.as_deref(),
        Some("Project > Usage"),
        "{hs:?}"
    );
}

#[test]
fn adds_section_wording_for_a_new_subsection() {
    let new = DOC.replace("Clone.\n", "Clone.\n\nFrom pip\n--------\n\nInstall it.\n");
    let hs = one("a.rst", DOC, &new);
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"From pip".to_string()))
        .unwrap_or_else(|| panic!("{hs:?}"));
    assert_eq!(h.rationale, "adds section From pip");
    assert_eq!(h.symbols[0].kind, "section");
}

#[test]
fn a_bullet_is_not_a_comment() {
    let old = "Title\n=====\n\n* one\n";
    let new = "Title\n=====\n\n* one\n* two\n";
    let hs = one("a.rst", old, new);
    assert!(!hs[0].rationale.contains("comment"), "{hs:?}");
    assert_eq!(hs[0].enclosing.as_deref(), Some("Title"), "{hs:?}");
}
