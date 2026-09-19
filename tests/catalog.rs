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

fn python(src: &str, options: serde_json::Value) -> Vec<String> {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "", "new": src }],
        "options": options,
    }))
    .expect("fixture parses");
    let mut n: Vec<String> = ordo::run(inp)
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.findings.iter())
        .filter(|f| f.source == FindingSource::Catalog)
        .map(|f| f.name.clone())
        .collect();
    n.sort();
    n.dedup();
    n
}

const HARDCODED: &str = "import time\n\
                         def poll(retries):\n\
                         \x20   if retries > 86400:\n\
                         \x20       return None\n\
                         \x20   time.sleep(300)\n\
                         \x20   return retries\n";

/// The catalog is what a reviewer gets without configuring anything, and it has
/// to be refusable — by the run, by the rules file, or one entry at a time.
#[test]
fn the_catalog_can_be_turned_off_whole_or_by_name() {
    assert_eq!(
        python(HARDCODED, serde_json::json!({})),
        vec!["magic-argument", "magic-number"],
        "on by default"
    );
    assert!(
        python(HARDCODED, serde_json::json!({ "catalog": false })).is_empty(),
        "catalog = false silences all of it"
    );
    assert!(
        python(HARDCODED, serde_json::json!({ "disable": ["magic-*"] })).is_empty(),
        "a glob reaches catalog entries, not only the caller's own rules"
    );
    assert_eq!(
        python(
            HARDCODED,
            serde_json::json!({ "disable": ["magic-number"] })
        ),
        vec!["magic-argument"],
        "and silences exactly the one named"
    );
}

/// Turning the catalog off must not take the caller's own rules with it: they
/// are the reason someone would turn it off.
#[test]
fn the_callers_own_rules_survive_the_catalog_being_off() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "", "new": HARDCODED }],
        "options": {
            "catalog": false,
            "rules": [{ "name": "mine", "when": { "lang": "python" }, "note": "still here" }]
        }
    }))
    .unwrap();
    let names: Vec<(FindingSource, String)> = ordo::run(inp)
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.findings.iter())
        .map(|f| (f.source, f.name.clone()))
        .collect();
    assert_eq!(names, vec![(FindingSource::Rule, "mine".to_string())]);
}

/// The numbers everyone writes are not magic, and a number already sitting in a
/// named constant needs no naming.
#[test]
fn the_number_rules_leave_idiomatic_code_alone() {
    let clean = "TIMEOUT_S = 30\n\
                 PAGE = 4096\n\
                 def f(buf, n):\n\
                 \x20   if n > 1:\n\
                 \x20       return buf[0], buf[-1]\n\
                 \x20   return range(0, len(buf), 2)\n";
    assert!(
        python(clean, serde_json::json!({})).is_empty(),
        "named constants and 0/1/2/-1 are not findings: {:?}",
        python(clean, serde_json::json!({}))
    );
}

/// The string rules carry no `lang`, so they must work in every grammar — and
/// a grammar names its string node whatever it likes. `kind = "string"` alone
/// silently matched nothing in rust, go, c, c++ and java.
#[test]
fn the_string_rules_reach_every_grammar() {
    let cases = [
        ("a.py", "X = \"/usr/lib/z\"\n"),
        ("a.rs", "const X: &str = \"/usr/lib/z\";\n"),
        ("a.go", "package m\nvar X = \"/usr/lib/z\"\n"),
        ("a.js", "const X = \"/usr/lib/z\"\n"),
        ("a.ts", "const X: string = \"/usr/lib/z\"\n"),
        ("a.c", "char *x = \"/usr/lib/z\";\n"),
        ("a.cpp", "const char *x = \"/usr/lib/z\";\n"),
        ("a.java", "class A { String x = \"/usr/lib/z\"; }\n"),
        ("a.lua", "local x = \"/usr/lib/z\"\n"),
    ];
    for (path, src) in cases {
        let inp: Input = serde_json::from_value(serde_json::json!({
            "changes": [{ "path": path, "old": "", "new": src }]
        }))
        .unwrap();
        let hit = ordo::run(inp)
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .flat_map(|h| h.findings.iter())
            .any(|f| f.name == "absolute-path");
        assert!(hit, "{path}: the absolute path went unreported");
    }
}

/// A `disable` nobody can parse must say so. Dropping it silently is the
/// difference between "that rule is off" and "you typed it wrong".
#[test]
fn a_malformed_disable_glob_is_reported_not_swallowed() {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "", "new": HARDCODED }],
        "options": { "disable": ["["] }
    }))
    .unwrap();
    let out = ordo::run(inp);
    assert!(
        out.problems.iter().any(|p| p.contains("is not a glob")),
        "a broken disable must be reported: {:?}",
        out.problems
    );
    // and the catalog still runs: one typo does not silence everything
    assert!(out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .any(|h| !h.findings.is_empty()));
}
