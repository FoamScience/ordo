use serde_json::Value;
use std::fmt::Write;

fn main() {
    go_template();
    hunch_model();
}

// Compiles the one grammar this crate vendors rather than takes as a
// dependency: Go templates (Helm), which no crate publishes for a current
// tree-sitter. See grammars/tree-sitter-go-template/README.md.
fn go_template() {
    let dir = std::path::Path::new("grammars/tree-sitter-go-template/src");
    println!("cargo:rerun-if-changed={}", dir.join("parser.c").display());
    cc::Build::new()
        .include(dir)
        .file(dir.join("parser.c"))
        // generated parsers trip these; upstream ships them the same way
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .compile("tree-sitter-go-template");
}

// hunch's trained model (scripts/hunch-train.py) as static arrays, so the
// engine never parses JSON for it. Trees become one node list with indices
// made absolute and a root per tree; a logistic model becomes its four
// vectors. See `crate::hunch::Model`.
fn hunch_model() {
    let path = "src/hunch.json";
    println!("cargo:rerun-if-changed={path}");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("src/hunch.json"))
        .expect("src/hunch.json is JSON");
    let names: Vec<&str> = doc["feature_schema"]["names"]
        .as_array()
        .expect("feature_schema.names")
        .iter()
        .map(|n| n.as_str().expect("a feature name"))
        .collect();
    let model = &doc["model"];
    let hash = doc["feature_schema"]["hash"]
        .as_str()
        .expect("feature_schema.hash");
    let mut out = format!(
        "static NAMES: [&str; {}] = {names:?};\nstatic SCHEMA_HASH: &str = {hash:?};\n",
        names.len()
    );
    match model["type"].as_str() {
        Some("lightgbm") => {
            assert_eq!(model["link"], "sigmoid", "hunch: only a sigmoid link");
            let (mut nodes, mut roots) = (String::new(), vec![]);
            let mut n = 0;
            for tree in model["trees"].as_array().expect("model.trees") {
                let tree = tree.as_array().expect("a tree is a node list");
                roots.push(n);
                for node in tree {
                    if let Some(v) = node.get("leaf_value") {
                        writeln!(nodes, "Node::Leaf({}),", f32_lit(v)).unwrap();
                        continue;
                    }
                    let feature = node["feature"].as_u64().expect("feature") as usize;
                    assert!(feature < names.len(), "hunch: split on feature {feature}");
                    let child = |k: &str| {
                        let c = node[k].as_u64().expect(k) as usize;
                        assert!(c < tree.len(), "hunch: {k} child {c} outside its tree");
                        n + c
                    };
                    writeln!(
                        nodes,
                        "Node::Split {{ feature: {feature}, threshold: {}, left: {}, right: {}, value: {} }},",
                        f32_lit(&node["threshold"]),
                        child("left"),
                        child("right"),
                        f32_lit(&node["internal_value"]),
                    )
                    .unwrap();
                }
                n += tree.len();
            }
            writeln!(
                out,
                "static NODES: [Node; {n}] = [\n{nodes}];\n\
                 static ROOTS: [u32; {}] = {roots:?};\n\
                 static MODEL: Model = Model::Trees {{ nodes: &NODES, roots: &ROOTS }};",
                roots.len()
            )
            .unwrap();
        }
        Some("logistic") => {
            let vector = |k: &str| {
                let v = model[k].as_array().expect(k);
                assert_eq!(v.len(), names.len(), "hunch: {k} has one value per feature");
                let items: Vec<String> = v.iter().map(f32_lit).collect();
                format!("[{}]", items.join(", "))
            };
            writeln!(
                out,
                "static MODEL: Model = Model::Logistic {{ mean: &{}, scale: &{}, coef: &{}, intercept: {} }};",
                vector("mean"),
                vector("scale"),
                vector("coef"),
                f32_lit(&model["intercept"]),
            )
            .unwrap();
        }
        other => panic!("hunch: unknown model type {other:?}"),
    }
    let dest =
        std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("hunch_model.rs");
    std::fs::write(dest, out).expect("write hunch_model.rs");
}

/// A JSON number as an f32 literal; `{:?}` prints the shortest form that
/// reads back to the same f32.
fn f32_lit(v: &Value) -> String {
    format!("{:?}f32", v.as_f64().expect("a number") as f32)
}
