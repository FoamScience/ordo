//! Config formats (json, yaml, toml): a key is a def named by its key text,
//! nested through a dotted path, and a member of the key above it. Covers key
//! naming per grammar, the nested path, the "changes k" wording (a key has no
//! signature), and the P15 detail layer naming the container key.
mod fixture;
use fixture::{hunks as one, run_json};

fn supported(path: &str, old: &str, new: &str) -> bool {
    let out = run_json(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }));
    !out.files[0].unsupported
}

#[test]
fn config_extensions_are_supported() {
    assert!(supported("a.yml", "a: 1\n", "a: 2\n"));
    assert!(supported("a.yaml", "a: 1\n", "a: 2\n"));
    assert!(supported("a.json", "{\"a\": 1}\n", "{\"a\": 2}\n"));
    assert!(supported("a.toml", "a = 1\n", "a = 2\n"));
}

#[test]
fn yaml_key_path_and_detail() {
    let old = "services:\n  web:\n    image: nginx:1.0\n  db:\n    image: pg\n";
    let new = "services:\n  web:\n    image: nginx:2.0\n  db:\n    image: pg\n";
    let hs = one("docker-compose.yml", old, new);
    let h = hs
        .iter()
        .find(|h| h.enclosing.as_deref() == Some("services.web.image"));
    assert!(h.is_some(), "{hs:?}");
    assert!(
        h.unwrap()
            .details
            .contains(&"changes image in services.web".to_string()),
        "{hs:?}"
    );
}

#[test]
fn json_keys_are_unquoted_and_nested() {
    let old = "{\n  \"scripts\": {\n    \"build\": \"tsc\"\n  }\n}\n";
    let new = "{\n  \"scripts\": {\n    \"build\": \"tsc\",\n    \"test\": \"vitest\"\n  }\n}\n";
    let hs = one("package.json", old, new);
    let h = hs.iter().find(|h| h.defines.contains(&"test".to_string()));
    assert!(h.is_some(), "{hs:?}");
    assert!(
        h.unwrap()
            .details
            .contains(&"adds test to scripts".to_string()),
        "{hs:?}"
    );
}

#[test]
fn toml_table_header_is_a_container() {
    let old = "[package]\nname = \"a\"\n\n[deps]\nserde = \"1\"\n";
    let new = "[package]\nname = \"a\"\n\n[deps]\nserde = \"1\"\ntoml = \"0.8\"\n";
    let hs = one("Config.toml", old, new);
    let h = hs.iter().find(|h| h.defines.contains(&"toml".to_string()));
    assert!(h.is_some(), "{hs:?}");
    assert!(
        h.unwrap()
            .enclosing
            .as_deref()
            .is_some_and(|e| e.starts_with("deps")),
        "{hs:?}"
    );
}

#[test]
fn changed_key_has_no_signature() {
    let hs = one(
        "Config.toml",
        "[package]\nversion = \"0.1.0\"\n",
        "[package]\nversion = \"0.2.0\"\n",
    );
    let h = hs
        .iter()
        .find(|h| h.defines.contains(&"version".to_string()));
    assert_eq!(h.map(|h| h.rationale.as_str()), Some("changes version"));
}

#[test]
fn gitconfig_is_ini_and_keeps_its_subsection_quotes() {
    let old = "[user]\n\tname = A\n[remote \"origin\"]\n\turl = git@old\n";
    let new = "[user]\n\tname = A\n\temail = a@b.c\n[remote \"origin\"]\n\turl = git@new\n";
    let hs = one(".gitconfig", old, new);
    let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert_eq!(enc, vec![Some("user.email"), Some("remote \"origin\".url")]);
    // the quotes are part of git's subsection name, not a quoted key
}

#[test]
fn a_local_variant_resolves_like_the_file_it_overrides() {
    // dvc quotes the whole section name; that pair *is* stripped
    let old = "[core]\n    remote = a\n['remote \"a\"']\n    url = s3://old\n";
    let new = "[core]\n    remote = a\n['remote \"a\"']\n    url = s3://new\n";
    for path in [".dvc/config", ".dvc/config.local"] {
        let hs = one(path, old, new);
        let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
        assert_eq!(enc, vec![Some("remote \"a\".url")], "{path}");
    }
}

#[test]
fn ini_by_extension() {
    for path in ["setup.cfg", "tox.ini", "pytest.ini"] {
        let hs = one(path, "[m]\nname = a\n", "[m]\nname = a\nversion = 1\n");
        let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
        assert_eq!(enc, vec![Some("m.version")], "{path}");
    }
}

#[test]
fn a_yaml_anchor_is_defined_and_its_alias_uses_it() {
    let old = "app:\n  name: x\n\nmiddle: 1\n\ndev:\n  db: dev\n";
    let new = "app:\n  name: x\n\nbase: &base\n  adapter: pg\n\nmiddle: 1\n\ndev:\n  db: dev\n  <<: *base\n";
    let out = run_json(serde_json::json!({
        "changes": [{ "path": "database.yml", "old": old, "new": new }]
    }));
    let hs: Vec<_> = out.files.iter().flat_map(|f| f.hunks.iter()).collect();
    let defines: Vec<&[String]> = hs.iter().map(|h| h.defines.as_slice()).collect();
    let uses: Vec<&[String]> = hs.iter().map(|h| h.uses.as_slice()).collect();
    assert_eq!(
        defines,
        vec![
            ["adapter".to_string(), "base".to_string()].as_slice(),
            ["dev".to_string()].as_slice()
        ]
    );
    assert_eq!(uses, vec![[].as_slice(), ["base".to_string()].as_slice()]);
    // the only def→use edge a config format can produce
    let why: Vec<&str> = out.edges.iter().map(|e| e.why.as_str()).collect();
    assert_eq!(why, vec!["def→use: base"]);
}

#[test]
fn the_yaml_merge_key_is_not_a_name() {
    let hs = one(
        "a.yml",
        "dev:\n  db: dev\n",
        "base: &base\n  a: 1\ndev:\n  db: dev\n  <<: *base\n",
    );
    assert!(
        !hs.iter().any(|h| h.defines.contains(&"<<".to_string())),
        "{hs:?}"
    );
}

#[test]
fn multi_document_yaml_scopes_each_document() {
    let doc = |port: &str, replicas: &str| {
        format!(
            "apiVersion: v1\nkind: Service\nmetadata:\n  name: web\nspec:\n  port: {port}\n---\n\
             apiVersion: v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: {replicas}\n"
        )
    };
    let hs = one("k8s.yaml", &doc("80", "1"), &doc("8080", "3"));
    // both objects own a `spec`; the path has to say which
    let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert_eq!(
        enc,
        vec![
            Some("kind=Service.spec.port"),
            Some("kind=Deployment.spec.replicas")
        ]
    );
}

#[test]
fn a_single_document_file_keeps_its_bare_paths() {
    let hs = one("one.yaml", "spec:\n  port: 80\n", "spec:\n  port: 8080\n");
    let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert_eq!(enc, vec![Some("spec.port")]);
}

#[test]
fn a_sibling_key_is_not_reported_as_a_member_of_its_neighbour() {
    // `two` sits beside `one`, not inside it — the detail layer must not
    // borrow the hunk's enclosing key as a parent for a top-level member
    for (path, old, new) in [
        ("a.yml", "one: 1\n", "one: 2\ntwo: 9\n"),
        (
            "a.json",
            "{\n  \"one\": 1\n}\n",
            "{\n  \"one\": 2,\n  \"two\": 9\n}\n",
        ),
    ] {
        let hs = one(path, old, new);
        let d: Vec<&String> = hs.iter().flat_map(|h| h.details.iter()).collect();
        assert!(!d.iter().any(|s| s.contains(" to one")), "{path}: {d:?}");
        assert_eq!(d, vec!["adds two", "changes one"], "{path}");
    }
}

#[test]
fn a_nested_key_still_names_its_parent() {
    // the counterpart to the sibling case: a key that really does sit inside
    // another must keep naming it. (A pure insert stays silent by design —
    // the rationale already says `adds port` — so this edits an existing key.)
    let hs = one(
        "a.yml",
        "svc:\n  image: a\n  port: 80\n",
        "svc:\n  image: b\n  port: 80\n",
    );
    assert!(
        hs.iter()
            .any(|h| h.details.contains(&"changes image in svc".to_string())),
        "{hs:?}"
    );
}

#[test]
fn a_value_has_no_signature_to_change() {
    // the wording rule is the *kind*, not the language: a cmake `set()` and a
    // make variable are values too, and neither is a `data` format
    for (path, old, new, want) in [
        ("x.cmake", "set(S a)\n", "set(S a b)\n", "changes S"),
        (
            "Makefile",
            "build: a.c\n\techo x\n",
            "build: a.c b.c\n\techo x\n",
            "changes build",
        ),
    ] {
        let hs = one(path, old, new);
        assert!(
            hs.iter().any(|h| h.rationale == want),
            "{path}: want {want:?}, got {hs:?}"
        );
    }
}

#[test]
fn a_callable_still_changes_its_signature() {
    let hs = one(
        "a.py",
        "def f(x):\n    return x\n",
        "def f(x, y):\n    return x\n",
    );
    assert!(
        hs.iter().any(|h| h.rationale == "changes signature of f"),
        "{hs:?}"
    );
}

#[test]
fn a_hash_comment_in_yaml_is_a_comment() {
    // the extension table used to fall through to the C-family default for
    // yaml, so `#` lines were not comments and the hunk was not comment-only
    let out = run_json(serde_json::json!({
        "changes": [ { "path": "a.yaml",
            "old": "svc:\n  # old note\n  port: 80\n",
            "new": "svc:\n  # new note\n  port: 80\n" } ]
    }));
    assert!(
        out.files[0].hunks.iter().all(|h| h.comment),
        "yaml # line is a comment: {:?}",
        out.files[0].hunks
    );
}

#[test]
fn a_key_repeated_under_another_parent_is_new_here() {
    // `enable` already exists under `linters`; the one under the new
    // `formatters` is added, not changed
    let hs = one(
        "a.yml",
        "linters:\n  enable:\n    - vet\nrun:\n  deadline: 5m\n",
        "linters:\n  enable:\n    - vet\nrun:\n  timeout: 5m\nformatters:\n  enable:\n    - gofmt\n",
    );
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert_eq!(rats, vec!["adds enable, formatters, timeout"]);
}

#[test]
fn keys_sharing_a_line_are_named_only_when_their_own_text_changed() {
    let hs = one(
        "libs.toml",
        "[libraries]\ncg = { module = \"io:cg\", version = \"1.0\" }\n",
        "[libraries]\ncg = { module = \"io:cg\", version = \"1.1\" }\n",
    );
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert!(!hs[0].rationale.contains("module"), "{}", hs[0].rationale);
    assert!(hs[0].rationale.contains("version"), "{}", hs[0].rationale);
}

#[test]
fn a_sibling_that_only_gained_a_trailing_comma_is_not_edited() {
    let hs = one(
        "en.json",
        "{\n  \"picker\": {\n    \"a\": \"A\"\n  }\n}\n",
        "{\n  \"picker\": {\n    \"a\": \"A\",\n    \"b\": \"B\"\n  }\n}\n",
    );
    let rats: Vec<&str> = hs.iter().map(|h| h.rationale.as_str()).collect();
    assert!(rats.iter().all(|r| !r.contains("edits a")), "{rats:?}");
    assert!(rats.iter().any(|r| r.contains("adds b")), "{rats:?}");
}

#[test]
fn a_yaml_document_keeps_its_name_when_another_is_inserted_before_it() {
    let doc = |name: &str, tag: &str| format!("---\nkind: build\nname: {name}\ntag: {tag}\n");
    let old = [doc("a", "1"), doc("b", "1")].concat();
    let new = [doc("new", "1"), doc("a", "1"), doc("b", "2")].concat();
    let hs = one("order.yaml", &old, &new);
    let enc: Vec<Option<&str>> = hs.iter().map(|h| h.enclosing.as_deref()).collect();
    assert!(enc.contains(&Some("name=b.tag")), "{hs:?}");
    assert!(hs.iter().all(|h| h.details.iter().all(|d| !d.contains("document"))), "{hs:?}");
}

#[test]
fn a_yaml_document_whose_naming_value_is_edited_changes_in_place() {
    let doc = |pkgs: &str| format!("---\nkind: pip\npackages: {pkgs}\n");
    let old = [doc("a b"), doc("c")].concat();
    let new = [doc("b"), doc("c")].concat();
    let hs = one("order.yaml", &old, &new);
    assert_eq!(hs[0].details, vec!["changes packages in packages=b".to_string()]);
}

#[test]
fn a_yaml_document_with_no_unique_pair_is_named_by_position() {
    let doc = |port: &str| format!("---\nkind: svc\nspec:\n  port: {port}\n");
    let hs = one("s.yaml", &[doc("1"), doc("1")].concat(), &[doc("1"), doc("2")].concat());
    assert_eq!(hs[0].enclosing.as_deref(), Some("document 2.spec.port"), "{hs:?}");
}
