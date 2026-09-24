//! A definition whose promise to its callers changed in a way no single line
//! shows, held against the calls to it in the same change.
mod fixture;
use fixture::run_json;

fn notes(v: serde_json::Value) -> Vec<String> {
    run_json(v)
        .files
        .iter()
        .flat_map(|f| f.hunks.iter().flat_map(|h| h.notes.clone()))
        .collect()
}

fn with<'a>(n: &'a [String], word: &str) -> Vec<&'a String> {
    n.iter().filter(|s| s.contains(word)).collect()
}

#[test]
fn a_function_that_became_async_names_the_callers_that_do_not_await_it() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u):\n    return u\n",
       "new": "async def fetch(u):\n    return u\n"},
      {"path": "cli.py",
       "old": "from api import fetch\n\ndef run():\n    fetch('a')\n    if fetch('b'):\n        pass\n\nasync def go():\n    await fetch('c')\n    x = await fetch('d')\n",
       "new": "from api import fetch\n\ndef run():\n    fetch('a')\n    if fetch('b'):\n        pass\n\nasync def go():\n    await fetch('c')\n    x = await fetch('d')\n"}]}));
    let a = with(&n, "became async");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("cli.py:L4, cli.py:L5"), "{a:?}");
    assert!(a[0].contains("2 calls"), "{a:?}");
}

#[test]
fn an_async_function_handed_to_the_event_loop_says_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "import asyncio\n\ndef fetch(u):\n    return u\n\ndef main():\n    asyncio.run(fetch('a'))\n",
       "new": "import asyncio\n\nasync def fetch(u):\n    return u\n\ndef main():\n    asyncio.run(fetch('a'))\n"}]}));
    assert!(with(&n, "became async").is_empty(), "{n:?}");
}

#[test]
fn a_method_that_became_async_is_matched_through_self_only() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "svc.py",
       "old": "class S:\n    def load(self):\n        return 1\n\n    def run(self, other):\n        self.load()\n        other.load()\n",
       "new": "class S:\n    async def load(self):\n        return 1\n\n    def run(self, other):\n        self.load()\n        other.load()\n"}]}));
    let a = with(&n, "became async");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("svc.py:L6") && !a[0].contains("L7"), "{a:?}");
}

#[test]
fn a_js_function_that_became_async_is_caught_too() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.js",
       "old": "function save(x) {\n  return x;\n}\nfunction main() {\n  save(1);\n}\n",
       "new": "async function save(x) {\n  return x;\n}\nfunction main() {\n  save(1);\n}\n"}]}));
    assert_eq!(with(&n, "became async").len(), 1, "{n:?}");
}

#[test]
fn a_property_that_became_a_method_names_the_reads_left_uncalled() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "shape.py",
       "old": "class Box:\n    @property\n    def empty(self):\n        return True\n\n    def show(self):\n        if self.empty:\n            return 1\n",
       "new": "class Box:\n    def empty(self):\n        return True\n\n    def show(self):\n        if self.empty:\n            return 1\n"},
      {"path": "use.py",
       "old": "from shape import Box\n\nb = Box()\nprint(b.empty)\nprint(b.empty())\n",
       "new": "from shape import Box\n\nb = Box()\nprint(b.empty)\nprint(b.empty())\n"},
      {"path": "other.py",
       "old": "def f(q):\n    return q.empty\n",
       "new": "def f(q):\n    return q.empty\n"}]}));
    let a = with(&n, "went from a property to a method");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("shape.py:L6") && a[0].contains("use.py:L4"),
        "{a:?}"
    );
    // a file that never names Box may be reading some other `empty`
    assert!(
        !a[0].contains("other.py") && !a[0].contains("use.py:L5"),
        "{a:?}"
    );
}

#[test]
fn a_method_that_became_a_property_names_the_calls_left_behind() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "shape.py",
       "old": "class Box:\n    def size(self):\n        return 1\n\n    def show(self):\n        return self.size()\n",
       "new": "class Box:\n    @functools.cached_property\n    def size(self):\n        return 1\n\n    def show(self):\n        return self.size()\n"}]}));
    let a = with(&n, "went from a method to a property");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("shape.py:L7"), "{a:?}");
}

#[test]
fn a_parameter_inserted_mid_signature_names_the_positional_calls_left_as_they_were() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u, retries=1):\n    return u\n",
       "new": "def fetch(u, timeout=5, retries=1):\n    return u\n"},
      {"path": "cli.py",
       "old": "from api import fetch\n\nfetch('a', 3)\nfetch('b')\nfetch('c', 2)\n",
       "new": "from api import fetch\n\nfetch('a', 3)\nfetch('b')\nfetch('c', 9, 2)\n"}]}));
    let a = with(&n, "land on different parameters");
    assert_eq!(a.len(), 1, "{n:?}");
    // L3 was left as it was; L4 passes nothing past `u`; L5 was rewritten
    assert!(
        a[0].contains("cli.py:L3") && !a[0].contains("L4") && !a[0].contains("L5"),
        "{a:?}"
    );
}

#[test]
fn a_parameter_added_at_the_end_shifts_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u):\n    return u\n\nfetch('a')\n",
       "new": "def fetch(u, retries=1):\n    return u\n\nfetch('a')\n"}]}));
    assert!(with(&n, "land on different").is_empty(), "{n:?}");
}

#[test]
fn a_renamed_keyword_names_the_calls_still_passing_the_old_one() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u, retries=1):\n    return u\n\nfetch('a', retries=2)\n",
       "new": "def fetch(u, attempts=1):\n    return u\n\nfetch('a', retries=2)\nfetch('b', attempts=2)\n"}]}));
    let a = with(&n, "no longer takes `retries`");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("api.py:L4") && !a[0].contains("L5"), "{a:?}");
}

#[test]
fn a_kwargs_catch_all_takes_any_old_keyword() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u, retries=1):\n    return u\n\nfetch('a', retries=2)\n",
       "new": "def fetch(u, **opts):\n    return u\n\nfetch('a', retries=2)\n"}]}));
    assert!(with(&n, "no longer takes").is_empty(), "{n:?}");
}

#[test]
fn a_changed_default_names_the_calls_that_rely_on_it() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u, timeout=30):\n    return u\n\nfetch('a')\nfetch('b', 10)\nfetch('c', timeout=1)\n",
       "new": "def fetch(u, timeout=5):\n    return u\n\nfetch('a')\nfetch('b', 10)\nfetch('c', timeout=1)\n"}]}));
    let a = with(&n, "default for `timeout` changed from 30 to 5");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("api.py:L4") && !a[0].contains("L5") && !a[0].contains("L6"),
        "{a:?}"
    );
}

#[test]
fn a_patch_target_left_on_a_renamed_function_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/cfg.py",
       "old": "def parse_cfg(p):\n    return p\n",
       "new": "def load_cfg(p):\n    return p\n"},
      {"path": "tests/test_cfg.py",
       "old": "from unittest import mock\n\n@mock.patch('pkg.cfg.parse_cfg')\ndef test_a(m):\n    pass\n",
       "new": "from unittest import mock\n\n@mock.patch('pkg.cfg.parse_cfg')\ndef test_a(m):\n    pass\n\n@mock.patch('other.parse_cfg')\ndef test_b(m):\n    pass\n"}]}));
    let a = with(&n, "still name");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("tests/test_cfg.py:L3 'pkg.cfg.parse_cfg'"),
        "{a:?}"
    );
    // a module that is not the one the name left says nothing about it
    assert!(!a[0].contains("other.parse_cfg"), "{a:?}");
}

#[test]
fn an_entry_point_left_on_a_moved_function_is_named() {
    let n = notes(
        serde_json::json!({"options": {"cross_file": true}, "changes": [
      {"path": "pkg/old.py",
       "old": "def main():\n    return 1\n\ndef other():\n    return 2\n",
       "new": "def other():\n    return 2\n"},
      {"path": "pkg/cli.py",
       "old": "",
       "new": "def main():\n    return 1\n"},
      {"path": "pyproject.toml",
       "old": "[project.scripts]\nrun = \"pkg.old:main\"\n",
       "new": "[project.scripts]\nrun = \"pkg.old:main\"\n"}]}),
    );
    let a = with(&n, "moved from pkg/old.py");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("pyproject.toml:L2"), "{a:?}");
}

#[test]
fn a_test_that_drops_an_assertion_next_to_the_code_it_tests_is_asked_about() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/calc.py",
       "old": "def total(xs):\n    return sum(xs)\n",
       "new": "def total(xs):\n    return sum(xs) + 1\n"},
      {"path": "tests/test_calc.py",
       "old": "from pkg.calc import total\n\ndef test_total():\n    assert total([1]) == 1\n    assert total([]) == 0\n",
       "new": "from pkg.calc import total\n\ndef test_total():\n    assert total([1]) == 2\n"}]}));
    let a = with(&n, "drops 1 assertion");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("edits total"), "{a:?}");
}

#[test]
fn a_skip_added_next_to_the_code_it_tests_is_asked_about() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/calc.py",
       "old": "def total(xs):\n    return sum(xs)\n",
       "new": "def total(xs):\n    return sum(xs) + 1\n"},
      {"path": "tests/test_calc.py",
       "old": "import pytest\nfrom pkg.calc import total\n\ndef test_total():\n    assert total([1]) == 1\n",
       "new": "import pytest\nfrom pkg.calc import total\n\n@pytest.mark.skip\ndef test_total():\n    assert total([1]) == 1\n"}]}));
    assert_eq!(with(&n, "adds a skip").len(), 1, "{n:?}");
}

#[test]
fn a_test_rewritten_without_losing_assertions_says_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/calc.py",
       "old": "def total(xs):\n    return sum(xs)\n",
       "new": "def total(xs):\n    return sum(xs) + 1\n"},
      {"path": "tests/test_calc.py",
       "old": "from pkg.calc import total\n\ndef test_total():\n    assert total([1]) == 1\n",
       "new": "from pkg.calc import total\n\ndef test_total():\n    assert total([1]) == 2\n"}]}));
    assert!(with(&n, "loosened").is_empty(), "{n:?}");
}

#[test]
fn an_override_left_on_the_old_base_signature_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "base.py",
       "old": "class Handler:\n    def handle(self, req):\n        return req\n",
       "new": "class Handler:\n    def handle(self, req, ctx):\n        return req\n"},
      {"path": "impl.py",
       "old": "from base import Handler\n\nclass Json(Handler):\n    def handle(self, req):\n        return 1\n\nclass Xml(Handler):\n    def handle(self, req, ctx):\n        return 2\n",
       "new": "from base import Handler\n\nclass Json(Handler):\n    def handle(self, req):\n        return 1\n\nclass Xml(Handler):\n    def handle(self, req, ctx):\n        return 2\n"}]}));
    let a = with(&n, "override");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("Json.handle impl.py:L4") && !a[0].contains("Xml"),
        "{a:?}"
    );
}

#[test]
fn a_new_abstract_method_names_the_subclasses_that_lack_it() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "base.py",
       "old": "import abc\n\nclass Store(abc.ABC):\n    @abc.abstractmethod\n    def get(self, k):\n        ...\n",
       "new": "import abc\n\nclass Store(abc.ABC):\n    @abc.abstractmethod\n    def get(self, k):\n        ...\n\n    @abc.abstractmethod\n    def put(self, k, v):\n        ...\n"},
      {"path": "mem.py",
       "old": "from base import Store\n\nclass Mem(Store):\n    def get(self, k):\n        return 1\n\nclass Full(Store):\n    def get(self, k):\n        return 1\n\n    def put(self, k, v):\n        pass\n",
       "new": "from base import Store\n\nclass Mem(Store):\n    def get(self, k):\n        return 1\n\nclass Full(Store):\n    def get(self, k):\n        return 1\n\n    def put(self, k, v):\n        pass\n"}]}));
    let a = with(&n, "gained abstract method put");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("Mem mem.py:L3") && !a[0].contains("Full"),
        "{a:?}"
    );
}

#[test]
fn a_changed_exception_type_names_the_handlers_that_catch_the_old_one() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "store.py",
       "old": "def get(k):\n    raise KeyError(k)\n",
       "new": "def get(k):\n    raise errors.MissingKey(k)\n"},
      {"path": "use.py",
       "old": "from store import get\n",
       "new": "from store import get\n\ntry:\n    get('a')\nexcept KeyError:\n    pass\n\ntry:\n    get('b')\nexcept (KeyError, MissingKey):\n    pass\n\ntry:\n    get('c')\nexcept Exception:\n    pass\n"}]}));
    let a = with(&n, "now raises MissingKey instead of KeyError");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("use.py:L4") && !a[0].contains("L9") && !a[0].contains("L14"),
        "{a:?}"
    );
}

#[test]
fn a_changed_enum_value_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "color.py",
       "old": "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n    BLUE = 2\n",
       "new": "from enum import Enum\n\nclass Color(Enum):\n    RED = 5\n    BLUE = 2\n    GREEN = 3\n"},
      {"path": "color.ts",
       "old": "enum Mode { Fast = 'f', Slow = 's' }\n",
       "new": "enum Mode { Fast = 'fast', Slow = 's' }\n"}]}));
    assert_eq!(
        with(&n, "Color.RED's value changed from 1 to 5").len(),
        1,
        "{n:?}"
    );
    assert_eq!(with(&n, "Mode.Fast's value changed").len(), 1, "{n:?}");
    assert!(
        with(&n, "BLUE").is_empty() && with(&n, "GREEN").is_empty(),
        "{n:?}"
    );
}

#[test]
fn a_dropped_lock_or_await_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "svc.py",
       "old": "async def save(db, lock, row):\n    with lock:\n        db.put(row)\n    await db.flush()\n    return 1\n",
       "new": "async def save(db, lock, row):\n    db.put(row)\n    db.flush()\n    return 1\n"}]}));
    let a = with(&n, "no longer enters `lock`");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(
        a[0].contains("calls `db.flush()` without awaiting it"),
        "{a:?}"
    );
}

#[test]
fn a_lock_swapped_for_another_says_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "svc.py",
       "old": "def save(db, lock, row):\n    with lock:\n        db.put(row)\n",
       "new": "def save(db, lock, row):\n    with db.tx():\n        db.put(row)\n"}]}));
    assert!(with(&n, "no longer enters").is_empty(), "{n:?}");
}

#[test]
fn an_import_that_closes_a_cycle_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/a.py",
       "old": "from pkg.b import g\n\ndef f():\n    return g()\n",
       "new": "from pkg.b import g\n\ndef f():\n    return g()\n"},
      {"path": "pkg/b.py",
       "old": "def g():\n    return 1\n",
       "new": "from pkg.a import f\n\ndef g():\n    return 1\n\ndef h():\n    return f()\n"}]}));
    let a = with(&n, "closes a cycle");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("pkg/b.py → pkg/a.py → pkg/b.py"), "{a:?}");
}

#[test]
fn a_cycle_that_already_existed_says_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "pkg/a.py",
       "old": "from pkg.b import g\n\ndef f():\n    return g()\n",
       "new": "from pkg.b import g\n\ndef f():\n    return g() + 1\n"},
      {"path": "pkg/b.py",
       "old": "from pkg.a import f\n\ndef g():\n    return 1\n",
       "new": "from pkg.a import f\n\ndef g():\n    return 2\n"}]}));
    assert!(with(&n, "closes a cycle").is_empty(), "{n:?}");
}

#[test]
fn the_same_helper_added_to_two_files_is_named() {
    let body = "def slugify(s):\n    s = s.strip().lower()\n    return s.replace(' ', '-')\n";
    let n = notes(serde_json::json!({"changes": [
      {"path": "web/views.py", "old": "", "new": body},
      {"path": "cli/main.py", "old": "", "new": format!("import sys\n\n{body}")},
      {"path": "cli/other.py", "old": "", "new": "def slugify(s):\n    return urllib.parse.quote(s.encode('utf-8'), safe='')\n"}]}));
    let a = with(&n, "nearly verbatim");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("web/views.py:L1"), "{a:?}");
}

#[test]
fn a_renamed_positional_parameter_shifts_nothing() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.py",
       "old": "def fetch(u, retries=1):\n    return u\n\nfetch('a', 3)\n",
       "new": "def fetch(url, retries=1):\n    return url\n\nfetch('a', 3)\n"}]}));
    assert!(with(&n, "land on different").is_empty(), "{n:?}");
}

#[test]
fn a_dropped_await_in_js_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "svc.js",
       "old": "async function save(db) {\n  await db.flush();\n  return 1;\n}\n",
       "new": "async function save(db) {\n  db.flush();\n  return 1;\n}\n"}]}));
    assert_eq!(
        with(&n, "calls `db.flush()` without awaiting it").len(),
        1,
        "{n:?}"
    );
}

#[test]
fn a_consumer_outside_the_change_is_checked_as_a_caller() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "script/run.py",
       "old": "def main(cfg, steps=1):\n    return cfg\n",
       "new": "def main(cfg, mesh, steps=1):\n    return cfg\n"}],
      "consumers": [
      {"path": "../pipeline/driver.py",
       "content": "from script.run import main\n\nmain(cfg, 3)\n"}]}));
    let a = with(&n, "land on different parameters");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("../pipeline/driver.py:L3"), "{a:?}");
}

#[test]
fn a_consumer_is_never_ordered_or_output() {
    let out = run_json(serde_json::json!({"changes": [
      {"path": "a.py", "old": "def f():\n    return 1\n", "new": "def f():\n    return 2\n"}],
      "consumers": [{"path": "../b/use.py", "content": "from a import f\nf()\n"}]}));
    assert_eq!(out.files.len(), 1);
    assert!(out.order.iter().all(|o| o.path == "a.py"));
}

#[test]
fn an_async_call_tested_with_js_and_is_named() {
    let n = notes(serde_json::json!({"changes": [
      {"path": "api.js",
       "old": "function save(x) {\n  return x;\n}\nfunction main(ready) {\n  if (ready && save(1)) {\n    return 1;\n  }\n}\n",
       "new": "async function save(x) {\n  return x;\n}\nfunction main(ready) {\n  if (ready && save(1)) {\n    return 1;\n  }\n}\n"}]}));
    let a = with(&n, "became async");
    assert_eq!(a.len(), 1, "{n:?}");
    assert!(a[0].contains("api.js:L5"), "{a:?}");
}
