//! Hunk accounting: every hunk the diff produced is either ordered or recorded
//! as dropped, with the reason. Without this a consumer cannot tell a hunk the
//! engine deliberately removed from one it never found.
use ordo::model::{DropReason, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

#[test]
fn an_import_hunk_is_kept_as_noise_rather_than_dropped() {
    // it follows from the real change rather than being it — but a reviewer
    // still wants to see a new dependency arrive, and a dropped one used to
    // make a *moved* import read as a deletion with no counterpart
    let out = run(serde_json::json!({
        "changes": [{
            "path": "a.py",
            "old": "import os\n\n\ndef run():\n    return 1\n",
            "new": "import os\nimport sys\n\n\ndef run():\n    return 2\n",
        }]
    }));
    let f = &out.files[0];
    assert!(f.dropped.is_empty(), "{:?}", f.dropped);
    let imp = f
        .hunks
        .iter()
        .find(|h| h.new_range[0] == 2)
        .expect("the import hunk");
    assert!(imp.noise, "an import hunk is skippable, not invisible");
    assert_eq!(imp.rationale, "adds import sys");
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

#[test]
fn a_deleted_import_is_an_import_hunk_too() {
    // a hunk that only deletes has no new side to classify from; without
    // reading the old side, a removed import read as a plain `other` hunk
    // while an added one was an import — the same asymmetry, one level down
    let out = run(serde_json::json!({
        "changes": [{
            "path": "a.py",
            "old": "import os\nfrom loguru import logger\n\n\ndef run():\n    return 1\n",
            "new": "import os\n\n\ndef run():\n    return 1\n",
        }]
    }));
    let h = &out.files[0].hunks[0];
    assert_eq!(h.category, ordo::model::Category::Import);
    assert!(h.noise, "as skippable as the addition it mirrors");
    // and the wording still comes from the removal path, which knows what left
    assert_eq!(h.rationale, "removes import logger");
}

#[test]
fn a_deleted_definition_is_not_mistaken_for_an_import() {
    let out = run(serde_json::json!({
        "changes": [{
            "path": "a.py",
            "old": "import os\n\n\ndef gone():\n    return 1\n\n\ndef stays():\n    return 2\n",
            "new": "import os\n\n\ndef stays():\n    return 2\n",
        }]
    }));
    let h = &out.files[0].hunks[0];
    assert_eq!(h.category, ordo::model::Category::Other);
    assert!(!h.noise);
    assert_eq!(h.rationale, "removes gone");
}
