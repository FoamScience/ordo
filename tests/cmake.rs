//! cmake: every construct is a command, so which command a node *is* lives in
//! its identifier rather than its node kind. `function`/`macro` define,
//! `set`/`option` bind a name, `include`/`find_package`/`add_subdirectory` are
//! imports, and every other command is transparent.
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
fn cmakelists_is_matched_by_filename() {
    // `.txt` says nothing about the format
    let hs = one(
        "CMakeLists.txt",
        "project(demo)\n",
        "project(demo)\nset(SOURCES a.cpp)\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.defines.contains(&"SOURCES".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_function_defines_and_a_call_uses_it() {
    let old = "function(caller)\n  message(hi)\nendfunction()\n";
    let new = "function(helper a)\n  message(${a})\nendfunction()\n\n\
               function(caller)\n  helper(1)\nendfunction()\n";
    let out = run("cmake/x.cmake", old, new);
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(
        hs.iter().any(|h| h.defines.contains(&"helper".to_string())),
        "{hs:?}"
    );
    assert!(
        out.edges.iter().any(|e| e.why.contains("helper")),
        "{:?}",
        out.edges
    );
    // a parameter is a binding, not a use of whatever else is called `a`
    assert!(
        !hs.iter().any(|h| h.uses.contains(&"a".to_string())),
        "{hs:?}"
    );
}

#[test]
fn include_and_find_package_are_imports() {
    let hs = one(
        "CMakeLists.txt",
        "project(demo)\n\nadd_library(core a.cpp)\n",
        "project(demo)\ninclude(Utils)\nfind_package(Boost REQUIRED)\n\nadd_library(core a.cpp)\n",
    );
    let h = hs.iter().find(|h| h.rationale.starts_with("adds import"));
    assert!(h.is_some(), "{hs:?}");
    let r = &h.unwrap().rationale;
    assert!(r.contains("Utils") && r.contains("Boost"), "{r}");
}

#[test]
fn an_ordinary_command_defines_nothing() {
    // `message()` is not a definition — only set/option/function/macro are
    let hs = one(
        "x.cmake",
        "set(X 1)\n",
        "set(X 1)\nmessage(STATUS \"hello\")\n",
    );
    assert!(
        !hs.iter().any(|h| h.defines.contains(&"STATUS".to_string())),
        "{hs:?}"
    );
}
