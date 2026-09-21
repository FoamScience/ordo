//! bash: a command *is* a call, so `deploy prod` uses the function `deploy`,
//! and `source x.sh` is an import spelled as a command rather than a keyword.
//! Command and function names are bare `word` nodes — a kind make also uses —
//! so they are read explicitly rather than through IDENT_KINDS.
mod fixture;
use fixture::{hunks as one, run_file as run, run_json};

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
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: deploy"]);
}

#[test]
fn both_function_spellings_define() {
    for body in [
        "rollback() {\n  echo x\n}\n",
        "function rollback {\n  echo x\n}\n",
    ] {
        let hs = one("a.sh", "echo hi\n", &format!("echo hi\n\n{body}"));
        assert_eq!(hs.len(), 1, "{body}: {hs:?}");
        assert_eq!(hs[0].defines, vec!["rollback".to_string()]);
    }
}

#[test]
fn source_is_an_import_naming_the_script() {
    let hs = one(
        "deploy.sh",
        "set -eu\n\nmain\n",
        "set -eu\nsource ./lib/common.sh\n\nmain\n",
    );
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(rats, vec!["adds import ./lib/common.sh"]);
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
        let out = run_json(serde_json::json!({
            "changes": [{ "path": path, "old": "PORT=80\n", "new": "PORT=8080\n" }]
        }));
        assert!(!out.files[0].unsupported, "{path}");
    }
}

#[test]
fn a_zsh_script_has_a_grammar() {
    // `zsh` resolved from a markdown fence but not from a path
    let out = run_json(serde_json::json!({
        "changes": [{ "path": "s.zsh",
            "old": "greet() {\n  echo hi\n}\n",
            "new": "greet() {\n  echo bye\n}\n" }]
    }));
    assert!(!out.files[0].unsupported, "{:?}", out.files[0]);
}
