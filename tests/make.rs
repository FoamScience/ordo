//! make: a rule is a definition named by its target and a prerequisite is a
//! use of another target, so a makefile's own dependency graph becomes the
//! reading order. Targets, prerequisites and variable names are all bare
//! `word` nodes, so uses are read from the parents that make one a reference.
mod fixture;
use fixture::{hunks as one, run_file as run};

#[test]
fn a_makefile_is_matched_by_name_and_by_extension() {
    for path in ["Makefile", "GNUmakefile", "common.mk"] {
        let hs = one(path, "CC = gcc\n", "CC = gcc\nLD = ld\n");
        let defines: Vec<&[String]> = hs.iter().map(|h| h.defines.as_slice()).collect();
        assert_eq!(defines, vec![["LD".to_string()].as_slice()], "{path}");
    }
}

#[test]
fn a_prerequisite_is_a_use_of_the_target_it_names() {
    let old = "build:\n\t$(CC) -o app a.c\n";
    let new = "build:\n\t$(CC) -o app a.c\n\ntest: build\n\t./app\n";
    let out = run("Makefile", old, new);
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    // one hunk: the new rule. It defines the target and uses its prerequisite,
    // and `any` would have passed even if those had landed on separate hunks
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].defines, vec!["test".to_string()]);
    assert_eq!(hs[0].uses, vec!["build".to_string()]);
}

#[test]
fn a_variable_definition_links_to_its_reference() {
    let old = "build:\n\t gcc -o app a.c\n";
    let new = "SRCS := a.c b.c\n\nbuild:\n\t gcc -o app $(SRCS)\n";
    let out = run("Makefile", old, new);
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: SRCS"]);
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
    // the negative is the point of the test and stays a universal claim
    assert!(
        !hs.iter().any(|h| h.defines.contains(&".PHONY".to_string())),
        "{hs:?}"
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(hs[0].uses, vec!["all".to_string()]);
    assert!(hs[0].defines.is_empty(), "{:?}", hs[0].defines);
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
