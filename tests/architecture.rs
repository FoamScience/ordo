//! `docs/architecture.md` describes where things live. A map that rots is
//! worse than no map, and the version of it that lived in a bead had gone
//! stale within a fortnight — it cited line ranges in a file that grew.
//!
//! So the paths it names must exist, the client sections it points at must
//! still be there, and the counts it quotes must still be true.
use std::path::Path;

fn doc() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/architecture.md"))
        .expect("docs/architecture.md")
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `path/like/this` in a backticked span names something on disk.
#[test]
fn every_path_the_map_names_exists() {
    let doc = doc();
    let mut checked = 0;
    for span in doc.split('`').skip(1).step_by(2) {
        // a path, not prose or a code identifier: has a separator or a known
        // extension, and no spaces
        let looks_like_path = !span.contains(' ')
            && (span.contains('/') || span.ends_with(".rs") || span.ends_with(".md"))
            && !span.contains("::")
            && !span.contains('(');
        if !looks_like_path {
            continue;
        }
        let rel = span.trim_end_matches('/');
        // a glob names a directory that must hold at least one match
        if let Some((dir, pat)) = rel.rsplit_once('/').filter(|(_, p)| p.contains('*')) {
            let ext = pat.rsplit('.').next().unwrap_or("");
            let matched = std::fs::read_dir(root().join(dir))
                .unwrap_or_else(|_| {
                    panic!("docs/architecture.md names `{span}`, no such directory")
                })
                .filter_map(|e| e.ok())
                .any(|e| e.path().extension().is_some_and(|x| x == ext));
            assert!(
                matched,
                "docs/architecture.md names `{span}`, which matches nothing"
            );
        } else {
            assert!(
                root().join(rel).exists(),
                "docs/architecture.md names `{span}`, which does not exist"
            );
        }
        checked += 1;
    }
    assert!(checked > 10, "only {checked} paths checked; the scan broke");
}

/// The map says each client module file is one concern with a `// ---- name`
/// banner at its top. That only stays true while the banners are there.
#[test]
fn every_client_module_opens_with_its_banner() {
    let dir = root().join("src/bin/ordo");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("src/bin/ordo") {
        let path = entry.expect("entry").path();
        if path.file_name().is_some_and(|n| n == "main.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("module");
        let first = src.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        assert!(
            first.starts_with("// ----"),
            "{} does not open with a `// ---- name` banner: {first:?}",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked > 10,
        "only {checked} modules checked; the scan broke"
    );
}

/// The map quotes field counts for the three copies of the rule schema. They
/// are the argument for the drift tests existing, so they have to be right.
#[test]
fn the_rule_schema_counts_the_map_quotes_are_still_true() {
    let doc = doc();
    let model = std::fs::read_to_string(root().join("src/model.rs")).expect("model");
    let when = model
        .split("pub struct When {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("When body");
    let fields = when
        .lines()
        .filter(|l| l.trim_start().starts_with("pub "))
        .count();
    assert!(
        doc.contains(&format!("({fields} fields)")),
        "`When` has {fields} fields; docs/architecture.md says otherwise"
    );

    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root().join("schema/v2.json")).unwrap())
            .expect("schema parses");
    let declared = find_when(&schema)
        .and_then(|w| w.get("properties"))
        .and_then(|p| p.as_object())
        .map(|o| o.len())
        .expect("when properties");
    assert!(
        doc.contains(&format!("({declared} declared")),
        "schema/v2.json declares {declared} `when` properties; the map says otherwise"
    );
}

fn find_when(v: &serde_json::Value) -> Option<&serde_json::Value> {
    if let Some(o) = v.as_object() {
        if o.contains_key("warn") && o.contains_key("noise") && o.contains_key("when") {
            return o.get("when");
        }
        for v in o.values() {
            if let Some(r) = find_when(v) {
                return Some(r);
            }
        }
    } else if let Some(a) = v.as_array() {
        for v in a {
            if let Some(r) = find_when(v) {
                return Some(r);
            }
        }
    }
    None
}

/// The map claims `lib.rs` builds no rationale prose — that the duplication
/// with `order.rs` is closed. If a phrase creeps back the claim is a lie.
#[test]
fn the_pipeline_core_still_writes_no_rationale_prose() {
    let lib = std::fs::read_to_string(root().join("src/lib.rs")).expect("lib");
    let offenders: Vec<&str> = lib
        .lines()
        .filter(|l| {
            ["adds ", "changes ", "edits ", "removes ", "moves "]
                .iter()
                .any(|p| l.contains(&format!("format!(\"{p}")))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "rationale prose is back in lib.rs; it belongs in order.rs: {offenders:?}"
    );
}
