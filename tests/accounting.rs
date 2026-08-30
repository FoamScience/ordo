//! Hunk accounting: every hunk the diff produced is either ordered or recorded
//! as dropped, with the reason. Without this a consumer cannot tell a hunk the
//! engine deliberately removed from one it never found.
use ordo::model::{DropReason, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

#[test]
fn dropped_imports_are_recorded_with_their_range() {
    let out = run(serde_json::json!({
        "changes": [{
            "path": "a.py",
            "old": "import os\n\n\ndef run():\n    return 1\n",
            "new": "import os\nimport sys\n\n\ndef run():\n    return 2\n",
        }]
    }));
    let f = &out.files[0];
    let d: Vec<_> = f.dropped.iter().filter(|d| d.reason == DropReason::Import).collect();
    assert_eq!(d.len(), 1, "{:?}", f.dropped);
    // the recorded range is the import hunk's own, not the file's
    assert_eq!(d[0].new_range, [2, 2], "{:?}", d[0]);
    assert!(f.hunks.iter().all(|h| h.new_range[0] != 2));
}

#[test]
fn only_comments_records_what_it_removed() {
    let src = serde_json::json!({
        "changes": [{
            "path": "a.py",
            "old": "# note\ndef run():\n    return 1\n",
            "new": "# NOTE\ndef run():\n    return 2\n",
        }],
        "options": { "only_comments": true }
    });
    let out = run(src);
    let f = &out.files[0];
    assert!(f.hunks.iter().all(|h| h.comment), "kept a non-comment hunk");
    assert!(
        f.dropped.iter().any(|d| d.reason == DropReason::NonComment),
        "{:?}",
        f.dropped
    );
}

#[test]
fn kept_plus_dropped_accounts_for_every_hunk() {
    // same change with and without `only_comments`: the unfiltered run says how
    // many hunks the diff produced, and the filtered one must still add up.
    let changes = serde_json::json!([{
        "path": "a.py",
        "old": "import os\n\n\n# note\ndef run():\n    return 1\n",
        "new": "import os\nimport sys\n\n\n# NOTE\ndef run():\n    return 2\n",
    }]);
    let full = run(serde_json::json!({ "changes": changes }));
    let produced = full.files[0].hunks.len() + full.files[0].dropped.len();

    let filtered = run(serde_json::json!({
        "changes": changes, "options": { "only_comments": true }
    }));
    let f = &filtered.files[0];
    assert_eq!(f.hunks.len() + f.dropped.len(), produced, "{:?}", f.dropped);
}

#[test]
fn nothing_dropped_leaves_the_field_empty() {
    let out = run(serde_json::json!({
        "changes": [{ "path": "a.py", "old": "def run():\n    return 1\n",
                      "new": "def run():\n    return 2\n" }]
    }));
    assert!(out.files[0].dropped.is_empty());
}
