//! End-to-end CLI: `ordo-engine review [--full-context]` over a patch on stdin.
use std::io::Write;
use std::process::{Command, Stdio};

fn review(args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ordo-engine"))
        .arg("review")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
}

const PATCH: &str = "--- a/m.py\n+++ b/m.py\n@@ -1,4 +1,4 @@\n def caller():\n     return helper()\n def helper():\n-    return 1\n+    return 2\n";

#[test]
fn review_full_context_gets_semantics() {
    let out = review(&["--full-context"], PATCH);
    assert!(
        out.contains("\"enclosing\": \"helper\""),
        "full-context review → full semantics:\n{out}"
    );
}

#[test]
fn review_bare_patch_is_degraded() {
    let out = review(&[], PATCH);
    assert!(
        out.contains("\"degraded\": true"),
        "bare modified-file patch → degraded:\n{out}"
    );
}

fn run(args: &[&str], stdin: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ordo-engine"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8(out.stdout).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn no_arguments_runs_the_default_command() {
    // `order` is the documented default; slicing the arg vector for its flags
    // used to panic before the vector was known to be that long
    let (out, code) = run(&[], r#"{"changes":[]}"#);
    assert_eq!(code, 0, "bare invocation must not fail:\n{out}");
    assert!(out.contains("\"schema\""), "default is order:\n{out}");
}

#[test]
fn a_mistyped_flag_is_refused_not_ignored() {
    // silently ordering the whole diff when the caller asked for the comment
    // subset is worse than refusing
    let (_, code) = run(&["order", "--only-comment"], r#"{"changes":[]}"#);
    assert_eq!(code, 2, "typo'd --only-comments must exit 2");
    let (_, code) = run(&["review", "--ful-context"], PATCH);
    assert_eq!(code, 2, "typo'd --full-context must exit 2");
}

#[test]
fn documented_flags_are_accepted() {
    for args in [&["order", "--json"][..], &["order", "--only-comments"][..]] {
        let (out, code) = run(args, r#"{"changes":[]}"#);
        assert_eq!(code, 0, "{args:?} must be accepted:\n{out}");
    }
}
