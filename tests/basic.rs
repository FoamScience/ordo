//! Pipeline + invariant tests. Python change: an import, a helper definition,
//! and a top-level call that uses it — import hunks are skipped entirely, and
//! comprehension order must put the definition before its use.
mod fixture;
use fixture::run_json;
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
    // the import hunk is skipped; the helper def + its use remain
    assert!(total >= 2, "expected >=2 non-import hunks, got {total}");
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
fn imports_are_noise_and_defs_come_before_uses() {
    let out = ordo::run(input("comprehension"));
    let hunks = &out.files[0].hunks;

    // an import hunk is visible but skippable: it follows from the real change
    // rather than being it. Dropping it outright hid new dependencies and made
    // a moved import read as a deletion with no counterpart.
    for h in hunks.iter().filter(|h| h.category == Category::Import) {
        assert!(h.noise, "an import hunk is noise");
    }
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

    assert!(
        def_oi < use_oi,
        "helper definition (oi={def_oi}) must precede its use (oi={use_oi})"
    );
}

#[test]
fn def_edge_derived() {
    let out = ordo::run(input("comprehension"));
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: helper"]);
}

#[test]
fn lua_def_before_use() {
    // lua is the eating-own-dogfood language (gitplay is a Lua plugin)
    let j = serde_json::json!({
        "changes": [ { "path": "m.lua",
            "old": "-- a\n-- b\n-- c\n-- d\n",
            "new": "-- a\nlocal function helper()\n  return 1\nend\n-- c\n-- d\nlocal x = helper()\n" } ]
    });
    let out = run_json(j);
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

#[test]
fn a_dependency_cycle_still_orders_every_hunk_once() {
    // the topological sort carries its ready set across iterations now; a cycle
    // falls back to the same deterministic key, and nothing may be lost or
    // repeated on either path
    let out = run_json(serde_json::json!({
        "changes": [
            { "path": "a.py",
              "old": "def f():\n    return 1\n",
              "new": "def f():\n    return g()\n" },
            { "path": "b.py",
              "old": "def g():\n    return 2\n",
              "new": "def g():\n    return f()\n" }
        ]
    }));
    let total: usize = out.files.iter().map(|f| f.hunks.len()).sum();
    assert_eq!(out.order.len(), total, "{:?}", out.order);
    let mut ids: Vec<&str> = out.order.iter().map(|o| o.hunk.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "a hunk was ordered twice: {ids:?}");
}

/// A doc describes code, so the code is read first and the doc judged against
/// it. Prose carries no symbols, so nothing in the graph forces that — a
/// markdown hunk lands wherever the file order puts it, which is alphabetical
/// and so usually first. `docs_last` is the switch; only prose moves, and
/// every strategy answers to it.
#[test]
fn docs_sort_after_the_code_unless_told_otherwise() {
    let changes = serde_json::json!([
        { "path": "README.md", "old": "# Tool\n\nDocs.\n", "new": "# Tool\n\nDocs, updated.\n" },
        { "path": "pkg.json", "old": "{\n  \"v\": \"1\"\n}\n", "new": "{\n  \"v\": \"2\"\n}\n" },
        { "path": "src/core.py", "old": "def run(x):\n    return x\n",
          "new": "def run(x, y):\n    return x + y\n" },
        // sorts after README.md alphabetically, and is a use rather than a
        // definition — so it shares a rank with the doc under every strategy
        // and only `docs_last` can separate the two
        { "path": "src/use.py", "old": "from core import run\n\ndef go():\n    return run(1)\n",
          "new": "from core import run\n\ndef go():\n    return run(1, 2)\n" }
    ]);
    let order = |strategy: &str, docs_last: bool| -> Vec<String> {
        let out = run_json(serde_json::json!({
            "changes": changes,
            "options": { "docs_last": docs_last, "strategy": strategy }
        }));
        out.order.iter().map(|o| o.path.clone()).collect()
    };
    let at = |v: &[String], p: &str| v.iter().position(|x| x == p).expect(p);

    for strategy in ["comprehension", "defs-first", "file"] {
        // the default: the README is read after the code it describes
        let last = order(strategy, true);
        assert_eq!(
            last.last().map(String::as_str),
            Some("README.md"),
            "{strategy}: {last:?}"
        );
        // …and a data file is not prose: `pkg.json` keeps its place, because a
        // config change often drives the code around it
        assert!(
            at(&last, "pkg.json") < at(&last, "README.md"),
            "{strategy}: {last:?}"
        );

        // off, the file order decides again, and git hands them over
        // alphabetically — README.md ahead of src/use.py
        let first = order(strategy, false);
        assert!(
            at(&first, "README.md") < at(&first, "src/use.py"),
            "{strategy}: {first:?}"
        );
    }
}
