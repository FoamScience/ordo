//! make: a rule is a definition named by its target and a prerequisite is a
//! use of another target, so a makefile's own dependency graph becomes the
//! reading order. Targets, prerequisites and variable names are all bare
//! `word` nodes, so uses are read from the parents that make one a reference.
use ordo::model::{HunkOut, Input, Output};

fn run(path: &str, old: &str, new: &str) -> Output {
    ordo::run(
        serde_json::from_value::<Input>(serde_json::json!({
            "changes": [{ "path": path, "old": old, "new": new }]
        }))
        .unwrap(),
    )
}

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    run(path, old, new)
        .files
        .into_iter()
        .flat_map(|f| f.hunks)
        .collect()
}

#[test]
fn a_makefile_is_matched_by_name_and_by_extension() {
    for path in ["Makefile", "GNUmakefile", "common.mk"] {
        let hs = one(path, "CC = gcc\n", "CC = gcc\nLD = ld\n");
        assert!(
            hs.iter().any(|h| h.defines.contains(&"LD".to_string())),
            "{path}: {hs:?}"
        );
    }
}

#[test]
fn a_prerequisite_is_a_use_of_the_target_it_names() {
    let old = "build:\n\t$(CC) -o app a.c\n";
    let new = "build:\n\t$(CC) -o app a.c\n\ntest: build\n\t./app\n";
    let out = run("Makefile", old, new);
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(
        hs.iter().any(|h| h.defines.contains(&"test".to_string())),
        "{hs:?}"
    );
    assert!(
        hs.iter().any(|h| h.uses.contains(&"build".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_variable_definition_links_to_its_reference() {
    let old = "build:\n\t gcc -o app a.c\n";
    let new = "SRCS := a.c b.c\n\nbuild:\n\t gcc -o app $(SRCS)\n";
    let out = run("Makefile", old, new);
    assert!(
        out.edges.iter().any(|e| e.why.contains("SRCS")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_special_target_is_not_a_definition() {
    // `.PHONY` names no recipe anyone navigates to — but the targets it lists
    // are still uses of the real rules
    let hs = one(
        "Makefile",
        "all:\n\t@echo hi\n",
        ".PHONY: all\n\nall:\n\t@echo hi\n",
    );
    assert!(
        !hs.iter().any(|h| h.defines.contains(&".PHONY".to_string())),
        "{hs:?}"
    );
    assert!(
        hs.iter().any(|h| h.uses.contains(&"all".to_string())),
        "{hs:?}"
    );
}

#[test]
fn include_names_the_makefile_it_pulls_in() {
    let hs = one(
        "Makefile",
        "CC = gcc\n\nall:\n\t@echo hi\n",
        "CC = gcc\ninclude common.mk\n\nall:\n\t@echo hi\n",
    );
    assert!(
        hs.iter().any(|h| h.rationale.contains("common.mk")),
        "{hs:?}"
    );
}
