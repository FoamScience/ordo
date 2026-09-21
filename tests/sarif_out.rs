//! `ordo-engine --sarif` renders the findings as SARIF 2.1.0, so ordo's
//! advisories reach whatever already reads analyzer output. Findings only:
//! SARIF describes results at locations and has no vocabulary for a reading
//! order or a def→use graph.
mod fixture;
use fixture::run_json;

fn sarif(v: serde_json::Value) -> serde_json::Value {
    let out = run_json(v);
    serde_json::from_str(&ordo::sarif(&out)).expect("valid json")
}

fn run(results: &serde_json::Value) -> &serde_json::Value {
    &results["runs"][0]
}

const EVAL: &str = "def f(a):\n    x = eval(\"1\")\n    return x\n";

#[test]
fn a_finding_becomes_a_result_located_at_its_hunk() {
    // two files, the same catalog rule firing in both: the rule is declared
    // once and reported twice
    let doc = sarif(serde_json::json!({
        "changes": [
            { "path": "a.py", "old": "def f(a):\n    return a\n", "new": EVAL },
            { "path": "b.py", "old": "def g(a):\n    return a\n", "new": EVAL }
        ]
    }));
    assert_eq!(doc["version"], "2.1.0");
    let r = run(&doc);
    assert_eq!(r["tool"]["driver"]["name"], "ordo");

    let results = r["results"].as_array().expect("results");
    assert_eq!(results.len(), 2, "one per finding: {doc}");
    let hit = &results[0];
    assert_eq!(hit["ruleId"], "eval/exec");
    let loc = &hit["locations"][0]["physicalLocation"];
    assert_eq!(loc["artifactLocation"]["uri"], "a.py");
    // the region is the hunk, which is the resolution ordo works at — not a
    // line the engine never identified
    assert!(loc["region"]["startLine"].as_u64().unwrap() >= 1);
    assert!(
        loc["region"]["endLine"].as_u64() >= loc["region"]["startLine"].as_u64(),
        "{loc}"
    );

    // every result's rule is declared once, so a consumer can show what fired
    let rules = r["tool"]["driver"]["rules"].as_array().expect("rules");
    let ids: Vec<&str> = rules.iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"eval/exec"), "{ids:?}");
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "a rule is declared once: {ids:?}");
}

#[test]
fn a_change_with_nothing_to_say_is_an_empty_run_not_an_error() {
    let doc = sarif(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "x = 1\n", "new": "x = 2\n" }]
    }));
    let r = run(&doc);
    assert_eq!(r["results"].as_array().map(Vec::len), Some(0), "{doc}");
    assert_eq!(
        r["tool"]["driver"]["rules"].as_array().map(Vec::len),
        Some(0)
    );
}

/// A reviewer's own rule reaches the dashboard beside the catalog's findings,
/// and each level maps onto the one SARIF spells.
#[test]
fn a_rule_hit_carries_its_level_and_its_source() {
    let doc = sarif(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "", "new": "def f():\n    return 1\n" }],
        "options": { "rules": [
            { "name": "no-bare-def", "when": { "kind": "function_definition" },
              "verdict": "every function needs a decorator here" }
        ]}
    }));
    let results = run(&doc)["results"].as_array().expect("results").clone();
    let mine: Vec<&serde_json::Value> = results
        .iter()
        .filter(|r| r["ruleId"] == "no-bare-def")
        .collect();
    assert_eq!(mine.len(), 1, "{results:?}");
    // a verdict is an assertion, not an FYI — SARIF's "error"
    assert_eq!(mine[0]["level"], "error");
    assert_eq!(mine[0]["properties"]["source"], "rule");
}

/// A consumer matches an alert across commits on the fingerprint. It must
/// describe what the finding is about — rule, file, enclosing definition — and
/// not where it sits today, or an edit above it re-raises everything.
#[test]
fn a_fingerprint_survives_the_lines_moving() {
    let fp = |pad: &str| {
        let doc = sarif(serde_json::json!({
            "changes": [{ "path": "a.py", "old": "def f(a):\n    return a\n",
                          "new": format!("{pad}def f(a):\n    x = eval(\"1\")\n    return x\n") }]
        }));
        run(&doc)["results"][0]["partialFingerprints"]["ordo/v1"]
            .as_str()
            .expect("fingerprint")
            .to_string()
    };
    let here = fp("");
    let shifted = fp("# a comment pushed in above\n# and another\n");
    assert_eq!(here, shifted, "the finding did not move, its lines did");
    assert_eq!(here.len(), 16, "hex of a u64: {here}");
}

/// An empty `results` on a patch the engine could not analyse must not read as
/// a clean review. The degradation is reported in the run itself, not only on
/// stderr where a dashboard never sees it.
#[test]
fn a_degraded_file_is_reported_in_the_run() {
    let doc = sarif(serde_json::json!({
        "changes": [{ "path": "a.py",
                      "diff": "@@ -1,2 +1,2 @@\n def f(a):\n-    return a\n+    return a + 1\n" }]
    }));
    let notes = run(&doc)["invocations"][0]["toolExecutionNotifications"]
        .as_array()
        .expect("notifications");
    assert!(
        notes.iter().any(|n| n["message"]["text"]
            .as_str()
            .unwrap_or("")
            .contains("positional order only")),
        "{notes:?}"
    );
}

/// `shortDescription` is what a consumer puts in a rule index beside forty
/// others; the catalog's own message is a numbered remedy list.
#[test]
fn a_rule_descriptor_is_short_and_still_carries_the_whole_message() {
    let doc = sarif(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "def f(a):\n    return a\n", "new": EVAL }]
    }));
    let rule = &run(&doc)["tool"]["driver"]["rules"][0];
    let short = rule["shortDescription"]["text"].as_str().expect("short");
    let full = rule["fullDescription"]["text"].as_str().expect("full");
    assert!(!short.contains('\n'), "one sentence, one line: {short:?}");
    assert!(short.len() < full.len(), "{short:?} vs {full:?}");
    assert!(
        full.contains("literal_eval"),
        "the remedy survives: {full:?}"
    );
}
