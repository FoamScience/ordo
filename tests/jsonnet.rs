//! jsonnet (with jrsonnet's extensions): objects are the main structure, so a
//! `field` is both a definition and a member of the object above it — the
//! config shape. `local x = …` is a `bind`, a method `f(x):: …` is a field,
//! and `import`/`importstr`/`importbin` are one node kind.
mod fixture;
use fixture::{hunks as one, run_file as run};

#[test]
fn a_local_links_to_where_it_is_referenced() {
    let old = "local version = '1.0';\n{\n  name: 'demo',\n}\n";
    let new = "local version = '2.0';\n{\n  name: 'demo',\n  rev: version,\n}\n";
    let out = run("main.jsonnet", old, new);
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: version"]);
}

#[test]
fn self_and_dollar_member_reads_are_uses_of_the_field() {
    let old = "{\n  port:: 80,\n  a: 1,\n  b: 2,\n  c: 3,\n  url: 'x',\n  d: 4,\n  e: 5,\n  f: 6,\n  alt: 'y',\n}\n";
    let new = "{\n  port:: 8080,\n  a: 1,\n  b: 2,\n  c: 3,\n  url: 'http://h:' + self.port,\n  d: 4,\n  e: 5,\n  f: 6,\n  alt: $.port,\n}\n";
    let out = run("svc.libsonnet", old, new);
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: port", "def→use: port"], "{out:?}");
}

#[test]
fn a_field_nests_under_its_object_and_a_method_is_a_field() {
    let hs = one(
        "main.jsonnet",
        "{\n  server: {\n    port: 80,\n    a: 1,\n    b: 2,\n    c: 3,\n    greet(name):: 'hi ' + name,\n  },\n}\n",
        "{\n  server: {\n    port: 81,\n    a: 1,\n    b: 2,\n    c: 3,\n    greet(name):: 'hello ' + name,\n  },\n}\n",
    );
    let enclosing: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert_eq!(
        enclosing,
        vec![Some("server.port"), Some("server.greet")],
        "{hs:?}"
    );
}

#[test]
fn a_computed_field_has_no_name() {
    let hs = one(
        "main.jsonnet",
        "local k = 'a';\n{\n  [k]: 1,\n}\n",
        "local k = 'a';\n{\n  [k]: 2,\n}\n",
    );
    assert!(
        hs.iter()
            .all(|h| !h.defines.iter().any(|d| d.starts_with('['))),
        "{hs:?}"
    );
    assert!(
        hs.iter().any(|h| h.uses.contains(&"k".to_string())),
        "{hs:?}"
    );
}

#[test]
fn an_import_is_bound_to_the_local_that_holds_it() {
    let hs = one(
        "main.jsonnet",
        "local a = 1;\n{ x: a }\n",
        "local a = 1;\nlocal lib = import 'lib.libsonnet';\n{ x: a }\n",
    );
    let defines: Vec<&[String]> = hs.iter().map(|h| h.defines.as_slice()).collect();
    assert_eq!(defines, vec![["lib".to_string()].as_slice()], "{hs:?}");
}

#[test]
fn text_blocks_and_verbatim_strings_parse_cleanly() {
    let out = run(
        "main.jsonnet",
        "{\n  t: |||\n    a\n  |||,\n}\n",
        "{\n  t: |||-\n    b\n  |||,\n  v: @'x',\n  w: f(1) tailstrict,\n}\n",
    );
    assert!(
        !out.files[0].degraded && !out.files[0].unsupported,
        "{out:?}"
    );
}

#[test]
fn a_parameter_or_loop_variable_is_not_a_use_of_an_outer_name() {
    let old = "local x = 1;\nlocal a = 1;\nlocal b = 2;\nlocal c = 3;\n{\n  f(x):: x,\n  g: [x for x in [1]],\n}\n";
    let new = "local x = 2;\nlocal a = 1;\nlocal b = 2;\nlocal c = 3;\n{\n  f(x):: x + 1,\n  g: [x * 2 for x in [1]],\n}\n";
    let out = run("main.jsonnet", old, new);
    assert!(out.edges.is_empty(), "{:?}", out.edges);
}
