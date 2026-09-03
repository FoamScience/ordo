//! Jinja templates: a `.j2` over a format that has a grammar is analyzed as
//! that format (its `{% … %}` statements blanked out at unchanged offsets),
//! and its `{{ … }}` variables become uses that can link to wherever they are
//! actually set. A `.j2` over a format with no grammar is parsed as jinja.
use ordo::model::{HunkOut, Input, Output};

fn run(v: serde_json::Value) -> Output {
    ordo::run(serde_json::from_value::<Input>(v).unwrap())
}

fn one(path: &str, old: &str, new: &str) -> Vec<HunkOut> {
    run(serde_json::json!({
        "changes": [{ "path": path, "old": old, "new": new }]
    }))
    .files
    .into_iter()
    .flat_map(|f| f.hunks)
    .collect()
}

#[test]
fn a_statement_does_not_break_the_underlying_format() {
    // unmasked, one `{% for %}` makes the whole yaml document a parse error
    let old = "services:\n{% for s in services %}\n  {{ s.name }}:\n    port: 80\n{% endfor %}\n";
    let new = "services:\n{% for s in services %}\n  {{ s.name }}:\n    port: 8080\n{% endfor %}\n";
    let hs = one("app.yaml.j2", old, new);
    let h = hs.iter().find(|h| h.defines.contains(&"port".to_string()));
    assert!(h.is_some(), "{hs:?}");
    assert_eq!(
        h.unwrap().enclosing.as_deref(),
        Some("services.{{s.name}}.port"),
        "{hs:?}"
    );
}

#[test]
fn template_extensions_resolve_to_the_inner_format() {
    for path in ["a.toml.j2", "a.toml.jinja", "a.toml.jinja2", "a.toml.tmpl"] {
        let hs = one(path, "[s]\nport = 80\n", "[s]\nport = 8080\n");
        assert!(
            hs.iter().any(|h| h.enclosing.as_deref() == Some("s.port")),
            "{path}: {hs:?}"
        );
    }
}

#[test]
fn an_interpolated_variable_is_a_use_of_what_defines_it() {
    let out = run(serde_json::json!({
        "options": {"cross_file": true},
        "changes": [
            { "path": "templates/app.yml.j2",
              "old": "server:\n  host: {{ db_host }}\n",
              "new": "server:\n  host: {{ db_host }}\n  port: {{ db_port }}\n" },
            { "path": "group_vars/all.yml",
              "old": "db_host: a\n",
              "new": "db_host: a\ndb_port: 5432\n" },
        ]
    }));
    let tmpl = &out.files[0].hunks[0];
    assert!(tmpl.uses.contains(&"db_port".to_string()), "{tmpl:?}");
    assert!(
        out.edges.iter().any(|e| e.why.contains("db_port")),
        "{:?}",
        out.edges
    );
}

#[test]
fn a_bare_template_is_parsed_as_jinja() {
    let old = "{% block server %}\nlisten 80;\n{% endblock %}\n";
    let new = "{% include 'tls.j2' %}\n{% block server %}\nlisten 443;\n{% endblock %}\n\
               {% macro upstream(name) %}{{ name }}{% endmacro %}\n";
    let hs = one("templates/nginx.conf.j2", old, new);
    assert!(
        hs.iter().any(|h| h.rationale == "adds import tls.j2"),
        "{hs:?}"
    );
    assert!(
        hs.iter().any(|h| h.enclosing.as_deref() == Some("server")),
        "{hs:?}"
    );
    let mac = hs
        .iter()
        .find(|h| h.defines.contains(&"upstream".to_string()));
    assert!(mac.is_some(), "{hs:?}");
    // a macro's own name and parameters are bindings, not uses of themselves
    assert!(mac.unwrap().uses.is_empty(), "{mac:?}");
}

#[test]
fn a_template_over_a_filename_matched_format_still_resolves() {
    let old = "[core]\n    remote = a\n['remote \"a\"']\n    url = s3://old\n";
    let new = "[core]\n    remote = a\n{% if prod %}\n['remote \"a\"']\n    url = s3://new\n\
               {% endif %}\n";
    for path in [
        ".dvc/config.j2",
        ".dvc/config.local.j2",
        ".gitconfig.j2",
        "setup.cfg.j2",
    ] {
        let hs = one(path, old, new);
        assert!(
            hs.iter()
                .any(|h| h.enclosing.as_deref() == Some("remote \"a\".url")),
            "{path}: {hs:?}"
        );
    }
}

#[test]
fn a_jinja_only_hunk_is_not_formatting_noise() {
    // the added `{% if %}` guard is blank to the ini grammar, but it is the
    // substance of the change, not its formatting
    let old = "[core]\n    remote = a\n";
    let new = "[core]\n{% if prod %}\n    remote = a\n{% endif %}\n";
    let hs = one(".dvc/config.j2", old, new);
    let h = hs.iter().find(|h| h.uses.contains(&"prod".to_string()));
    assert!(h.is_some(), "{hs:?}");
    assert_ne!(h.unwrap().rationale, "formatting only", "{hs:?}");
}
