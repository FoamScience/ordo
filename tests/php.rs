//! php: functions, classes, interfaces, traits and enums are definitions, and
//! so is a property, like a java field. A variable keeps its `$`, so `$user`
//! and a function `user()` never meet; `$this->count` reads as a use of the
//! property `$count`. `use` statements and `require`/`include` are imports.
mod fixture;
use fixture::{hunks as one, run_file as run};

const PAD: &str = "\nfunction a() {}\nfunction b() {}\nfunction c() {}\n";

#[test]
fn a_function_links_to_where_it_is_called() {
    let old = [
        "<?php\nfunction greet($n) { return 'hi'; }\n",
        PAD,
        "\nfunction main() {\n  return 1;\n}\n",
    ]
    .concat();
    let new = [
        "<?php\nfunction greet($n) { return 'hello'; }\n",
        PAD,
        "\nfunction main() {\n  return greet('x');\n}\n",
    ]
    .concat();
    let out = run("app.php", &old, &new);
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: greet"], "{out:?}");
}

#[test]
fn a_property_is_used_through_this() {
    let old = "<?php\nclass Counter {\n  private int $count = 0;\n  public function a() {}\n  public function b() {}\n  public function c() {}\n  public function d() {}\n  public function bump() {\n    return 1;\n  }\n}\n";
    let new = "<?php\nclass Counter {\n  private int $count = 1;\n  public function a() {}\n  public function b() {}\n  public function c() {}\n  public function d() {}\n  public function bump() {\n    return $this->count + 1;\n  }\n}\n";
    let out = run("Counter.php", old, new);
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: $count"], "{out:?}");
}

#[test]
fn a_method_edit_reads_against_its_class() {
    let hs = one(
        "User.php",
        "<?php\nclass User {\n  public function name(): string {\n    return 'a';\n  }\n}\n",
        "<?php\nclass User {\n  public function name(): string {\n    return 'b';\n  }\n}\n",
    );
    let enclosing: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert_eq!(enclosing, vec![Some("User.name")], "{hs:?}");
}

#[test]
fn a_variable_keeps_its_sigil_and_is_not_a_function() {
    let hs = one(
        "a.php",
        "<?php\nfunction user() {}\n",
        "<?php\nfunction user() {}\n$user = 1;\necho $user;\n",
    );
    let uses: Vec<&String> = hs.iter().flat_map(|h| &h.uses).collect();
    assert!(!uses.iter().any(|u| *u == "user"), "{hs:?}");
}

#[test]
fn use_binds_the_last_segment_or_its_alias() {
    let hs = one(
        "a.php",
        "<?php\nnamespace App;\n\nuse App\\Models\\Post;\n\nclass A {}\n",
        "<?php\nnamespace App;\n\nuse App\\Models\\Post;\nuse App\\Models\\User;\nuse Foo\\{Bar, Baz as Q};\n\nclass A {}\n",
    );
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(rats, vec!["adds import Bar, Q, User"], "{hs:?}");
}

#[test]
fn require_names_the_file_it_pulls_in() {
    let hs = one(
        "a.php",
        "<?php\nfunction f() {}\n",
        "<?php\nrequire_once __DIR__ . '/helpers.php';\nfunction f() {}\n",
    );
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(rats, vec!["adds import /helpers.php"], "{hs:?}");
}

#[test]
fn html_around_the_tags_parses_cleanly() {
    let out = run(
        "page.php",
        "<html><body><?php echo $title; ?></body></html>\n",
        "<html><body><h1><?= $title ?></h1><?php foreach ($items as $i): ?><li><?= $i ?></li><?php endforeach; ?></body></html>\n",
    );
    assert!(
        !out.files[0].degraded && !out.files[0].unsupported,
        "{out:?}"
    );
}
