//! P23.1: one entry per *symbol* the change touches, rather than per hunk. A
//! projection of data the engine already has — `symbols`, the per-file status
//! maps, and every hunk's `uses` — never new analysis.
use ordo::model::{Input, LedgerEntry, Output, SymbolChange};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Vec<LedgerEntry> {
    run(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .ledger
}

fn find<'a>(l: &'a [LedgerEntry], name: &str) -> Option<&'a LedgerEntry> {
    l.iter().find(|e| e.name == name)
}

#[test]
fn an_added_definition_is_added() {
    let l = one(
        "a.py",
        "def f():\n    return 1\n",
        "def f():\n    return 1\n\ndef g():\n    return 2\n",
    );
    assert_eq!(find(&l, "g").map(|e| e.change), Some(SymbolChange::Added));
    // the tree-sitter kind rides along for anything still present
    assert_eq!(
        find(&l, "g").and_then(|e| e.kind.clone()),
        Some("function_definition".to_string())
    );
}

#[test]
fn a_changed_declaration_is_a_signature_and_a_changed_body_is_not() {
    let l = one(
        "a.py",
        "def f(x):\n    return x\n\ndef g():\n    return 1\n",
        "def f(x, y):\n    return x\n\ndef g():\n    return 2\n",
    );
    assert_eq!(
        find(&l, "f").map(|e| e.change),
        Some(SymbolChange::Signature)
    );
    assert_eq!(find(&l, "g").map(|e| e.change), Some(SymbolChange::Body));
}

#[test]
fn fan_in_names_the_hunks_that_use_it() {
    let out = run(serde_json::json!({
        "options": {"cross_file": true},
        "changes": [
          {"path": "api.py",
           "old": "def fetch(u):\n    return u\n\ndef main():\n    return fetch('a')\n",
           "new": "def fetch(u, r):\n    return u\n\ndef main():\n    return fetch('a', 3)\n"},
          {"path": "cli.py",
           "old": "from api import fetch\n\ndef run():\n    return fetch('b')\n",
           "new": "from api import fetch\n\ndef run():\n    return fetch('b', 1)\n"}
        ]
    }));
    let e = find(&out.ledger, "fetch").expect("fetch in ledger");
    assert_eq!(e.change, SymbolChange::Signature);
    // both call sites, across files — the fan-in the arity check will consume
    assert_eq!(e.used_by.len(), 2, "{:?}", e.used_by);
    // a symbol never counts as using itself
    assert!(!e.used_by.iter().any(|h| h == "h0"), "{:?}", e.used_by);
}

#[test]
fn a_rename_carries_the_name_it_had_before() {
    let old = "def parse_cfg(p):\n    a = 1\n    b = 2\n    c = 3\n    d = 4\n    e = 5\n    f = 6\n    g = 7\n    return p\n";
    let new = "def load_cfg(p):\n    a = 1\n    b = 2\n    c = 3\n    d = 4\n    e = 5\n    f = 6\n    g = 7\n    return p\n";
    let l = one("a.py", old, new);
    let e = find(&l, "load_cfg").expect("renamed symbol in ledger");
    assert_eq!(e.change, SymbolChange::Renamed);
    assert_eq!(e.from.as_deref(), Some("parse_cfg"));
}

#[test]
fn a_deleted_definition_is_removed_and_has_no_kind() {
    let l = one(
        "a.py",
        "def keep():\n    return 1\n\ndef gone():\n    return 2\n",
        "def keep():\n    return 1\n",
    );
    let e = find(&l, "gone").expect("removed symbol in ledger");
    assert_eq!(e.change, SymbolChange::Removed);
    // its defining node no longer exists to be asked for a kind
    assert!(e.kind.is_none(), "{e:?}");
}

#[test]
fn the_ledger_follows_the_reading_order() {
    // util.py's definition sorts ahead of main.py's use, so its ledger line does
    let out = run(serde_json::json!({
        "options": {"cross_file": true},
        "changes": [
          {"path": "main.py", "old": "x = 1\n", "new": "x = 1\ny = helper()\n"},
          {"path": "util.py", "old": "z = 0\n", "new": "z = 0\n\ndef helper():\n    return 2\n"}
        ]
    }));
    let names: Vec<&str> = out.ledger.iter().map(|e| e.name.as_str()).collect();
    let h = names.iter().position(|n| *n == "helper");
    assert!(h.is_some(), "{names:?}");
    assert_eq!(out.order[0].path, "util.py", "{:?}", out.order);
}

#[test]
fn one_line_per_symbol_not_per_hunk() {
    // a def touched by two separate hunks still gets a single ledger entry
    let old = "def f(x):\n    a = 1\n\n\n\n\n\n\n\n\n\n    return a\n";
    let new = "def f(x, y):\n    a = 2\n\n\n\n\n\n\n\n\n\n    return a\n";
    let l = one("a.py", old, new);
    assert_eq!(l.iter().filter(|e| e.name == "f").count(), 1, "{l:?}");
}

#[test]
fn every_entry_is_anchored_to_a_hunk() {
    // a ledger line that cannot point at a hunk is not actionable
    let out = run(serde_json::json!({"changes": [
        {"path": "a.py",
         "old": "def keep():\n    return 1\n\ndef gone():\n    return 2\n",
         "new": "def keep():\n    return 1\n\ndef added():\n    return 3\n"}]}));
    let ids: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.id.as_str()))
        .collect();
    assert!(!out.ledger.is_empty());
    for e in &out.ledger {
        assert!(ids.contains(&e.at.as_str()), "{e:?} not in {ids:?}");
    }
}

#[test]
fn the_pack_leads_with_the_ledger() {
    let out = run(serde_json::json!({"changes": [
        {"path": "a.py",
         "old": "def f(x):\n    return x\n",
         "new": "def f(x, y):\n    return x\n"}]}));
    let p = ordo::pack(&out);
    let (led, order) = (p.find("## ledger"), p.find("## reading order"));
    assert!(led.is_some(), "{p}");
    assert!(led < order, "the ledger is read before the hunks:\n{p}");
    assert!(p.contains("f — signature"), "{p}");
}
