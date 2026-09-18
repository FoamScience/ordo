//! Fixture helpers shared by the integration tests.
//!
//! Lives under `tests/fixture/` rather than `tests/` so cargo does not build it
//! as an integration-test target of its own — the same reason `tests/common/`
//! is a directory. Each test file pulls in what it needs and may alias it to
//! whatever name reads best there:
//!
//! ```ignore
//! mod fixture;
//! use fixture::{hunks as one, run_file as run};
//! ```
//!
//! These were written out again in roughly eighteen files, in five spellings
//! that all meant the same thing.
#![allow(dead_code)]
use ordo::model::{HunkOut, Input, Output};

/// One changed file, as the engine takes it.
pub fn input(path: &str, old: &str, new: &str) -> Input {
    serde_json::from_value(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .expect("fixture input parses")
}

/// Run a hand-written `Input` json value.
pub fn run_json(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).expect("fixture input parses"))
}

/// Run a single changed file.
pub fn run_file(path: &str, old: &str, new: &str) -> Output {
    ordo::run(input(path, old, new))
}

/// Every hunk of a single changed file, in engine order.
pub fn hunks(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    run_file(path, old, new)
        .files
        .into_iter()
        .flat_map(|f| f.hunks)
        .collect()
}

/// Every hunk's rationale, for a finished output.
pub fn rationales_of(out: &Output) -> Vec<String> {
    out.files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.rationale.clone()))
        .collect()
}

/// Every hunk's rationale, from a hand-written `Input` json value.
pub fn rationales_json(v: serde_json::Value) -> Vec<String> {
    rationales_of(&run_json(v))
}
