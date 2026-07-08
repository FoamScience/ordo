//! Golden-file snapshot harness. Each `tests/golden/<name>/` has an
//! `input.json`; its `expected.json` is the pretty-printed engine output.
//! Regenerate after intentional changes with `UPDATE_GOLDEN=1 cargo test`.
use std::fs;
use std::path::Path;

fn run_case(dir: &Path) {
    let input = fs::read_to_string(dir.join("input.json"))
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    let inp: ordo::model::Input = serde_json::from_str(&input).expect("parse input.json");
    let got = serde_json::to_string_pretty(&ordo::run(inp)).unwrap();
    let exp_path = dir.join("expected.json");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&exp_path, format!("{got}\n")).unwrap();
        return;
    }
    let exp = fs::read_to_string(&exp_path)
        .unwrap_or_else(|_| panic!("missing {}; run UPDATE_GOLDEN=1", exp_path.display()));
    assert_eq!(got.trim(), exp.trim(), "golden mismatch: {}", dir.display());
}

#[test]
fn golden() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut cases: Vec<_> = fs::read_dir(&root)
        .expect("tests/golden dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no golden cases");
    for c in &cases {
        run_case(c);
    }
}
