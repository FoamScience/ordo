//! P4: cross-file def→use. `helper` is defined in util.py but that file is
//! listed *second*; only a cross-file edge can pull its definition ahead of the
//! use in main.py (listed first).
use ordo::model::Input;

fn input(cross_file: bool) -> Input {
    let j = serde_json::json!({
        "changes": [
            { "path": "main.py", "old": "# m\n", "new": "# m\nx = helper()\n" },
            { "path": "util.py", "old": "# u\n", "new": "# u\ndef helper():\n    return 1\n" }
        ],
        "options": { "strategy": "comprehension", "cross_file": cross_file }
    });
    serde_json::from_value(j).unwrap()
}

fn pos(out: &ordo::model::Output, path: &str) -> usize {
    out.order.iter().position(|o| o.path == path).unwrap()
}

#[test]
fn cross_file_orders_def_before_use() {
    let out = ordo::run(input(true));
    assert!(
        pos(&out, "util.py") < pos(&out, "main.py"),
        "with cross_file, util.py's helper definition must precede its use in main.py; order: {:?}",
        out.order
    );
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: helper"], "exactly one cross-file edge");
}

#[test]
fn cross_file_off_keeps_input_file_order() {
    let out = ordo::run(input(false));
    assert!(
        pos(&out, "main.py") < pos(&out, "util.py"),
        "without cross_file, files keep input order; order: {:?}",
        out.order
    );
    assert!(
        out.edges.is_empty(),
        "cross_file off means no edges at all, not merely none naming helper: {:?}",
        out.edges
    );
}

#[test]
fn cross_file_rationale_names_the_other_file() {
    let out = ordo::run(input(true));
    let rats: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.rationale.as_str()))
        .collect();
    // both sides, exactly: `any` would have passed with one of the two missing
    assert_eq!(
        rats,
        vec![
            "uses helper, defined in util.py",
            "adds helper, used in main.py"
        ],
        "each side names the other file"
    );
}
