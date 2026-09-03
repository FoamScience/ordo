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
