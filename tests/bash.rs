//! bash: a command *is* a call, so `deploy prod` uses the function `deploy`,
//! and `source x.sh` is an import spelled as a command rather than a keyword.
//! Command and function names are bare `word` nodes — a kind make also uses —
//! so they are read explicitly rather than through IDENT_KINDS.
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
fn a_function_links_to_the_command_that_calls_it() {
    let old = "main() {\n  echo start\n}\n\nmain\n";
    let new = "deploy() {\n  echo deploying\n}\n\nmain() {\n  deploy prod\n}\n\nmain\n";
    let out = run("deploy.sh", old, new);
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    assert!(
        hs.iter().any(|h| h.defines.contains(&"deploy".to_string())),
        "{hs:?}"
    );
    assert!(
        out.edges.iter().any(|e| e.why.contains("deploy")),
        "{:?}",
        out.edges
    );
}

#[test]
fn both_function_spellings_define() {
    for body in [
        "rollback() {\n  echo x\n}\n",
        "function rollback {\n  echo x\n}\n",
    ] {
        let hs = one("a.sh", "echo hi\n", &format!("echo hi\n\n{body}"));
        assert!(
            hs.iter()
                .any(|h| h.defines.contains(&"rollback".to_string())),
            "{body}: {hs:?}"
        );
    }
}

#[test]
fn source_is_an_import_naming_the_script() {
    let hs = one(
        "deploy.sh",
        "set -eu\n\nmain\n",
        "set -eu\nsource ./lib/common.sh\n\nmain\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.rationale == "adds import ./lib/common.sh"),
        "{hs:?}"
    );
}

#[test]
fn a_positional_parameter_is_not_a_symbol() {
    // `$1` can never resolve to a definition
    let hs = one("a.sh", "f() {\n  echo hi\n}\n", "f() {\n  echo \"$1\"\n}\n");
    assert!(
        !hs.iter().any(|h| h.uses.contains(&"1".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_dotenv_file_is_read_as_shell() {
    // `.env` carries no extension, and `.env.local` resolves through the
    // variant strip
    for path in [".env", ".env.local"] {
        let inp: Input = serde_json::from_value(serde_json::json!({
            "changes": [{ "path": path, "old": "PORT=80\n", "new": "PORT=8080\n" }]
        }))
        .unwrap();
        assert!(!ordo::run(inp).files[0].unsupported, "{path}");
    }
}
