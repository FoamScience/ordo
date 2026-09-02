//! Language injection: a fenced code block in a prose file is parsed with the
//! fence's own grammar, not left as opaque text.
use ordo::model::{Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn md(old: &str, new: &str) -> Output {
    run(serde_json::json!({ "changes": [{ "path": "README.md", "old": old, "new": new }] }))
}

#[test]
fn a_code_fence_links_the_docs_to_the_definition_they_document() {
    let out = run(serde_json::json!({ "changes": [
        { "path": "config.py",
          "old": "def parse_cfg(p):\n    return 1\n",
          "new": "def parse_cfg(p, strict=False):\n    return 1\n" },
        { "path": "README.md",
          "old": "# Usage\n\n```python\nx = old_helper(1)\n```\n",
          "new": "# Usage\n\n```python\nx = parse_cfg(\"a.toml\", strict=True)\n```\n" },
    ]}));
    let doc = out.files.iter().find(|f| f.path == "README.md").unwrap();
    assert!(
        doc.hunks[0].uses.contains(&"parse_cfg".to_string()),
        "{:?}",
        doc.hunks[0].uses
    );
    assert_eq!(
        doc.hunks[0].rationale,
        "uses parse_cfg, defined in config.py"
    );
    assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
}

#[test]
fn a_sample_never_defines_anything() {
    // a ```python block demonstrating a function must not claim to define it:
    // docs illustrate an API, they do not provide it
    let out = md(
        "# Guide\n\n```python\nprint(1)\n```\n",
        "# Guide\n\n```python\ndef demo(x):\n    return x\n```\n",
    );
    let h = &out.files[0].hunks[0];
    assert!(h.defines.is_empty(), "{:?}", h.defines);
    assert!(h.symbols.is_empty(), "{:?}", h.symbols);
    assert!(h.uses.contains(&"demo".to_string()), "{:?}", h.uses);
}

#[test]
fn a_fence_whose_language_has_no_grammar_is_left_alone() {
    let out = md(
        "# Run\n\n```console\n$ old --flag\n```\n",
        "# Run\n\n```console\n$ new --other\n```\n",
    );
    assert!(
        out.files[0].hunks[0].uses.is_empty(),
        "{:?}",
        out.files[0].hunks[0].uses
    );
}

#[test]
fn an_info_string_with_attributes_still_resolves() {
    let out = md(
        "# X\n\n```rs title=\"main.rs\"\nlet a = one();\n```\n",
        "# X\n\n```rs title=\"main.rs\"\nlet a = two();\n```\n",
    );
    assert!(
        out.files[0].hunks[0].uses.contains(&"two".to_string()),
        "{:?}",
        out.files[0].hunks[0].uses
    );
}

#[test]
fn injected_rows_are_reported_in_the_prose_file_coordinates() {
    // the use must land on the markdown line it is written on, not on a line
    // numbered from the top of the fence
    let out = md(
        "# T\n\ntext\n\n```python\na = 1\nb = alpha()\n```\n",
        "# T\n\ntext\n\n```python\na = 1\nb = beta()\n```\n",
    );
    let h = &out.files[0].hunks[0];
    assert_eq!(h.new_range, [7, 7], "{:?}", h.new_range);
    assert!(h.uses.contains(&"beta".to_string()), "{:?}", h.uses);
}
