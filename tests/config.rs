//! Config formats (json, yaml, toml): a key is a def named by its key text,
//! nested through a dotted path, and a member of the key above it. Covers key
//! naming per grammar, the nested path, the "changes k" wording (a key has no
//! signature), and the P15 detail layer naming the container key.
use ordo::model::{HunkOut, Input};

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .unwrap();
    ordo::run(inp)
        .files
        .into_iter()
        .flat_map(|f| f.hunks)
        .collect()
}

fn supported(path: &str, old: &str, new: &str) -> bool {
    let inp: Input = serde_json::from_value(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .unwrap();
    !ordo::run(inp).files[0].unsupported
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
    assert!(
        hs.iter()
            .any(|h| h.enclosing.as_deref() == Some("user.email")),
        "{hs:?}"
    );
    // the quotes are part of git's subsection name, not a quoted key
    assert!(
        hs.iter()
            .any(|h| h.enclosing.as_deref() == Some("remote \"origin\".url")),
        "{hs:?}"
    );
}

#[test]
fn a_local_variant_resolves_like_the_file_it_overrides() {
    // dvc quotes the whole section name; that pair *is* stripped
    let old = "[core]\n    remote = a\n['remote \"a\"']\n    url = s3://old\n";
    let new = "[core]\n    remote = a\n['remote \"a\"']\n    url = s3://new\n";
    for path in [".dvc/config", ".dvc/config.local"] {
        let hs = one(path, old, new);
        assert!(
            hs.iter()
                .any(|h| h.enclosing.as_deref() == Some("remote \"a\".url")),
            "{path}: {hs:?}"
        );
    }
}

#[test]
fn ini_by_extension() {
    for path in ["setup.cfg", "tox.ini", "pytest.ini"] {
        let hs = one(path, "[m]\nname = a\n", "[m]\nname = a\nversion = 1\n");
        assert!(
            hs.iter()
                .any(|h| h.enclosing.as_deref() == Some("m.version")),
            "{path}: {hs:?}"
        );
    }
}
