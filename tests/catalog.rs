//! Every catalog rule fires on the sample written for it.
//!
//! The catalog is data now (`rulesets/catalog/*.toml`, compiled into the
//! engine by `src/catalog.rs`). Data goes stale silently: a query that stops
//! compiling when a grammar is bumped, or a `kind` the grammar renamed, looks
//! exactly like a construct nobody writes. `rulesets/*.toml` is held to this
//! standard by `scripts/ruleset-check.py`; the catalog is held to it here,
//! inside the normal test run, because the catalog is always on and so a dead
//! rule silently stops flagging code for everyone.
use ordo::model::{FindingSource, Input};
use std::collections::BTreeSet;
use std::path::Path;

/// Run one sample as a wholly-new file and collect the catalog names that fired.
fn fired(path: &Path) -> BTreeSet<String> {
    let new = std::fs::read_to_string(path).expect("read sample");
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": name, "old": "", "new": new }]
    }))
    .expect("fixture parses");
    let out = ordo::run(inp);
    assert!(out.problems.is_empty(), "{}: {:?}", name, out.problems);
    assert!(
        ordo::catalog::problem().is_none(),
        "{:?}",
        ordo::catalog::problem()
    );
    out.files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.findings.iter())
        .filter(|f| f.source == FindingSource::Catalog)
        .map(|f| f.name.clone())
        .collect()
}

#[test]
fn every_catalog_rule_fires_on_a_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("rulesets/catalog/samples");
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut samples: Vec<_> = std::fs::read_dir(&root)
        .expect("rulesets/catalog/samples")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    samples.sort();
    assert!(!samples.is_empty(), "no samples");
    for s in &samples {
        seen.extend(fired(s));
    }
    // the walkers in `src/advisories.rs` emit catalog findings too, and a
    // sample may trip one in passing — only the rules have to be covered
    let want: BTreeSet<String> = ordo::catalog::rules()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let dead: Vec<&String> = want.difference(&seen).collect();
    assert!(
        dead.is_empty(),
        "catalog rules that fired on no sample: {dead:?}\n\
         Add the construct to the sample for its language, or delete the rule."
    );
}

/// A catalog finding must not lend its name to one of the caller's rules.
///
/// `any_noise` and `priority` resolve a finding back to a rule by name, and the
/// catalog now shares that list — so a rule named after a construct used to
/// reclassify every hunk the catalog's rule of that name fired on.
#[test]
fn a_user_rule_named_after_a_catalog_construct_does_not_inherit_its_hits() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.c", "old": "", "new":
            "int f(int n) {\n    if (n) goto done;\n    n = 1;\ndone:\n    return n;\n}\n" }],
        // same name as the catalog's C rule, but matching nothing here
        "options": { "rules": [
            { "name": "goto", "when": { "path": "vendor/**" }, "noise": true, "priority": 9 }
        ]}
    }))
    .unwrap();
    let out = ordo::run(inp);
    let h = &out.files[0].hunks[0];
    assert!(
        h.findings
            .iter()
            .any(|f| f.name == "goto" && f.source == FindingSource::Catalog),
        "the catalog still fires: {:?}",
        h.findings
    );
    assert!(
        !h.noise,
        "a non-matching user rule must not mark it skippable"
    );
}
