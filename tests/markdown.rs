//! Markdown v1: a def is a section (heading + its content). Covers section
//! naming, the nested section path, prose wording ("adds section X"), and the
//! P15 detail layer reporting a subsection added to its parent.
use ordo::model::{HunkOut, Input};

fn hunks(v: serde_json::Value) -> Vec<HunkOut> {
    let inp: Input = serde_json::from_value(v).unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks)
        .collect()
}

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    hunks(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
}

#[test]
fn unsupported_is_false_for_markdown() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "README.md", "old": "# A\n", "new": "# A\n\nx\n" }]
    }))
    .unwrap();
    let out = ordo::run(inp);
    assert!(!out.files[0].unsupported, "{:?}", out.files[0].unsupported);
}

#[test]
fn section_named_from_heading_text() {
    let hs = one(
        "a.md",
        "# Project\n",
        "# Project\n\n## Install\n\nRun it.\n",
    );
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"Install".to_string()));
    assert!(h.is_some(), "{hs:?}");
    assert_eq!(h.unwrap().symbols[0].kind, "section");
}

#[test]
fn nested_section_path() {
    let old = "# Project\n\n## Install\n\n### From source\n\nClone.\n";
    let new = "# Project\n\n## Install\n\n### From source\n\nClone and build.\n";
    let hs = one("a.md", old, new);
    // editing prose under "From source" reports the full qualified path
    let h = hs
        .iter()
        .find(|h| h.enclosing.as_deref() == Some("Project > Install > From source"));
    assert!(h.is_some(), "{hs:?}");
}

#[test]
fn adds_section_wording_for_a_new_subsection() {
    let old = "# Project\n\n## Install\n\nRun it.\n\n### From source\n\nClone.\n";
    let new =
        "# Project\n\n## Install\n\nRun it.\n\n### From source\n\nClone.\n\n### Usage\n\nRun it.\n";
    let hs = one("a.md", old, new);
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"Usage".to_string()))
        .expect("hunk defining Usage");
    assert_eq!(h.rationale, "adds section Usage");
    assert_eq!(h.enclosing.as_deref(), Some("Project > Install"));
}

#[test]
fn p15_reports_the_subsection_added_to_its_parent() {
    let old = "# Project\n\n## Install\n\nRun it.\n\n### From source\n\nClone.\n";
    let new =
        "# Project\n\n## Install\n\nRun it.\n\n### From source\n\nClone.\n\n### Usage\n\nRun it.\n";
    let hs = one("a.md", old, new);
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"Usage".to_string()))
        .expect("hunk defining Usage");
    assert!(
        h.details
            .iter()
            .any(|d| d == "adds section Usage to Project > Install"),
        "{:?}",
        h.details
    );
}

#[test]
fn edits_section_wording_for_a_prose_only_change() {
    let old = "# Project\n\n## Install\n\nRun it.\n";
    let new = "# Project\n\n## Install\n\nRun the installer.\n";
    let hs = one("a.md", old, new);
    let h = hs
        .iter()
        .find(|h| h.rationale.starts_with("edits section"))
        .expect("prose-only edit hunk");
    assert_eq!(h.rationale, "edits section Project > Install");
    assert!(h.defines.is_empty());
}

#[test]
fn removes_section_wording_for_a_deleted_subsection() {
    let old = "# Project\n\n## Install\n\nRun it.\n\n### Usage\n\nRun it.\n";
    let new = "# Project\n\n## Install\n\nRun it.\n";
    let hs = one("a.md", old, new);
    assert!(
        hs.iter().any(|h| h.rationale == "removes section Usage"),
        "{hs:?}"
    );
}

#[test]
fn a_hunk_with_a_heading_is_a_definition_category() {
    let old = "# Project\n";
    let new = "# Project\n\n## Install\n\nRun it.\n";
    let hs = one("a.md", old, new);
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"Install".to_string()));
    assert_eq!(h.unwrap().category, ordo::model::Category::Definition);
}

#[test]
fn section_paths_use_an_arrow_not_a_dot() {
    // a dot reads like a code path; heading nesting is not one
    let rats: Vec<String> = one(
        "d.md",
        "# Project\n\n## Install\n\nRun it.\n",
        "# Project\n\n## Install\n\nRun it, revised.\n",
    )
    .into_iter()
    .map(|h| h.rationale)
    .collect();
    assert!(
        rats.iter().any(|r| r.contains("Project > Install")),
        "prose scope joins with an arrow: {rats:?}"
    );
    assert!(
        !rats.iter().any(|r| r.contains("Project.Install")),
        "prose scope must not use a dot: {rats:?}"
    );
}

#[test]
fn content_before_the_first_heading_has_no_enclosing_section() {
    // fmt's README opens with an <img> and badge links — a headless leading
    // section, which must not surface as `<anonymous>` (found by the corpus sweep)
    let rats: Vec<String> = one(
        "README.md",
        "<img src=\"x.png\">\n\n# Title\n\nProse.\n",
        "<img src=\"x.png\">\n[![b](b.svg)](u)\n\n# Title\n\nProse.\n",
    )
    .into_iter()
    .map(|h| h.rationale)
    .collect();
    assert!(
        !rats.iter().any(|r| r.contains("<anonymous>")),
        "headless leading section must not be anonymous: {rats:?}"
    );
}
