// ------------------------------------------------------ generated doc blocks

use super::*;

/// Whether a `UPDATE_*` escape hatch is actually switched on. Testing
/// `is_ok()` meant any value armed it, so `UPDATE_DOCS=0` rewrote the
/// generated blocks and asserted nothing while still reporting a pass.
fn update_requested(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false" | "no"))
}

/// Every file that carries generated blocks, relative to the crate root.
/// Kept in step with `.gitattributes` by a test below.
const FILES: &[&str] = &[
    "README.md",
    "docs/cli.md",
    "docs/languages.md",
    "docs/reviewing.md",
    "docs/rules.md",
    "docs/tui.md",
];

/// Wraps a comma-separated list at `width` columns, so a regenerated list
/// is stable rather than one very long line.
fn wrap(items: &[String], width: usize) -> String {
    let mut lines: Vec<String> = vec![String::new()];
    for (i, item) in items.iter().enumerate() {
        let sep = if i + 1 == items.len() { "" } else { "," };
        let last = lines.last_mut().unwrap();
        if last.is_empty() {
            *last = format!("{item}{sep}");
        } else if last.chars().count() + 1 + item.len() + sep.len() > width {
            lines.push(format!("{item}{sep}"));
        } else {
            last.push_str(&format!(" {item}{sep}"));
        }
    }
    lines.join("\n")
}

/// Escapes a generated table cell: `<glob>` is an HTML tag to a markdown
/// renderer, and a bare `|` ends the cell.
fn cell(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
}

fn langs() -> Vec<&'static str> {
    ordo::languages().into_iter().map(|(n, _)| n).collect()
}

/// The `| | vim | vscode |` table: one row per action, grouped by the same
/// categories the `?` popup uses, with each preset's keys for it. Built
/// from `Keymap.binds`, so it cannot name a key the table doesn't bind.
fn keys_table() -> String {
    let maps: Vec<Keymap> = ["vim", "vscode"]
        .iter()
        .map(|n| keymap(n).unwrap())
        .collect();
    // (category, desc) in first-seen order, vim first so the common
    // reading order is the default preset's
    let mut rows: Vec<(Category, &'static str)> = vec![];
    for m in &maps {
        for &(_, _, action) in &m.binds {
            let row = action_help(action);
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    let keys_for = |m: &Keymap, desc: &str| {
        let mut out: Vec<String> = vec![];
        for &(prefix, key, action) in &m.binds {
            if action_help(action).1 != desc {
                continue;
            }
            let label = format!("`{}`", chord_label(prefix, key));
            if !out.contains(&label) {
                out.push(label);
            }
        }
        out.join(", ")
    };
    let order = [
        Category::General,
        Category::Navigation,
        Category::Panes,
        Category::Search,
        Category::Review,
        Category::Editor,
        Category::Help,
    ];
    let mut out = vec![
        "| | `vim` (default) | `vscode` |".to_string(),
        "| --- | --- | --- |".to_string(),
    ];
    for cat in order {
        let group: Vec<_> = rows.iter().filter(|(c, _)| *c == cat).collect();
        if group.is_empty() {
            continue;
        }
        out.push(format!("| **{}** | | |", category_label(cat)));
        for (_, desc) in group {
            let cells: Vec<String> = maps.iter().map(|m| keys_for(m, desc)).collect();
            out.push(format!("| {} | {} |", cell(desc), cells.join(" | ")));
        }
    }
    out.join("\n")
}

/// The detail layer's member node kinds, one row per distinct member set —
/// the languages that read members the same way share a row, as they did
/// when this table was written by hand.
fn members_table() -> String {
    let mut groups: Vec<(Vec<&'static str>, &'static [&'static str])> = vec![];
    for (name, members) in ordo::languages() {
        if members.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(_, m)| *m == members) {
            Some((names, _)) => names.push(name),
            None => groups.push((vec![name], members)),
        }
    }
    let mut out = vec![
        "| language | member node kinds |".to_string(),
        "| --- | --- |".to_string(),
    ];
    for (names, members) in groups {
        let kinds: Vec<String> = members.iter().map(|k| format!("`{k}`")).collect();
        out.push(format!("| {} | {} |", names.join(" / "), kinds.join(", ")));
    }
    out.join("\n")
}

/// The bundled rulesets, sourced from `PRESETS` — the name a config file
/// writes in `include`, and the first comment line of the file itself,
/// which is where each ruleset already states what it is. That first line
/// is therefore a contract: it has to stand on its own.
fn rulesets_table() -> String {
    let mut out = vec![
        "| preset | source |".to_string(),
        "| --- | --- |".to_string(),
    ];
    for (name, text) in PRESETS {
        let source = text
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches('#')
            .trim();
        out.push(format!("| `{name}` | {} |", cell(source)));
    }
    out.join("\n")
}

fn commands_table() -> String {
    let mut out = vec![
        "| command | does |".to_string(),
        "| --- | --- |".to_string(),
    ];
    for c in COMMANDS {
        let head = if c.args.is_empty() {
            format!(":{}", c.name)
        } else {
            format!(":{} {}", c.name, c.args)
        };
        out.push(format!("| `{head}` | {} |", cell(c.help)));
    }
    out.join("\n")
}

/// The `[[rule]]` keys the TOML surface actually accepts, kebab-cased.
/// Read out of `RuleToml` itself: `deny_unknown_fields` makes serde list
/// every expected field when it rejects one, which is a cheaper source of
/// truth than a second hand-kept list.
fn rule_toml_keys() -> Vec<String> {
    let err = match toml::from_str::<RuleToml>("name = 'x'\nordo-not-a-key = 1") {
        Err(e) => e.to_string(),
        Ok(_) => panic!("RuleToml no longer rejects an unknown key"),
    };
    let (_, list) = err
        .split_once("expected one of ")
        .expect("serde no longer lists the expected fields — teach this fn the new wording");
    let keys: Vec<String> = list
        .split(", ")
        .filter_map(|s| s.trim().trim_start_matches('`').split('`').next())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .map(str::to_string)
        .collect();
    assert!(
        keys.contains(&"name".to_string()),
        "parsed nothing useful from: {err}"
    );
    keys
}

/// What each `[[rule]]` key means, and whether it is a condition (the
/// first table in `docs/rules.md`) or an action (the second). The prose is
/// hand-written; the *set* of keys is checked against `RuleToml`, so a
/// condition added to the rules engine and left undocumented fails a test.
const RULE_DOC: &[(&str, bool, &str)] = &[
    ("path", true, "glob against the file path"),
    (
        "path-not",
        true,
        "glob the file path must *not* match — third-party code, a framework carve-out",
    ),
    (
        "test",
        true,
        "whether the file is a test (`tests/`, `test_*`, `*_spec.*`, …) — `test = false` is how a rule says production code only",
    ),
    (
        "lang",
        true,
        "`python`, `cpp`, `markdown`, … as `src/lang.rs` names them",
    ),
    ("category", true, "`import` · `definition` · `other`"),
    (
        "enclosing-kind",
        true,
        "what holds the hunk — `none` (nothing does) · `definition` · the region kinds in [cli.md](cli.md)",
    ),
    ("defines", true, "glob against any name the hunk defines"),
    ("uses", true, "glob against any name the hunk uses"),
    ("imports", true, "glob against any name the hunk imports"),
    (
        "noise-when",
        true,
        "the engine's own noise classification (`true` / `false`)",
    ),
    ("comment", true, "the hunk is comment/docstring-only"),
    ("query", true, "a tree-sitter query, inline (below)"),
    (
        "query-file",
        true,
        "a tree-sitter query, read from a file relative to the rules file",
    ),
    ("kind", true, "a node kind the hunk introduces (below)"),
    ("with", true, "…whose direct children include each of these"),
    (
        "without",
        true,
        "…and none of these — absence, as a table entry",
    ),
    ("text", true, "…and whose text matches this regex"),
    (
        "text-not",
        true,
        "…and whose text does not match this regex",
    ),
    (
        "max-params",
        true,
        "a definition the hunk introduces takes more parameters (below)",
    ),
    ("max-lines", true, "…is longer than this"),
    (
        "max-nesting",
        true,
        "…sits deeper in control flow than this",
    ),
    (
        "max-file-lines",
        true,
        "this change pushed the file past this many lines",
    ),
    (
        "recursive",
        true,
        "a definition starting in the hunk calls itself",
    ),
    (
        "container-with",
        true,
        "glob against the members of the container the hunk defines into (below)",
    ),
    ("container-without", true, "…the same, negated"),
    (
        "member-uninitialized",
        true,
        "the hunk adds a data member nothing in this change initializes",
    ),
    (
        "name",
        false,
        "how the rule identifies itself in the review — required",
    ),
    ("note", false, "says something on the hunk"),
    (
        "warn",
        false,
        "says it at warning level — `⚠` in the reading order",
    ),
    (
        "verdict",
        false,
        "asserts a concrete downgrade rather than an FYI — the level the construct catalog uses when a signal backs the call",
    ),
    (
        "noise",
        false,
        "marks the hunk skippable, like generated code",
    ),
    (
        "priority",
        false,
        "sorts it earlier (see the guarantee below)",
    ),
];

fn rule_table(conditions: bool) -> String {
    let mut out = vec![
        format!("| key | {} |", if conditions { "matches" } else { "does" }),
        "| --- | --- |".to_string(),
    ];
    for (key, is_cond, meaning) in RULE_DOC {
        if *is_cond == conditions {
            out.push(format!("| `{key}` | {meaning} |"));
        }
    }
    out.join("\n")
}

/// The `enclosing_kind` table. The key is `ContainerKind`'s own serde name
/// and the match is exhaustive, so a new container kind cannot be added
/// without this table gaining a row.
fn container_kinds_table() -> String {
    use ordo::model::ContainerKind::{self, *};
    const ALL: &[ContainerKind] = ContainerKind::ALL;
    let describe = |k: ContainerKind| -> (&'static str, &'static str) {
        match k {
            Definition => ("a definition — a function, class, macro, …", "`parse_cfg`"),
            Test => (
                "a named block: `describe`/`it`/`test`, or a rust test macro",
                "`describe \"cli\" > it \"parses flags\"`",
            ),
            Region => ("conditional compilation", "`#ifdef CURL_DISABLE_HTTP`"),
            Namespace => ("a namespace: it qualifies what it holds", "`particode`"),
            Preamble => ("prose before a document's first heading", "`preamble`"),
            FrontMatter => ("a document's `---` metadata block", "`front matter`"),
            Document => (
                "one `---` document of a multi-document yaml file",
                "`document 2`",
            ),
            Binding => (
                "a file-scope binding whose multi-line value holds the hunk",
                "`ALLOWED_IMPORTS`",
            ),
            Call => (
                "a file-scope call whose multi-line arguments hold it",
                "`execa('unicorns')`",
            ),
        }
    };
    let mut out = vec![
        "| `enclosing_kind` | what holds the hunk | example `enclosing` |".to_string(),
        "| --- | --- | --- |".to_string(),
    ];
    for &k in ALL {
        let key = match serde_json::to_value(k).unwrap() {
            serde_json::Value::String(s) => s,
            v => panic!("ContainerKind serialized as {v:?}"),
        };
        // omitted on the wire for a plain definition — the common case
        let shown = if k == Definition {
            "*(omitted)*".to_string()
        } else {
            format!("`{key}`")
        };
        let (what, example) = describe(k);
        out.push(format!("| {shown} | {what} | {example} |"));
    }
    out.join("\n")
}

/// The body for one block key, or `None` when the key isn't one we render
/// — an unknown key in a document is a typo, and fails the check rather
/// than silently leaving stale text in place.
fn render(key: &str) -> Option<String> {
    Some(match key {
        "langs" => {
            let names: Vec<String> = langs().iter().map(|n| n.to_string()).collect();
            wrap(&names, 76)
        }
        "langs-badge" => format!(
            "<img src=\"https://img.shields.io/badge/languages-{n}-5fd4c0\" \
             alt=\"{n} supported languages\">",
            n = langs().len()
        ),
        "members" => members_table(),
        "keys" => keys_table(),
        "commands" => commands_table(),
        "themes" => {
            let names: Vec<String> = theme_names().iter().map(|n| format!("`{n}`")).collect();
            wrap(&names, 76)
        }
        "theme-roles" => {
            let names: Vec<String> = THEME_ROLES.iter().map(|r| format!("`{r}`")).collect();
            wrap(&names, 76)
        }
        "rulesets" => rulesets_table(),
        "rule-conditions" => rule_table(true),
        "rule-actions" => rule_table(false),
        "container-kinds" => container_kinds_table(),
        _ => return None,
    })
}

/// Every key `render` knows, so an orphaned generator — one no document
/// still asks for — is caught too.
const KEYS: &[&str] = &[
    "langs",
    "langs-badge",
    "members",
    "keys",
    "commands",
    "themes",
    "theme-roles",
    "rulesets",
    "rule-conditions",
    "rule-actions",
    "container-kinds",
];

/// Rewrites every marked block of `text`, returning the new text and the
/// keys it filled. The indent of the opening marker is applied to each
/// generated line, so a block inside indented HTML stays aligned.
fn splice(text: &str, path: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut rest = text;
    let mut seen = vec![];
    while let Some(i) = rest.find("<!-- ordo:begin ") {
        let line_start = rest[..i].rfind('\n').map(|n| n + 1).unwrap_or(0);
        let indent = &rest[line_start..i];
        let after = &rest[i..];
        let key = after["<!-- ordo:begin ".len()..]
            .split(' ')
            .next()
            .unwrap_or_default()
            .to_string();
        let open_end = i + after.find("-->").expect("unterminated begin marker") + 3;
        let close = format!("{indent}<!-- ordo:end {key} -->");
        let close_at = rest[open_end..]
            .find(&close)
            .unwrap_or_else(|| panic!("{path}: no matching end marker for '{key}'"))
            + open_end;
        let body = render(&key).unwrap_or_else(|| panic!("{path}: unknown block '{key}'"));
        out.push_str(&rest[..open_end]);
        out.push('\n');
        for line in body.lines() {
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str(&format!("{indent}{line}\n"));
            }
        }
        out.push_str(indent);
        seen.push(key);
        rest = &rest[close_at + indent.len()..];
    }
    out.push_str(rest);
    (out, seen)
}

/// `src/catalog.generated.json` is generated from `rulesets/catalog/*.toml`.
///
/// The engine reads no files, so the catalog has to be compiled in; but a
/// catalog authored as JSON is a catalog nobody edits. The TOML is the
/// source, this is the compiler, and the drift check is the guarantee they
/// agree. Regenerate with `UPDATE_CATALOG_JSON=1`.
fn build_catalog_json() -> (String, Vec<String>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("rulesets/catalog");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("rulesets/catalog")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    #[derive(serde::Serialize)]
    struct Compiled {
        name: String,
        rules: Vec<ordo::model::Rule>,
    }
    let mut out: Vec<Compiled> = vec![];
    let mut problems = vec![];
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read catalog file");
        let doc = parse_rules_doc(&text, &root);
        problems.extend(
            doc.problems
                .into_iter()
                .map(|p| format!("{}: {p}", f.display())),
        );
        // the file stem is the section a reviewer turns off as a unit
        out.push(Compiled {
            name: f
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            rules: doc.rules,
        });
    }
    (
        serde_json::to_string_pretty(&out).expect("serialize catalog"),
        problems,
    )
}

#[test]
fn the_compiled_catalog_matches_the_toml_it_is_built_from() {
    let (got, problems) = build_catalog_json();
    assert!(problems.is_empty(), "catalog does not parse: {problems:?}");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/catalog.generated.json");
    if update_requested("UPDATE_CATALOG_JSON") {
        std::fs::write(&path, format!("{got}\n")).expect("write catalog.generated.json");
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got.trim(),
        want.trim(),
        "src/catalog.generated.json is stale; regenerate with UPDATE_CATALOG_JSON=1"
    );
}

/// Every catalog rule has to fire somewhere, or the catalog quietly shrank:
/// a query that stops compiling, or a `kind` a grammar renamed, looks
/// exactly like a construct nobody writes.
#[test]
fn every_catalog_rule_compiles_for_the_language_it_names() {
    let engine = ordo::rules::Rules::with_catalog(ordo::catalog::rules(), &[]);
    assert!(
        engine.catalog_problems.is_empty(),
        "{:?}",
        engine.catalog_problems
    );
    assert!(engine.problems.is_empty(), "{:?}", engine.problems);
    assert!(
        ordo::catalog::problem().is_none(),
        "{:?}",
        ordo::catalog::problem()
    );
}

#[test]
fn generated_blocks_match_the_code_that_owns_them() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let update = update_requested("UPDATE_DOCS");
    let mut stale = vec![];
    let mut seen: Vec<String> = vec![];
    for rel in FILES {
        let path = root.join(rel);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let (new, keys) = splice(&text, rel);
        seen.extend(keys);
        if new == text {
            continue;
        }
        if update {
            std::fs::write(&path, new).unwrap();
        } else {
            stale.push(*rel);
        }
    }
    for key in KEYS {
        assert!(
            seen.iter().any(|k| k == key),
            "block '{key}' is generated but no document asks for it"
        );
    }
    assert!(
        stale.is_empty(),
        "generated doc blocks are stale in {stale:?} — run `UPDATE_DOCS=1 cargo test`"
    );
}

/// The `[[rule]]` condition key `k` as `model::When` spells it, or `None`
/// when it is a client-only key with no engine counterpart.
fn when_field_of(k: &str) -> Option<String> {
    match k {
        // the engine reads no files; this one resolves to `query` before
        // the rule is handed over
        "query-file" => None,
        // flattening the nested model collided with the rule-level
        // `noise`, so the TOML spells the condition differently
        "noise-when" => Some("noise".to_string()),
        other => Some(other.replace('-', "_")),
    }
}

/// Does `model::When` have a field by this name? Asked by deserializing a
/// one-key object and checking nothing landed in the catch-all — the same
/// technique `tests/schema.rs` uses against the published schema.
fn when_accepts(field: &str) -> bool {
    for v in [
        serde_json::json!("x"),
        serde_json::json!(true),
        serde_json::json!(1),
        serde_json::json!([]),
        serde_json::json!("import"),
    ] {
        if let Ok(w) = serde_json::from_value::<ordo::model::When>(serde_json::json!({ field: v }))
        {
            if w.unknown.is_empty() {
                return true;
            }
        }
    }
    false
}

/// `RuleToml` is a second spelling of the engine's rule schema — the TOML
/// surface is flat and kebab-cased while `model::Rule` nests its conditions
/// under `when`. That copy is deliberate: `deny_unknown_fields` on the flat
/// struct is what makes a typo in a `rules.toml` a parse error naming the
/// offending line and listing every valid key, which a `#[serde(flatten)]`
/// of `When` cannot do (it reports the wrong line and drops the list).
///
/// The copy being deliberate does not make drift acceptable. Every
/// condition the TOML accepts has to be one the engine reads, or it is a
/// documented option that parses and then does nothing — which is exactly
/// how `when.rules` reached the published schema (tasks-9sj.13).
#[test]
fn every_toml_condition_is_one_the_engine_reads() {
    let conditions: Vec<&str> = RULE_DOC
        .iter()
        .filter(|(_, is_cond, _)| *is_cond)
        .map(|(k, _, _)| *k)
        .collect();
    assert!(conditions.len() > 15, "RULE_DOC lost its conditions");

    for key in &conditions {
        let Some(field) = when_field_of(key) else {
            continue; // client-only, by the table above
        };
        assert!(
            when_accepts(&field),
            "`{key}` is a documented [[rule]] condition, but `model::When` \
             has no `{field}` — it would parse and then never match"
        );
    }
}

/// And the reverse: a condition the engine reads that the TOML cannot
/// express is a feature no rule author can reach.
#[test]
fn every_engine_condition_is_reachable_from_toml() {
    let documented: Vec<String> = RULE_DOC
        .iter()
        .filter(|(_, is_cond, _)| *is_cond)
        .filter_map(|(k, _, _)| when_field_of(k))
        .collect();
    // read the engine's own list off the published schema, which
    // tests/schema.rs already pins to `model::When`
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/v1.json"),
        )
        .expect("schema/v1.json"),
    )
    .expect("valid json");
    let when = schema["$defs"]["input"]["properties"]["options"]["properties"]["rules"]["items"]
        ["properties"]["when"]["properties"]
        .as_object()
        .expect("when properties");
    for field in when.keys() {
        assert!(
            documented.contains(field),
            "the engine reads `when.{field}`, but no [[rule]] key reaches it"
        );
    }
}

/// `RULE_DOC` is prose, but its *keys* are not allowed to be an opinion: a
/// condition the rules engine accepts and this table omits is a feature
/// nobody can find, and a key here that `RuleToml` rejects is a documented
/// option that silently fails to parse.
#[test]
fn every_rule_key_is_documented_exactly_once() {
    let real = rule_toml_keys();
    let documented: Vec<&str> = RULE_DOC.iter().map(|(k, _, _)| *k).collect();
    for key in &real {
        assert!(
            documented.contains(&key.as_str()),
            "`{key}` is a [[rule]] key but no row in RULE_DOC describes it"
        );
    }
    for key in &documented {
        assert!(
            real.contains(&key.to_string()),
            "RULE_DOC documents `{key}`, which RuleToml does not accept"
        );
        assert_eq!(
            documented.iter().filter(|k| k == &key).count(),
            1,
            "`{key}` is documented twice"
        );
    }
}

/// `ordo help <topic>` is only useful if it can reach every page, and only
/// honest if every page it names exists. Design notes (`*-design.md`) are
/// deliberately not topics: they record how a decision was reached, which
/// is not what someone at a prompt is asking for.
#[test]
fn every_documentation_page_is_a_help_topic() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pages: Vec<String> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".md") && !n.ends_with("-design.md"))
        .map(|n| n.trim_end_matches(".md").to_string())
        .collect();
    pages.sort();
    let mut topics: Vec<String> = TOPICS.iter().map(|(n, _, _)| n.to_string()).collect();
    topics.sort();
    assert_eq!(
        pages, topics,
        "docs/ and TOPICS disagree — a page nobody can reach, or a topic with no page"
    );
    for (name, blurb, body) in TOPICS {
        assert!(!blurb.is_empty(), "{name} has no one-line description");
        assert!(
            body.starts_with("# "),
            "{name} does not open with a heading"
        );
        assert!(
            topic_list().contains(name),
            "{name} is missing from the topic list"
        );
    }
}

#[test]
fn an_unknown_help_topic_is_a_usage_error() {
    assert_eq!(help_topic(Some("no-such-topic")), 2);
}

/// The `.gitattributes` list and `FILES` are two statements of the same
/// fact; a document that gains a generated block and is not marked, or is
/// marked and no longer has one, is a drift of its own.
#[test]
fn gitattributes_marks_every_generated_document() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let attrs = std::fs::read_to_string(root.join(".gitattributes")).unwrap();
    let marked: Vec<&str> = attrs
        .lines()
        .filter(|l| l.contains("ordo-generated=true"))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    for f in FILES {
        assert!(
            marked.contains(f),
            "{f} carries generated blocks but .gitattributes does not mark it"
        );
    }
    for m in &marked {
        assert!(
            FILES.contains(m),
            ".gitattributes marks {m} as generated, but it carries no blocks"
        );
    }
}
