//! Every shipped ruleset, run against its sample through the engine: no
//! `problems`, and every rule fires at least once. A rule that never fires
//! looks exactly like a convention nobody breaks — this is what keeps the
//! shipped sets honest as grammars and the engine move.
use ordo::model::{Input, Output, Rule, When};
use std::path::Path;

/// The rules files are kebab-case and flat; the engine's `Rule` nests its
/// conditions under `when` in snake_case. Same conversion the `ordo` reviewer does.
fn load(path: &Path) -> Vec<Rule> {
    let text = std::fs::read_to_string(path).unwrap();
    let doc: toml::Value =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    doc["rule"]
        .as_array()
        .unwrap_or_else(|| panic!("{}: no [[rule]]", path.display()))
        .iter()
        .map(|r| {
            let t = r.as_table().unwrap();
            let mut when = toml::map::Map::new();
            let mut rule = toml::map::Map::new();
            for (k, v) in t {
                let key = k.replace('-', "_");
                match key.as_str() {
                    "name" | "note" | "warn" | "noise" | "priority" => {
                        rule.insert(key, v.clone());
                    }
                    "noise_when" => {
                        when.insert("noise".into(), v.clone());
                    }
                    "query_file" => {
                        let q = std::fs::read_to_string(
                            path.parent().unwrap().join(v.as_str().unwrap()),
                        )
                        .unwrap();
                        when.insert("query".into(), toml::Value::String(q));
                    }
                    _ => {
                        when.insert(key, v.clone());
                    }
                }
            }
            let when: When = toml::Value::Table(when)
                .try_into()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut rule: Rule = toml::Value::Table(rule)
                .try_into()
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            rule.when = when;
            rule
        })
        .collect()
}

fn sample_for(set: &Path) -> Option<std::path::PathBuf> {
    let stem = set.file_stem()?.to_str()?.to_string();
    std::fs::read_dir(set.parent()?.join("samples"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(&stem))
}

#[test]
fn every_shipped_ruleset_loads_and_every_rule_fires_on_its_sample() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("rulesets");
    let mut sets: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    sets.sort();
    assert!(!sets.is_empty(), "no rulesets in {}", dir.display());
    for set in sets {
        let sample = sample_for(&set)
            .unwrap_or_else(|| panic!("{}: no sample under rulesets/samples/", set.display()));
        let rules = load(&set);
        let input = Input {
            changes: vec![ordo::model::Change {
                path: sample.file_name().unwrap().to_string_lossy().into_owned(),
                old: Some(String::new()),
                new: Some(std::fs::read_to_string(&sample).unwrap()),
                diff: None,
            }],
            options: ordo::model::Options {
                rules: rules.clone(),
                ..Default::default()
            },
        };
        let out: Output = ordo::run(input);
        assert!(
            out.problems.is_empty(),
            "{}: {:?}",
            set.display(),
            out.problems
        );
        let fired: std::collections::HashSet<&str> = out
            .files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .flat_map(|h| h.rules.iter())
            .map(|r| r.rule.as_str())
            .collect();
        // a file-size limit cannot be exercised by a sample short enough to read
        let silent: Vec<&str> = rules
            .iter()
            .filter(|r| r.when.max_file_lines.is_none())
            .map(|r| r.name.as_str())
            .filter(|n| !fired.contains(n))
            .collect();
        assert!(
            silent.is_empty(),
            "{}: never fired on its sample: {silent:?}",
            set.display()
        );
    }
}
