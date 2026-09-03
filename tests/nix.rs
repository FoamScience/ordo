//! nix: an attribute set is the language's main structure, so a `binding` is
//! both a definition and a member of the set above it — the config shape. A
//! function is not a separate declaration (it is a lambda bound to an
//! attribute), and `import ./x.nix` is an ordinary application whose function
//! happens to be named `import`.
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
fn a_let_binding_links_to_where_it_is_referenced() {
    let old = "{ pkgs, ... }:\nlet\n  version = \"1.0\";\nin\n{\n  name = \"demo\";\n}\n";
    let new = "{ pkgs, ... }:\nlet\n  version = \"2.0\";\nin\n{\n  name = \"demo\";\n  rev = version;\n}\n";
    let out = run("default.nix", old, new);
    assert!(
        out.edges.iter().any(|e| e.why.contains("version")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_dotted_attrpath_is_one_name() {
    let hs = one(
        "default.nix",
        "{\n  meta.description = \"a\";\n}\n",
        "{\n  meta.description = \"b\";\n}\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.enclosing.as_deref() == Some("meta.description")),
        "{hs:?}"
    );
}

#[test]
fn a_changed_attribute_has_no_signature() {
    let hs = one(
        "default.nix",
        "{\n  name = \"a\";\n}\n",
        "{\n  name = \"b\";\n}\n",
    );
    assert!(hs.iter().any(|h| h.rationale == "changes name"), "{hs:?}");
}

#[test]
fn lambda_formals_are_bound_not_used() {
    // `{ pkgs, lib, ... }:` binds both — neither is a reference to something
    // defined elsewhere
    let hs = one(
        "default.nix",
        "{ pkgs, lib, ... }:\n{\n  name = \"a\";\n}\n",
        "{ pkgs, lib, ... }:\n{\n  name = \"b\";\n}\n",
    );
    assert!(
        !hs.iter()
            .any(|h| h.uses.contains(&"pkgs".to_string()) || h.uses.contains(&"lib".to_string())),
        "{hs:?}"
    );
}

#[test]
fn an_imported_path_is_bound_to_the_name_that_holds_it() {
    // nix almost always binds an import (`overlay = import ./x.nix;`), and a
    // hunk that adds a real definition is a definition hunk, not an import
    // one — so the binding's name leads. The import rows are still recorded,
    // which is what an `imports` glob in a rule matches on.
    let hs = one(
        "default.nix",
        "let\n  a = 1;\nin a\n",
        "let\n  a = 1;\n  overlay = import ./overlays.nix;\nin a\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.defines.contains(&"overlay".to_string())),
        "{hs:?}"
    );
}
