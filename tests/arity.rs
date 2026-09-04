//! P23.2: a definition whose signature changed, against the calls to it in the
//! same change. Deliberately narrow — a false "wrong number of arguments" is
//! worse than a missed one, so everything it cannot state exactly is skipped.
use ordo::model::Input;

fn notes(v: serde_json::Value) -> Vec<String> {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().flat_map(|h| h.notes.clone()))
        .collect()
}

fn arity(n: &[String]) -> Vec<&String> {
    n.iter().filter(|s| s.contains("call sites")).collect()
}

#[test]
fn a_caller_left_behind_is_named() {
    let n = notes(
        serde_json::json!({"options": {"cross_file": true}, "changes": [
      {"path": "api.py",
       "old": "def fetch(u):\n    return u\n\ndef main():\n    return fetch('a')\n",
       "new": "def fetch(u, retries):\n    return u\n\ndef main():\n    return fetch('a', 3)\n"},
      {"path": "cli.py",
       "old": "from api import fetch\n\ndef run():\n    return fetch('b')\n",
       "new": "from api import fetch\n\ndef run():\n    return fetch('b')\n"}]}),
    );
    let a = arity(&n);
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("fetch"), "{a:?}");
    // the caller that was missed, by location
    assert!(a[0].contains("cli.py:L4"), "{a:?}");
}

#[test]
fn every_caller_updated_says_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u):\n    return u\n\ndef main():\n    return fetch('a')\n",
       "new": "def fetch(u, r):\n    return u\n\ndef main():\n    return fetch('a', 3)\n"}]}));
    assert!(arity(&n).is_empty(), "{n:?}");
}

#[test]
fn too_many_arguments_counts_too() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.py",
       "old": "def f(a, b):\n    return a\n\ndef m():\n    return f(1, 2)\n",
       "new": "def f(a):\n    return a\n\ndef m():\n    return f(1, 2)\n"}]}));
    assert_eq!(arity(&n).len(), 1, "{n:?}");
}

#[test]
fn an_optional_parameter_widens_the_range() {
    // required 1, total 2 — a one-argument call is still correct
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.py",
       "old": "def f(a):\n    return a\n\ndef m():\n    return f(1)\n",
       "new": "def f(a, b=2):\n    return a\n\ndef m():\n    return f(1)\n"}]}));
    assert!(arity(&n).is_empty(), "{n:?}");
}

#[test]
fn a_method_is_skipped_because_the_receiver_is_implicit() {
    // `c.m(1)` passes one argument to a two-parameter `m(self, a, b)`; counting
    // those against each other would report every method call as short
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.py",
       "old": "class C:\n    def m(self, a):\n        return a\n\ndef go(c):\n    return c.m(1)\n",
       "new": "class C:\n    def m(self, a, b):\n        return a\n\ndef go(c):\n    return c.m(1)\n"}]}));
    assert!(arity(&n).is_empty(), "{n:?}");
}

#[test]
fn a_variadic_definition_is_skipped() {
    // `*rest` makes the upper bound meaningless
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.py",
       "old": "def f(a, *rest):\n    return a\n\ndef m():\n    return f(1)\n",
       "new": "def f(a, b, *rest):\n    return a\n\ndef m():\n    return f(1)\n"}]}));
    assert!(arity(&n).is_empty(), "{n:?}");
}

#[test]
fn a_keyword_argument_call_is_skipped() {
    // passing by name says nothing about positional arity
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.py",
       "old": "def f(a):\n    return a\n\ndef m():\n    return f(a=1)\n",
       "new": "def f(a, b):\n    return a\n\ndef m():\n    return f(a=1)\n"}]}));
    assert!(arity(&n).is_empty(), "{n:?}");
}

#[test]
fn it_works_for_rust_too() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "a.rs",
       "old": "fn g(a: u32) -> u32 { a }\nfn main() { g(1); }\n",
       "new": "fn g(a: u32, b: u32) -> u32 { a }\nfn main() { g(1); }\n"}]}));
    assert_eq!(arity(&n).len(), 1, "{n:?}");
}
