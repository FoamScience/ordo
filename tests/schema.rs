//! `schema/v1.json` against the code it describes.
//!
//! The schema is the v1 promise, and nothing used to read it: it had drifted in
//! both directions at once — `hunks[].rules` was emitted but undeclared, while
//! `when.rules` was declared carrying the description of that very output field.
//! Both directions are checked here, so the next drift fails a test rather than
//! reaching a consumer.
use ordo::model::{Input, When};
use serde_json::Value;

fn schema() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/v1.json");
    serde_json::from_str(&std::fs::read_to_string(&p).expect("schema/v1.json")).expect("valid json")
}

/// Declared property names at a `/`-separated path, stepping through `items`
/// automatically so `files/hunks` means "the hunk objects inside files".
fn declared(schema: &Value, path: &str) -> Vec<String> {
    let mut node = schema
        .get("$defs")
        .and_then(|d| d.get("output"))
        .expect("$defs.output");
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        node = node
            .get("properties")
            .and_then(|p| p.get(seg))
            .unwrap_or_else(|| panic!("schema has no {seg} under {path}"));
        if let Some(items) = node.get("items") {
            node = items;
        }
    }
    node.get("properties")
        .and_then(|p| p.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// An input that reaches as many optional output fields as one fixture can: a
/// rename whose old name survives at one call site (a `notes` entry), a
/// cross-file use, a removal, a rule hit, and a config edit whose changed keys
/// produce `details`.
fn rich() -> Input {
    serde_json::from_value(serde_json::json!({
        "changes": [
            { "path": "a.py",
              "old": "import os\n\ndef parse_cfg(p):\n    # note\n    return os.path.join(p)\n",
              "new": "import os\nimport sys\n\ndef load_cfg(p):\n    # changed note\n    return os.path.join(p)\n" },
            { "path": "b.py",
              "old": "from a import parse_cfg\n\ndef main():\n    return parse_cfg('x')\n",
              "new": "from a import load_cfg\n\ndef main():\n    return load_cfg('x')\n\ndef legacy():\n    return parse_cfg('y')\n" },
            { "path": "c.yaml",
              "old": "svc:\n  port: 80\n  host: a\n",
              "new": "svc:\n  port: 443\n  host: a\n  tls: on\n" }
        ],
        "options": { "rules": [
            { "name": "py-note", "when": { "lang": "python" }, "note": "python touched" }
        ]}
    }))
    .expect("fixture parses")
}

/// `rich` cannot reach `advisories`: it contains no flagged construct.
fn advisory() -> Input {
    serde_json::from_value(serde_json::json!({
        "changes": [
            { "path": "d.py",
              "old": "def risky(s):\n    return 1\n",
              "new": "def risky(s):\n    return eval(s)\n" }
        ]
    }))
    .expect("fixture parses")
}

/// `rich` cannot reach `dropped` either: that needs `only_comments`, which
/// would throw away the hunks the other assertions rely on — including,
/// in the fixture above, the flagged construct itself.
fn dropped() -> Input {
    serde_json::from_value(serde_json::json!({
        "changes": [
            { "path": "e.py",
              "old": "x = 1\n",
              "new": "# why\nx = 2\n" }
        ],
        "options": { "only_comments": true }
    }))
    .expect("fixture parses")
}

/// Every key the engine emits is a key the schema declares.
#[test]
fn the_schema_declares_everything_the_engine_emits() {
    let sch = schema();

    let mut checked = 0;
    let check = |path: &str, obj: &Value| {
        let known = declared(&sch, path);
        assert!(!known.is_empty(), "schema declares no properties at {path}");
        for key in obj.as_object().expect("object").keys() {
            assert!(
                known.contains(key),
                "engine emits `{key}` at {path}, schema declares only {known:?}"
            );
        }
    };

    for out in [
        serde_json::to_value(ordo::run(rich())).expect("output serializes"),
        serde_json::to_value(ordo::run(advisory())).expect("output serializes"),
        serde_json::to_value(ordo::run(dropped())).expect("output serializes"),
    ] {
        check("", &out);
        for f in out["files"].as_array().expect("files") {
            check("files", f);
            for h in f["hunks"].as_array().expect("hunks") {
                check("files/hunks", h);
                checked += 1;
                for s in h["symbols"].as_array().into_iter().flatten() {
                    check("files/hunks/symbols", s);
                }
                for r in h["rules"].as_array().into_iter().flatten() {
                    check("files/hunks/rules", r);
                }
                for a in h["advisories"].as_array().into_iter().flatten() {
                    check("files/hunks/advisories", a);
                }
            }
            for d in f["dropped"].as_array().into_iter().flatten() {
                check("files/dropped", d);
            }
        }
        for e in out["ledger"].as_array().into_iter().flatten() {
            check("ledger", e);
        }
        for g in out["groups"].as_array().expect("groups") {
            check("groups", g);
        }
        for e in out["edges"].as_array().into_iter().flatten() {
            check("edges", e);
        }
        for o in out["order"].as_array().expect("order") {
            check("order", o);
        }
    }
    assert!(checked > 0, "fixture produced no hunks to check");
}

/// The fixture has to actually reach the fields this test exists to guard, or
/// it would pass by emitting nothing.
#[test]
fn the_fixture_reaches_the_optional_output_fields() {
    let out = serde_json::to_value(ordo::run(rich())).expect("output serializes");
    let hunks: Vec<&Value> = out["files"]
        .as_array()
        .expect("files")
        .iter()
        .flat_map(|f| f["hunks"].as_array().expect("hunks"))
        .collect();
    for field in ["rules", "symbols", "details", "notes"] {
        assert!(
            hunks.iter().any(|h| h.get(field).is_some()),
            "no hunk carries `{field}`; the schema check would not cover it"
        );
    }
    assert!(out.get("ledger").is_some(), "fixture produced no ledger");

    // the other two fixtures own the paths this one cannot reach; without them
    // the schema check walks those objects zero times and passes on nothing
    let adv = serde_json::to_value(ordo::run(advisory())).expect("serializes");
    assert!(
        adv["files"]
            .as_array()
            .expect("files")
            .iter()
            .any(|f| f["hunks"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(|h| h.get("advisories").is_some()))),
        "advisory fixture carries no advisory: {adv}"
    );
    let drp = serde_json::to_value(ordo::run(dropped())).expect("serializes");
    assert!(
        drp["files"]
            .as_array()
            .expect("files")
            .iter()
            .any(|f| f.get("dropped").is_some()),
        "dropped fixture drops no hunk: {drp}"
    );
}

/// Every condition the schema offers is one the engine actually reads. A
/// property here that `When` has no field for deserializes into its catch-all,
/// which is how `when.rules` survived: documented, accepted, and ignored.
#[test]
fn every_documented_rule_condition_is_one_the_engine_reads() {
    let sch = schema();
    let when = sch["$defs"]["input"]["properties"]["options"]["properties"]["rules"]["items"]
        ["properties"]["when"]["properties"]
        .as_object()
        .expect("when properties");

    for (key, spec) in when {
        // a plausible value of the declared type — enough to deserialize
        let value = match spec.get("enum").and_then(|e| e.as_array()) {
            // an enum names its own legal values; anything else goes by type
            Some(vs) => vs.first().expect("enum has a variant").clone(),
            None => match spec.get("type").and_then(|t| t.as_str()) {
                Some("array") => serde_json::json!([]),
                Some("boolean") => serde_json::json!(true),
                Some("integer") | Some("number") => serde_json::json!(1),
                _ => serde_json::json!("x"),
            },
        };
        let parsed: When = serde_json::from_value(serde_json::json!({ key: value }))
            .unwrap_or_else(|e| panic!("schema offers `when.{key}`, which does not parse: {e}"));
        assert!(
            parsed.unknown.is_empty(),
            "schema offers `when.{key}`, but `When` has no such field — it is \
             documented, accepted and then ignored"
        );
    }
}
