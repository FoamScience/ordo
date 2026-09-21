// ------------------------------------------------------------ reviewing rules
use crate::code_view::theme;
use crate::code_view::theme_names;
use crate::config::config_path;
use crate::config::key_label;
use crate::keys::action_help;
use crate::keys::keymap;
use crate::keys::theme_role_color;
use crate::keys::ACTION_NAMES;
use crate::keys::THEME_ROLES;
use ratatui::style::Color;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

/// Where rules come from, in the order they are read: this user's own, then the
/// repository's. Both apply — a personal preference and a team convention are
/// different things, and a reviewer wants both. The repo's file is read last so
/// its rules are reported after the user's on a hunk they both match.
///
/// Rule *files* are the client's business: the engine reads nothing (see
/// `ordo::model::Options::rules`), which is what keeps `ordo order --json` a
/// function of its arguments and the corpus tests meaningful.
/// The rulesets shipped with ordo, bundled so `include = ["go-uber-guide"]`
/// (or `--rules go-uber-guide`) needs no path. `rulesets/` is the source of
/// truth; a test checks every file there is listed here.
pub(super) const PRESETS: &[(&str, &str)] = &[
    (
        "c-power-of-ten",
        include_str!("../../../rulesets/c-power-of-ten.toml"),
    ),
    (
        "cpp-default-guidelines",
        include_str!("../../../rulesets/cpp-default-guidelines.toml"),
    ),
    (
        "go-uber-guide",
        include_str!("../../../rulesets/go-uber-guide.toml"),
    ),
    (
        "java-effective-java",
        include_str!("../../../rulesets/java-effective-java.toml"),
    ),
    (
        "javascript-airbnb",
        include_str!("../../../rulesets/javascript-airbnb.toml"),
    ),
    (
        "lua-style-guide",
        include_str!("../../../rulesets/lua-style-guide.toml"),
    ),
    ("markdown", include_str!("../../../rulesets/markdown.toml")),
    (
        "python-google-style",
        include_str!("../../../rulesets/python-google-style.toml"),
    ),
    (
        "rust-api-guidelines",
        include_str!("../../../rulesets/rust-api-guidelines.toml"),
    ),
    (
        "typescript-clean-code",
        include_str!("../../../rulesets/typescript-clean-code.toml"),
    ),
];

pub(super) fn preset(name: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The user's own rules file — where `:config` writes a catalog or ruleset
/// change, since it is the one that applies to every repository.
pub(super) fn user_rules_path() -> Option<PathBuf> {
    config_path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .map(|d| d.join("rules.toml"))
}

pub(super) fn rule_sources(repo_root: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(dir) = config_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
        out.push(dir.join("rules.toml"));
    }
    if !repo_root.is_empty() {
        out.push(PathBuf::from(repo_root).join(".ordo").join("rules.toml"));
    }
    out
}

/// Everything `load_rules_report` learned: the rules to run, and the record a
/// reviewer needs to trust them — where each came from, which definitions
/// replaced an earlier one, which names were disabled. A silenced rule looks
/// exactly like a convention nobody breaks, so the silencing is shown.
/// The three rule facts that always travel together: what to run, whether the
/// built-in catalog runs beside them, and what to silence in either.
#[derive(Clone, Default)]
pub(super) struct RuleSet {
    pub(super) rules: Vec<ordo::model::Rule>,
    pub(super) catalog: bool,
    pub(super) disables: Vec<String>,
    pub(super) includes: Vec<String>,
}

pub(super) struct RulesReport {
    pub(super) rules: Vec<ordo::model::Rule>,
    pub(super) problems: Vec<String>,
    /// (origin, active rules from it), in load order
    pub(super) origins: Vec<(String, usize)>,
    pub(super) replaced: Vec<String>,
    pub(super) disabled: Vec<String>,
    /// false when any rules file said `catalog = false`
    pub(super) catalog: bool,
    /// bundled rulesets named by `include`
    includes: Vec<String>,
    /// the `disable` globs themselves, forwarded to the engine so they reach
    /// the construct catalog too — a catalog entry is a rule, and silencing it
    /// by name is the same gesture
    disables: Vec<String>,
}

impl RulesReport {
    pub(super) fn rule_set(&self, catalog: bool) -> RuleSet {
        RuleSet {
            rules: self.rules.clone(),
            catalog,
            disables: self.disables.clone(),
            includes: self.includes.clone(),
        }
    }

    /// The `:rules` popup, one line per fact.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{} rule{} active",
            self.rules.len(),
            if self.rules.len() == 1 { "" } else { "s" }
        )];
        for (origin, n) in &self.origins {
            out.push(format!("  {n:>3}  {origin}"));
        }
        if !self.replaced.is_empty() {
            out.push(String::new());
            out.push("replaced (a later definition with the same name):".to_string());
            out.extend(self.replaced.iter().map(|r| format!("  {r}")));
        }
        if !self.disabled.is_empty() {
            out.push(String::new());
            out.push("disabled:".to_string());
            out.extend(self.disabled.iter().map(|d| format!("  {d}")));
        }
        if !self.problems.is_empty() {
            out.push(String::new());
            out.push("problems:".to_string());
            out.extend(self.problems.iter().map(|p| format!("  {p}")));
        }
        if self.rules.is_empty() && self.origins.is_empty() {
            out.push(String::new());
            out.push("no rules loaded — `include = [\"go-uber-guide\"]` in .ordo/rules.toml, or --rules <preset|file>".to_string());
        }
        out
    }
}

/// Merge one parsed rules document into the layered list: its `include`s first
/// (a bundled preset by name, or a path relative to the file), then its own
/// rules, where a name already present is *replaced* in place. Disables are
/// only collected here; they apply once everything is layered, so a user can
/// silence a rule the repo includes and the repo one a user includes.
#[allow(clippy::too_many_arguments)]
/// What layering a rules file accumulates: every rule kept and where it came
/// from, every `disable` glob, every name a later file replaced, whether the
/// catalog was switched off, and anything that went wrong.
#[derive(Default)]
struct Layering {
    layered: Vec<(ordo::model::Rule, String)>,
    disables: Vec<String>,
    replaced: Vec<String>,
    problems: Vec<String>,
    /// `None` until a file says; `Some(false)` turns the catalog off
    catalog: Option<bool>,
    /// bundled rulesets pulled in by `include`
    includes: Vec<String>,
}

fn layer_rules(text: &str, origin: &str, base: &Path, depth: usize, acc: &mut Layering) {
    if depth > 8 {
        acc.problems.push(format!(
            "{origin}: include nesting deeper than 8 — a cycle?"
        ));
        return;
    }
    let doc = parse_rules_doc(text, base);
    for p in doc.problems {
        acc.problems.push(format!("{origin}: {p}"));
    }
    for inc in &doc.include {
        if let Some(t) = preset(inc) {
            acc.includes.push(inc.clone());
            layer_rules(t, inc, Path::new("."), depth + 1, acc);
        } else {
            let path = base.join(inc);
            match std::fs::read_to_string(&path) {
                Ok(t) => {
                    let label = path.display().to_string();
                    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    layer_rules(&t, &label, &parent, depth + 1, acc);
                }
                Err(e) => acc.problems.push(format!("{origin}: include `{inc}`: {e}")),
            }
        }
    }
    acc.disables.extend(doc.disable);
    if let Some(c) = doc.catalog {
        // any file saying no wins: the switch is off, not voted on
        acc.catalog = Some(acc.catalog.unwrap_or(true) && c);
    }
    for rule in doc.rules {
        match acc.layered.iter().position(|(r, _)| r.name == rule.name) {
            Some(i) => {
                acc.replaced
                    .push(format!("{}  ({} → {origin})", rule.name, acc.layered[i].1));
                acc.layered[i] = (rule, origin.to_string());
            }
            None => acc.layered.push((rule, origin.to_string())),
        }
    }
}

/// Read the rule files that exist — this user's, then this repository's, then
/// any `--rules` file or preset — layer them, then apply every `disable`.
pub(super) fn load_rules_report(repo_root: &str, extra: &[String]) -> RulesReport {
    report_from(rule_sources(repo_root), extra)
}

/// `implicit` sources (the user's and the repo's files) may be absent; every
/// `extra` — a `--rules` argument — was asked for, so its absence is reported,
/// unless it names a bundled preset.
pub(super) fn report_from(implicit: Vec<PathBuf>, extra: &[String]) -> RulesReport {
    let mut acc = Layering::default();
    let n_implicit = implicit.len();
    for (i, src) in implicit
        .into_iter()
        .chain(extra.iter().map(PathBuf::from))
        .enumerate()
    {
        let implicit = i < n_implicit;
        let name = src.to_string_lossy().into_owned();
        if !implicit && !src.exists() {
            if let Some(t) = preset(&name) {
                layer_rules(t, &name, Path::new("."), 0, &mut acc);
                continue;
            }
        }
        let text = match std::fs::read_to_string(&src) {
            Ok(t) => t,
            Err(_) if implicit => continue,
            Err(e) => {
                acc.problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let base = src.parent().unwrap_or(Path::new(".")).to_path_buf();
        layer_rules(&text, &name, &base, 0, &mut acc);
    }
    // disables win, whoever wrote them
    let mut set = globset::GlobSetBuilder::new();
    for d in &acc.disables {
        match globset::Glob::new(d) {
            Ok(g) => {
                set.add(g);
            }
            Err(e) => acc
                .problems
                .push(format!("disable `{d}` is not a glob: {e}")),
        }
    }
    let set = set.build().unwrap_or_else(|_| globset::GlobSet::empty());
    let mut disabled = vec![];
    acc.layered.retain(|(r, origin)| {
        let keep = !set.is_match(&r.name);
        if !keep {
            disabled.push(format!("{}  ({origin})", r.name));
        }
        keep
    });
    let mut origins: Vec<(String, usize)> = vec![];
    for (_, origin) in &acc.layered {
        match origins.iter_mut().find(|(o, _)| o == origin) {
            Some((_, n)) => *n += 1,
            None => origins.push((origin.clone(), 1)),
        }
    }
    RulesReport {
        rules: acc.layered.into_iter().map(|(r, _)| r).collect(),
        problems: acc.problems,
        origins,
        replaced: acc.replaced,
        disabled,
        catalog: acc.catalog.unwrap_or(true),
        includes: acc.includes,
        disables: acc.disables,
    }
}

#[cfg(test)]
pub(super) fn load_rules(
    repo_root: &str,
    extra: &[String],
) -> (Vec<ordo::model::Rule>, Vec<String>) {
    let r = load_rules_report(repo_root, extra);
    (r.rules, r.problems)
}

/// `kind = "x"` and `kind = ["x", "y"]` both read; a one-entry list is the
/// common case and shouldn't need brackets. Mirrors `model::string_or_vec`,
/// which is private to that module.
fn string_or_vec<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<V>::deserialize(d)? {
        None => None,
        Some(V::One(s)) => Some(vec![s]),
        Some(V::Many(v)) => Some(v),
    })
}

/// Flat TOML shape of one `[[rule]]` block: the file keeps rule fields and
/// `when` conditions in one table, while `ordo::model::Rule` nests the
/// conditions under `when` — this is the shape that gets converted.
#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct RuleToml {
    name: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    warn: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    noise: bool,
    #[serde(default)]
    priority: i64,

    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    path_not: Option<String>,
    #[serde(default)]
    test: Option<bool>,
    #[serde(default, deserialize_with = "string_or_vec")]
    lang: Option<Vec<String>>,
    #[serde(default)]
    category: Option<ordo::model::Category>,
    #[serde(default)]
    enclosing_kind: Option<String>,
    #[serde(default)]
    defines: Option<String>,
    #[serde(default)]
    uses: Option<String>,
    #[serde(default)]
    imports: Option<String>,
    #[serde(default)]
    noise_when: Option<bool>,
    #[serde(default)]
    comment: Option<bool>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    query_file: Option<String>,
    #[serde(default, deserialize_with = "string_or_vec")]
    kind: Option<Vec<String>>,
    #[serde(default, deserialize_with = "string_or_vec")]
    with: Option<Vec<String>>,
    #[serde(default, deserialize_with = "string_or_vec")]
    without: Option<Vec<String>>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    text_not: Option<String>,
    #[serde(default)]
    max_params: Option<usize>,
    #[serde(default)]
    max_lines: Option<usize>,
    #[serde(default)]
    max_nesting: Option<usize>,
    #[serde(default)]
    max_file_lines: Option<usize>,
    #[serde(default)]
    recursive: Option<bool>,
    #[serde(default)]
    container_with: Option<String>,
    #[serde(default)]
    container_without: Option<String>,
    #[serde(default)]
    member_uninitialized: Option<bool>,
}

/// Convert one already-parsed `[[rule]]` table into a `Rule`, independently of
/// every other rule in the file — a bad type or an unknown key in one block
/// must not cost the file its other, good rules. `query-file` is read
/// relative to `base` and lands in `When.query`, same as an inline `query`;
/// a missing file is a problem, not a panic.
fn rule_from_toml(
    v: toml::Value,
    idx: usize,
    base: &Path,
    problems: &mut Vec<String>,
) -> Option<ordo::model::Rule> {
    let label = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("rule {}", idx + 1));
    let parsed: RuleToml = match serde::Deserialize::deserialize(v) {
        Ok(p) => p,
        Err(e) => {
            problems.push(format!("{label}: {e}"));
            return None;
        }
    };
    let mut when = ordo::model::When {
        // the TOML layer reports its own unknown keys (see `RuleToml`), so
        // nothing reaches the engine's catch-all from here
        unknown: Default::default(),
        path: parsed.path,
        path_not: parsed.path_not,
        test: parsed.test,
        lang: parsed.lang,
        category: parsed.category,
        enclosing_kind: parsed.enclosing_kind,
        defines: parsed.defines,
        uses: parsed.uses,
        imports: parsed.imports,
        noise: parsed.noise_when,
        comment: parsed.comment,
        query: parsed.query,
        kind: parsed.kind,
        with: parsed.with,
        without: parsed.without,
        text: parsed.text,
        text_not: parsed.text_not,
        max_params: parsed.max_params,
        max_lines: parsed.max_lines,
        max_nesting: parsed.max_nesting,
        max_file_lines: parsed.max_file_lines,
        recursive: parsed.recursive,
        container_with: parsed.container_with,
        container_without: parsed.container_without,
        member_uninitialized: parsed.member_uninitialized,
    };
    if let Some(qf) = &parsed.query_file {
        match std::fs::read_to_string(base.join(qf)) {
            Ok(q) => when.query = Some(q),
            Err(e) => problems.push(format!("{label}: {qf}: {e}")),
        }
    }
    Some(ordo::model::Rule {
        unknown: Default::default(),
        name: parsed.name,
        when,
        note: parsed.note,
        warn: parsed.warn,
        verdict: parsed.verdict,
        noise: parsed.noise,
        priority: parsed.priority,
    })
}

/// The rules file: a sequence of `[[rule]]` blocks, real TOML — arrays
/// (`kind = [...]`) and multi-line `'''...'''` query strings read like
/// anywhere else in TOML. A syntax error, or a key outside `rule`, fails the
/// whole file (there is no document to salvage rules from); once the document
/// itself parses, each rule converts independently so one bad rule can't sink
/// the rest (see `rule_from_toml`).
/// One rules file, read: its rules, what it includes, what it disables.
pub(super) struct RulesDoc {
    pub(super) rules: Vec<ordo::model::Rule>,
    include: Vec<String>,
    disable: Vec<String>,
    /// `catalog = false` in any rules file turns the built-in construct
    /// catalog off for good, the way `--no-catalog` does for one run
    pub(super) catalog: Option<bool>,
    pub(super) problems: Vec<String>,
}

/// Could these two rules ever fire on one file? An unscoped rule applies to
/// every language, so it overlaps with anything.
fn langs_overlap(a: &ordo::model::Rule, b: &ordo::model::Rule) -> bool {
    match (&a.when.lang, &b.when.lang) {
        (Some(x), Some(y)) => x.iter().any(|l| y.contains(l)),
        _ => true,
    }
}

pub(super) fn parse_rules_doc(text: &str, base: &Path) -> RulesDoc {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RulesFile {
        #[serde(default)]
        include: Vec<String>,
        #[serde(default)]
        disable: Vec<String>,
        #[serde(default)]
        catalog: Option<bool>,
        #[serde(default)]
        rule: Vec<toml::Value>,
    }
    let empty = |problems| RulesDoc {
        rules: vec![],
        include: vec![],
        disable: vec![],
        catalog: None,
        problems,
    };
    let doc: RulesFile = match toml::from_str(text) {
        Ok(d) => d,
        Err(e) => return empty(vec![e.to_string()]),
    };
    let mut rules: Vec<ordo::model::Rule> = vec![];
    let mut problems = vec![];
    for (i, v) in doc.rule.into_iter().enumerate() {
        if let Some(r) = rule_from_toml(v, i, base, &mut problems) {
            // One construct often needs one rule per grammar — `magic-number`
            // is a `comparison_operator` in python and a `binary_expression`
            // in rust — and the engine keys its work by rule index, not name,
            // so those coexist. What is still a mistake is two rules of the
            // same name that could fire on the *same* file: then the review
            // shows the name twice and nothing says which spoke.
            if rules
                .iter()
                .any(|x| x.name == r.name && langs_overlap(x, &r))
            {
                problems.push(format!(
                    "rule `{}` is defined twice in this file for the same language",
                    r.name
                ));
                continue;
            }
            rules.push(r);
        }
    }
    RulesDoc {
        rules,
        include: doc.include,
        disable: doc.disable,
        catalog: doc.catalog,
        problems,
    }
}

#[cfg(test)]
pub(super) fn parse_rules(text: &str, base: &Path) -> (Vec<ordo::model::Rule>, Vec<String>) {
    let d = parse_rules_doc(text, base);
    (d.rules, d.problems)
}

/// The config file ordo would write for the current preset and theme — every
/// binding and every colour, commented out, at its real value.
///
/// Generated from the same tables the program reads (`keymap`, `ACTION_NAMES`,
/// `THEME_ROLES`, the resolved `Theme`), never from a hand-written template, so
/// it cannot drift from what the program actually accepts. A test uncomments
/// the whole thing and checks it parses with no complaints and changes nothing.
pub(super) fn init_config(preset: &str, theme_name: &str) -> String {
    let mut out = String::new();
    let km = keymap(preset).unwrap_or_else(|| keymap("vim").expect("vim preset exists"));
    let t = theme(theme_name).unwrap_or_else(|| theme("dark").expect("dark theme exists"));
    for line in [
        "# ordo configuration — every line below is this build's own default,",
        "# commented out. Uncomment and edit what you want to change.",
        "#",
        "# Written by `ordo --init-config`; the values are this build's, for",
        &format!("# preset `{preset}` and theme `{theme_name}`."),
    ] {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "preset = \"{}\"", km.name);
    let _ = writeln!(out, "theme = \"{}\"\n", t.name);

    for line in [
        "",
        "# ---------------------------------------------------------------- keys",
        "#",
        "# A line binds one key to one action; the action `none` removes a binding.",
        "# A chord is two keys separated by a space: `\"g d\"`, `\"C-w l\"`.",
        "# Modifiers are `C-`, `S-`, `A-`; named keys are Esc, Enter, Tab, Space,",
        "# Backspace, Up, Down, Left, Right, Home, End, PageUp, PageDown, F1..F12.",
        "[binds]",
    ] {
        let _ = writeln!(out, "{line}");
    }
    for (prefix, key, action) in &km.binds {
        let keys = match prefix {
            Some(p) => format!("{} {}", key_label(*p), key_label(*key)),
            None => key_label(*key),
        };
        let name = ACTION_NAMES
            .iter()
            .find(|(_, a)| a == action)
            .map(|(n, _)| *n)
            .unwrap_or("");
        let (_, help) = action_help(*action);
        let _ = writeln!(out, "# \"{keys}\" = \"{name}\"  # {help}");
    }

    for line in [
        "".to_string(),
        "# --------------------------------------------------------------- theme".to_string(),
        "#".to_string(),
        "# `name` picks a built-in palette; the roles below override it, as #rrggbb.".to_string(),
        format!("# Built-in: {}.", theme_names().join(", ")),
        "[theme]".to_string(),
        format!("# name = \"{}\"", t.name),
    ] {
        let _ = writeln!(out, "{line}");
    }
    // a terminal theme leaves some roles to the terminal's own palette: there
    // is no honest hex to print for those, and printing a placeholder would
    // mean this file stops being valid the moment someone uncomments it
    let inherited: Vec<&str> = THEME_ROLES
        .iter()
        .filter(|r| !matches!(theme_role_color(&t, r), Color::Rgb(..)))
        .copied()
        .collect();
    if !inherited.is_empty() {
        let _ = writeln!(
            out,
            "# These follow the terminal's own palette on this theme, so they have no\n             # default to show — set any of them to a colour to take it over:\n             #   {}",
            inherited.join(", ")
        );
    }
    for role in THEME_ROLES {
        if let Color::Rgb(r, g, b) = theme_role_color(&t, role) {
            let _ = writeln!(out, "# {role} = \"#{r:02x}{g:02x}{b:02x}\"");
        }
    }
    out
}
