//! Xonsh: a python superset with shell syntax. The grammar shares python's
//! node kinds, so what needs pinning is that ordo *reaches* them — the spec
//! resolves by extension, imports bind the names python binds, `#` is a
//! comment, and `$FOO = …` names a binding python's `assignment` never sees.
use ordo::model::{Category, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Output {
    run(serde_json::json!({ "changes": [{ "path": path, "old": old, "new": new }] }))
}

fn rationales(out: &Output) -> Vec<String> {
    out.files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.rationale.clone()))
        .collect()
}

#[test]
fn every_xonsh_extension_resolves_to_the_grammar() {
    // `lang` is private, so the spec is observed through its effect: an
    // unparsed file has no container and would say "adds 2 lines"
    for path in [
        "tools/deploy.xsh",
        "tools/deploy.xonsh",
        ".xonshrc",
        "xonshrc",
    ] {
        let out = one(path, "x = 1\n", "x = 1\n\ndef deploy(t):\n    return t\n");
        assert_eq!(rationales(&out), vec!["adds deploy"], "{path}");
    }
}

#[test]
fn a_definition_is_named() {
    let out = one(
        "d.xsh",
        "x = 1\n",
        "x = 1\n\ndef deploy(target):\n    return target\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.category, Category::Definition);
    assert_eq!(h.rationale, "adds deploy");
}

#[test]
fn an_import_binds_the_name_it_actually_introduces() {
    // the python path: `from a.b import c` binds `c`, not the module path
    let out = one(
        "d.xsh",
        "import os\n",
        "import os\nfrom ppump.diagnostics import degrade\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.category, Category::Import);
    assert_eq!(h.rationale, "adds import degrade");
}

#[test]
fn an_env_assignment_is_a_named_binding() {
    // `$FOO = …` is xonsh's own node kind; python's `assignment` does not
    // cover it, so without the `env_assignment` entry this reads as a line count
    let out = one(
        "d.xsh",
        "def run(p):\n    return p\n",
        "$PROJECT_ROOT = '/srv/ppump'\n\ndef run(p):\n    return p\n",
    );
    assert_eq!(
        rationales(&out),
        vec!["adds PROJECT_ROOT, no uses in this file — check other files"]
    );
}

#[test]
fn an_env_binding_links_to_its_use_in_a_subprocess_line() {
    let out = one(
        "d.xsh",
        "def sync(target):\n    return target\n",
        "$PROJECT_ROOT = '/srv/ppump'\n\ndef sync(target):\n    ![rsync -a @(target) $PROJECT_ROOT]\n    return target\n",
    );
    assert!(
        rationales(&out)
            .iter()
            .any(|r| r.contains("adds PROJECT_ROOT, used at")),
        "{:?}",
        rationales(&out)
    );
}

#[test]
fn a_hash_comment_is_a_comment_not_code() {
    let out = one(
        "d.xsh",
        "def run(p):\n    return p\n",
        "def run(p):\n    # the old path is kept for one release\n    return p\n",
    );
    assert_eq!(rationales(&out), vec!["adds comment to run"]);
}

#[test]
fn a_python_advisory_still_fires_on_xonsh() {
    let out = one(
        "d.xsh",
        "import subprocess\n",
        "import subprocess\n\ndef go(cmd):\n    subprocess.run(cmd, shell=True)\n",
    );
    let found: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .flat_map(|h| h.advisories.iter().map(|a| a.construct.as_str()))
        })
        .collect();
    assert!(found.contains(&"shell-injection"), "{found:?}");
}

#[test]
fn a_command_line_at_file_scope_names_itself() {
    // a script's real work is bare command lines that belong to no definition;
    // the command and its subcommand words are the container
    let out = one(
        "b.xsh",
        "pip install --upgrade pip\npip cache remove '*cp311-linux*' || true\n",
        "pip install --upgrade pip\npip cache remove '*cp312-linux*' || true\n",
    );
    assert_eq!(rationales(&out), vec!["edits pip cache remove"]);
}

#[test]
fn a_flag_does_not_join_the_command_name() {
    let out = one("b.xsh", "make -j\n", "make -j$(nproc)\n");
    assert_eq!(rationales(&out), vec!["edits make"]);
}

#[test]
fn a_command_inside_a_with_block_still_names_the_command() {
    // the innermost container wins: `make` is more use to a reviewer than the
    // `with` block that holds it
    let out = one(
        "b.xsh",
        "with with_pushd(wd):\n    make -j\n    make install\n",
        "with with_pushd(wd):\n    make -j$(nproc)\n    make install\n",
    );
    assert_eq!(rationales(&out), vec!["edits make"]);
}
