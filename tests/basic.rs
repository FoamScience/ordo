//! Pipeline + invariant tests. Python change: an import, a helper definition,
//! and a top-level call that uses it — comprehension order must put import
//! first and the definition before its use.
use ordo::model::{Category, Input};

const OLD: &str = "# a\n# b\n# c\n# d\n# e\n";
const NEW: &str =
    "import os\n# a\n# b\ndef helper():\n    return os.getpid()\n# c\n# d\nx = helper()\n# e\n";

fn input(strategy: &str) -> Input {
    let j = serde_json::json!({
        "changes": [ { "path": "m.py", "old": OLD, "new": NEW } ],
        "options": { "strategy": strategy }
    });
    serde_json::from_value(j).unwrap()
}

#[test]
fn permutation_nothing_lost() {
    let out = ordo::run(input("comprehension"));
    let total: usize = out.files.iter().map(|f| f.hunks.len()).sum();
    assert!(total >= 3, "expected >=3 hunks, got {total}");
    assert_eq!(
        out.order.len(),
        total,
        "order must list every hunk exactly once"
    );
    let mut ids: Vec<_> = out.order.iter().map(|o| o.hunk.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        total,
        "order must be a permutation (no dupes/missing)"
    );
}

#[test]
fn import_first_and_def_before_use() {
    let out = ordo::run(input("comprehension"));
    let hunks = &out.files[0].hunks;

    let import_oi = hunks
        .iter()
        .find(|h| h.category == Category::Import)
        .expect("import")
        .order_index;
    let def_oi = hunks
        .iter()
        .find(|h| h.category == Category::Definition)
        .expect("def")
        .order_index;
    let use_oi = hunks
        .iter()
        .find(|h| h.uses.iter().any(|u| u == "helper"))
        .expect("use of helper")
        .order_index;

    assert_eq!(import_oi, 0, "import must lead");
    assert!(
        def_oi < use_oi,
        "helper definition (oi={def_oi}) must precede its use (oi={use_oi})"
    );
}

#[test]
fn def_edge_derived() {
    let out = ordo::run(input("comprehension"));
    assert!(
        out.edges.iter().any(|e| e.why.contains("helper")),
        "a def→use edge for helper should be derived; edges: {:?}",
        out.edges
    );
}

#[test]
fn lua_def_before_use() {
    // lua is the eating-own-dogfood language (gitplay is a Lua plugin)
    let j = serde_json::json!({
        "changes": [ { "path": "m.lua",
            "old": "-- a\n-- b\n-- c\n-- d\n",
            "new": "-- a\nlocal function helper()\n  return 1\nend\n-- c\n-- d\nlocal x = helper()\n" } ]
    });
    let out = ordo::run(serde_json::from_value(j).unwrap());
    let h = &out.files[0].hunks;
    let def = h
        .iter()
        .find(|x| x.defines.iter().any(|d| d == "helper"))
        .expect("helper def")
        .order_index;
    let use_ = h
        .iter()
        .find(|x| x.uses.iter().any(|u| u == "helper"))
        .expect("helper use")
        .order_index;
    assert!(
        def < use_,
        "lua: helper definition (oi={def}) must precede its use (oi={use_})"
    );
}

#[test]
fn deterministic() {
    let a = serde_json::to_string(&ordo::run(input("comprehension"))).unwrap();
    let b = serde_json::to_string(&ordo::run(input("comprehension"))).unwrap();
    assert_eq!(a, b, "output must be deterministic");
}

#[test]
fn file_strategy_keeps_position() {
    let out = ordo::run(input("file"));
    let hunks = &out.files[0].hunks;
    // in file order, order_index tracks new_range start ascending
    let mut prev = 0usize;
    let mut ordered: Vec<_> = hunks.iter().collect();
    ordered.sort_by_key(|h| h.order_index);
    for h in ordered {
        assert!(
            h.new_range[0] >= prev,
            "file strategy must preserve position"
        );
        prev = h.new_range[0];
    }
}
