//! Reviewing rules: the caller's own conventions, evaluated against the facts
//! the engine already computes.
//!
//! Two halves, deliberately:
//!
//! * **facts** — path, language, category, container kind, the names a hunk
//!   defines / uses / imports. Enough for "security paths first", "vendored
//!   code is noise", "flag a migration with no test beside it".
//! * **a tree-sitter query** — for a convention about code *shape* rather than
//!   about paths and names ("prefer `pathlib.Path` over `os.path.*`"). The
//!   query runs against the hunk's own lines, so a rule fires on what the
//!   change introduces rather than on everything the file already contained.
//!   That is the difference between a review signal and a linter backlog.
//!
//! Nothing here executes user code. A rule is data: globs and a query, both
//! matched deterministically, so the same input still yields the same output.
//! The engine also reads no files — `Options.rules` arrives from a client that
//! collected it (per-user and per-repo).
use crate::lang::{self, LangSpec};
use crate::model::{Category, ContainerKind, Finding, FindingSource, Level, Rule};
use globset::{Glob, GlobMatcher};
use regex::Regex;
use std::collections::HashMap;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

/// A rule with its globs and query compiled once, plus whatever refused to
/// compile — reported rather than silently dropped, since a rule that never
/// fires looks exactly like a convention nobody breaks.
pub struct Compiled<'r> {
    pub rule: &'r Rule,
    /// who this rule speaks for — the built-in construct catalog, or the
    /// caller's own conventions
    source: FindingSource,
    path: Option<GlobMatcher>,
    path_not: Option<GlobMatcher>,
    defines: Option<GlobMatcher>,
    uses: Option<GlobMatcher>,
    imports: Option<GlobMatcher>,
    container_with: Option<GlobMatcher>,
    container_without: Option<GlobMatcher>,
    text: Option<Regex>,
    text_not: Option<Regex>,
}

/// Everything the engine knows about one hunk that a rule can ask about.
pub struct HunkFacts<'a> {
    pub path: &'a str,
    /// 1-based inclusive new-side rows
    pub rows: (usize, usize),
    pub category: Category,
    /// the name of what holds the hunk, if anything — `enclosing_kind` alone
    /// cannot tell "a plain definition" from "no container at all", since both
    /// carry `None`
    pub enclosing: Option<&'a str>,
    pub enclosing_kind: Option<ContainerKind>,
    pub defines: &'a [String],
    pub uses: &'a [String],
    pub imports: &'a [String],
    pub noise: bool,
    pub comment: bool,
    pub def_lines: usize,
    pub def_params: usize,
    pub nesting: usize,
    /// the file's old (if known) and new line counts
    pub file_lines: (Option<usize>, usize),
    pub recursive: bool,
    pub container_members: &'a [String],
    pub uninit_members: &'a [String],
}

pub struct Rules<'r> {
    compiled: Vec<Compiled<'r>>,
    /// `rule: what was wrong`, for the caller's own rules — surfaced in
    /// `Output.problems`, because the caller wrote them and can fix them
    pub problems: Vec<String>,
    /// the same, for the built-in catalog. A catalog rule that fails to compile
    /// is *our* bug; putting it in `Output.problems` would tell a reviewer
    /// their rules file is broken when it is not. `tests/catalog.rs` asserts
    /// this is empty, which is where it belongs.
    pub catalog_problems: Vec<String>,
    /// how many leading entries of `compiled` are the catalog's
    catalog_len: usize,
    /// per language-less query rule: did it ever compile, and what did the
    /// first failure say. A query written for one grammar legitimately fails to
    /// parse under another, but one that parses under *none* of the languages
    /// in the change is broken, and saying so is the whole point of reporting.
    tried: HashMap<usize, (bool, String)>,
    /// compiled queries, cached by (rule index, language name) — `query_rows`
    /// runs once per file, so without this the same rule's `Query::new`
    /// (an automaton build, not cheap) reran for every file sharing a
    /// language. Same idea as the globs/regexes `Rules::new` compiles once.
    /// Keyed by index rather than by name: two rules may share a name (it is
    /// reported, not rejected) and must not share a compiled query.
    query_cache: HashMap<(usize, &'static str), Result<Query, String>>,
}

fn regex(
    pattern: &Option<String>,
    name: &str,
    rule: &str,
    problems: &mut Vec<String>,
) -> Option<Regex> {
    let p = pattern.as_ref()?;
    match Regex::new(p) {
        Ok(r) => Some(r),
        Err(e) => {
            problems.push(format!("rule `{rule}`: {name} `{p}` is not a regex: {e}"));
            None
        }
    }
}

fn glob(
    pattern: &Option<String>,
    name: &str,
    rule: &str,
    problems: &mut Vec<String>,
) -> Option<GlobMatcher> {
    let p = pattern.as_ref()?;
    match Glob::new(p) {
        Ok(g) => Some(g.compile_matcher()),
        Err(e) => {
            problems.push(format!("rule `{rule}`: {name} `{p}` is not a glob: {e}"));
            None
        }
    }
}

/// Every `enclosing_kind` a rule may name — the inverse of `kind_name`, so a
/// misspelling is reported rather than matching nothing forever.
const KIND_NAMES: &[&str] = &[
    "none",
    "definition",
    "test",
    "region",
    "preamble",
    "front-matter",
    "binding",
    "call",
    "document",
];

/// Everything in a rule that names something that does not exist: a
/// language, a key, a condition, a container kind.
fn unknown_parts(rule: &Rule) -> Vec<String> {
    let mut out = vec![];
    for l in rule.when.lang.iter().flatten() {
        if !lang::all().iter().any(|s| s.name == l) {
            out.push(format!("rule `{}`: unknown lang `{l}`", rule.name));
        }
    }
    for k in rule.unknown.keys() {
        out.push(format!("rule `{}`: unknown key `{k}`", rule.name));
    }
    for k in rule.when.unknown.keys() {
        out.push(format!("rule `{}`: unknown condition `{k}`", rule.name));
    }
    if let Some(k) = rule
        .when
        .enclosing_kind
        .as_deref()
        .filter(|k| !KIND_NAMES.contains(k))
    {
        out.push(format!(
            "rule `{}`: unknown enclosing_kind `{k}` (one of {})",
            rule.name,
            KIND_NAMES.join(", ")
        ));
    }
    out
}

impl<'r> Rules<'r> {
    pub fn new(rules: &'r [Rule]) -> Rules<'r> {
        Rules::with_catalog(&[], rules)
    }

    /// The built-in construct catalog and the caller's rules in one engine.
    ///
    /// They are the same mechanism and differ only in `FindingSource`, so they
    /// share one pass: `query_rows` parses each file once for both, rather
    /// than once per rule set.
    pub fn with_catalog(catalog: &'r [Rule], rules: &'r [Rule]) -> Rules<'r> {
        let mut problems = vec![];
        let mut catalog_problems = vec![];
        // A rule that never fires looks exactly like a convention nobody
        // breaks, so every way of writing one by accident is reported here:
        // a name that collides (the query cache and the noise/priority lookup
        // are both keyed by name), a language or container kind that does not
        // exist.
        let mut seen: Vec<&str> = vec![];
        for (own, rule) in catalog
            .iter()
            .map(|r| (false, r))
            .chain(rules.iter().map(|r| (true, r)))
        {
            // One construct can span grammars that spell it differently and
            // warrant different guidance — `unsafe` is a Rust block and a Go
            // package — so the catalog repeats a name on purpose. Two rules of
            // the caller's own sharing one name is still a mistake worth
            // reporting: both fire, and the review shows the name twice.
            if own && seen.contains(&rule.name.as_str()) {
                problems.push(format!("rule `{}`: duplicate rule name", rule.name));
            }
            if own {
                seen.push(&rule.name);
            }
            let sink = if own {
                &mut problems
            } else {
                &mut catalog_problems
            };
            sink.extend(unknown_parts(rule));
        }
        let compiled = catalog
            .iter()
            .map(|r| (FindingSource::Catalog, r))
            .chain(rules.iter().map(|r| (FindingSource::Rule, r)))
            .map(|(source, rule)| Compiled::new(rule, source, &mut problems))
            .collect();
        // a problem named after a catalog rule is ours; `Rules::new` compiles
        // the catalog first, so everything reported while `own` was false
        // belongs to us
        Rules {
            compiled,
            problems,
            catalog_problems,
            catalog_len: catalog.len(),
            tried: HashMap::new(),
            query_cache: HashMap::new(),
        }
    }

    /// Which rules match one hunk. `pattern_rows` holds, per rule *index*, the
    /// rows that rule's query or kind pattern matched in this file (see
    /// `query_rows`) — computed once per file rather than per hunk.
    ///
    /// Keyed by index, not name, because a construct name is the *finding's*
    /// name: `unsafe` is one advisory in Rust and another in Go, with its own
    /// message each, and a name-keyed table could only hold one of them.
    pub fn hits(&self, f: &HunkFacts, pattern_rows: &HashMap<usize, Vec<usize>>) -> Vec<Finding> {
        let lang = lang::for_path(f.path).map(|s| s.name);
        self.compiled
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                c.in_scope(f, lang)
                    && c.shape_matches(f)
                    && c.names_match(f)
                    && c.pattern_matches(f.rows, pattern_rows.get(i))
            })
            .flat_map(|(_, c)| c.findings())
            .collect()
    }

    /// Run every query rule against one file, returning the rows each matched.
    /// Rows are 1-based, to line up with hunk ranges.
    pub fn query_rows(&mut self, spec: &LangSpec, content: &str) -> HashMap<usize, Vec<usize>> {
        let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
        // A query is written against one grammar. A rule that names its
        // language is only tried there — and a failure there is the author's
        // bug, so it is reported. A rule that names none is tried everywhere,
        // and a grammar it doesn't parse simply doesn't match: complaining
        // that a Rust query "does not compile for markdown" would bury the
        // real errors in noise.
        let applies = |c: &Compiled| {
            c.rule
                .when
                .lang
                .as_ref()
                .is_none_or(|ls| ls.iter().any(|l| l == spec.name))
        };
        let queries: Vec<(usize, &'r str, &'r str, bool)> = self
            .compiled
            .iter()
            .enumerate()
            .filter(|(_, c)| applies(c))
            .filter_map(|(i, c)| {
                let q = c.rule.when.query.as_deref()?;
                Some((i, c.rule.name.as_str(), q, c.rule.when.lang.is_some()))
            })
            .collect();
        let kind_rules: Vec<(usize, &Compiled)> = self
            .compiled
            .iter()
            .enumerate()
            .filter(|(_, c)| c.rule.when.kind.is_some() && applies(c))
            .collect();
        if queries.is_empty() && kind_rules.is_empty() {
            return out;
        }
        let language = (spec.language)();
        let Some(tree) = lang::parse(spec, content) else {
            return out;
        };
        let by_kind = kind_rows(&kind_rules, tree.root_node(), content.as_bytes());
        let mut by_query: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, name, src, explicit) in queries {
            let compiled = self
                .query_cache
                .entry((i, spec.name))
                .or_insert_with(|| Query::new(&language, src).map_err(|e| e.to_string()));
            let rows = match compiled {
                Ok(q) => query_matches(q, tree.root_node(), content.as_bytes()),
                Err(e) => {
                    let e = e.clone();
                    self.query_failed(i, name, spec.name, explicit, e);
                    continue;
                }
            };
            if !explicit {
                self.tried.insert(i, (true, String::new()));
            }
            by_query.insert(i, rows);
        }
        // `When`'s conditions are ANDed, and `kind` and `query` are two of
        // them: a rule carrying both used to fire on the union of what each
        // matched. Intersect instead, so both have to point at the same row.
        for i in 0..self.compiled.len() {
            let rows = match (by_kind.get(&i), by_query.get(&i)) {
                (Some(k), Some(q)) => k.iter().filter(|r| q.contains(r)).copied().collect(),
                (Some(rows), None) | (None, Some(rows)) => rows.clone(),
                (None, None) => continue,
            };
            out.insert(i, rows);
        }
        out
    }

    /// A query that did not compile for `lang`: the author's bug when the
    /// rule named that language, else one more grammar it was not for.
    fn query_failed(&mut self, i: usize, name: &str, lang: &str, explicit: bool, err: String) {
        if !explicit {
            self.tried.entry(i).or_insert((false, err));
            return;
        }
        let msg = format!("rule `{name}`: query does not compile for {lang}: {err}");
        if i < self.catalog_len {
            self.catalog_problems.push(msg);
        } else {
            self.problems.push(msg);
        }
    }
}

/// kind rules: "this hunk introduces a node of kind K, with children X and
/// without children Y" — one walk of the tree, every rule checked at every
/// node. Children include anonymous tokens, so `without = "virtual"` reads a
/// keyword an anchor never could. Rows per rule index, sorted and unique.
fn kind_rows(
    kind_rules: &[(usize, &Compiled)],
    root: Node,
    src: &[u8],
) -> HashMap<usize, Vec<usize>> {
    if kind_rules.is_empty() {
        return HashMap::new();
    }
    let mut hits: Vec<Vec<usize>> = vec![vec![]; kind_rules.len()];
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        for (slot, (_, c)) in kind_rules.iter().enumerate() {
            if c.introduces(node, src) {
                hits[slot].push(node_row(node));
            }
        }
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
    }
    kind_rules
        .iter()
        .zip(hits)
        .map(|((i, _), rows)| (*i, unique_rows(rows)))
        .collect()
}

/// The rows a compiled query matches under `root`, sorted and unique.
fn query_matches(query: &Query, root: Node, src: &[u8]) -> Vec<usize> {
    let mut cursor = QueryCursor::new();
    let mut rows = vec![];
    let mut it = cursor.matches(query, root, src);
    while let Some(m) = it.next() {
        rows.extend(m.captures.iter().map(|cap| node_row(cap.node)));
    }
    unique_rows(rows)
}

fn unique_rows(mut rows: Vec<usize>) -> Vec<usize> {
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// A child by kind (`init_declarator`), by keyword token (`virtual`), or by
/// field name (`default_value`) — whichever the grammar exposes.
fn has_child(node: Node, want: &str) -> bool {
    if node.child_by_field_name(want).is_some() {
        return true;
    }
    let mut cur = node.walk();
    // bound, not returned: the iterator borrows `cur` past a tail expression
    let found = node.children(&mut cur).any(|ch| ch.kind() == want);
    found
}

impl<'r> Compiled<'r> {
    fn new(rule: &'r Rule, source: FindingSource, problems: &mut Vec<String>) -> Compiled<'r> {
        let w = &rule.when;
        let mut matcher =
            |pattern: &Option<String>, name: &str| glob(pattern, name, &rule.name, problems);
        let path = matcher(&w.path, "path");
        let path_not = matcher(&w.path_not, "path_not");
        let defines = matcher(&w.defines, "defines");
        let uses = matcher(&w.uses, "uses");
        let imports = matcher(&w.imports, "imports");
        let container_with = matcher(&w.container_with, "container_with");
        let container_without = matcher(&w.container_without, "container_without");
        Compiled {
            rule,
            source,
            path,
            path_not,
            defines,
            uses,
            imports,
            container_with,
            container_without,
            text: regex(&w.text, "text", &rule.name, problems),
            text_not: regex(&w.text_not, "text_not", &rule.name, problems),
        }
    }

    /// where the hunk is: its path, whether it is a test, its language
    fn in_scope(&self, f: &HunkFacts, lang: Option<&str>) -> bool {
        let w = &self.rule.when;
        self.path.as_ref().is_none_or(|g| g.is_match(f.path))
            && self.path_not.as_ref().is_none_or(|g| !g.is_match(f.path))
            && w.test.is_none_or(|t| t == lang::is_test_path(f.path))
            && w.lang
                .as_ref()
                .is_none_or(|ls| ls.iter().any(|l| lang == Some(l.as_str())))
    }

    /// what kind of hunk it is and what it measures
    fn shape_matches(&self, f: &HunkFacts) -> bool {
        let w = &self.rule.when;
        let (old_lines, new_lines) = f.file_lines;
        w.category.is_none_or(|c| c == f.category)
            && w.enclosing_kind.as_deref().is_none_or(|k| kind_name(f.enclosing, f.enclosing_kind) == k)
            && w.noise.is_none_or(|n| n == f.noise)
            && w.comment.is_none_or(|n| n == f.comment)
            && w.max_params.is_none_or(|m| f.def_params > m)
            && w.max_lines.is_none_or(|m| f.def_lines > m)
            && w.max_nesting.is_none_or(|m| f.nesting > m)
            // the file crossed the limit in this change — not every hunk of a
            // file that was already over it
            && w.max_file_lines.is_none_or(|m| new_lines > m && old_lines.is_none_or(|o| o <= m))
            && w.recursive.is_none_or(|r| r == f.recursive)
    }

    /// the names it defines, uses and imports, and what its container holds
    fn names_match(&self, f: &HunkFacts) -> bool {
        let hit = |m: &Option<GlobMatcher>, names: &[String]| {
            m.as_ref().map(|g| names.iter().any(|n| g.is_match(n)))
        };
        let any = |m, names| hit(m, names).unwrap_or(true);
        let none = |m, names| !hit(m, names).unwrap_or(false);
        let w = &self.rule.when;
        any(&self.defines, f.defines)
            && any(&self.uses, f.uses)
            && any(&self.imports, f.imports)
            && any(&self.container_with, f.container_members)
            && none(&self.container_without, f.container_members)
            && w.member_uninitialized
                .is_none_or(|b| b == !f.uninit_members.is_empty())
    }

    /// the rule's query or kind pattern matched a row of the hunk, when it
    /// has one (`matched` is what `query_rows` found for it in this file)
    fn pattern_matches(&self, (r0, r1): (usize, usize), matched: Option<&Vec<usize>>) -> bool {
        let w = &self.rule.when;
        (w.query.is_none() && w.kind.is_none())
            || matched.is_some_and(|rs| rs.iter().any(|r| r0 <= *r && *r <= r1))
    }

    /// What a matching rule says: one finding per message level it carries.
    /// A rule that only sets `noise` or `priority` still reports itself —
    /// otherwise a hunk sorts oddly with nothing to explain it.
    fn findings(&self) -> Vec<Finding> {
        let rule = self.rule;
        let finding = |message: String, level: Level| Finding {
            source: self.source,
            name: rule.name.clone(),
            message,
            level,
        };
        let mut out: Vec<Finding> = [
            (&rule.note, Level::Note),
            (&rule.warn, Level::Warn),
            (&rule.verdict, Level::Verdict),
        ]
        .into_iter()
        .filter_map(|(m, level)| m.clone().map(|m| finding(m, level)))
        .collect();
        if out.is_empty() {
            let what = match (rule.noise, rule.priority) {
                (true, 0) => "marked skippable".to_string(),
                (true, p) => format!("marked skippable, priority {p}"),
                (false, p) => format!("priority {p}"),
            };
            out.push(finding(what, Level::Note));
        }
        out
    }

    /// Does `node` satisfy this rule's `kind` / `with` / `without` / `text`?
    fn introduces(&self, node: Node, src: &[u8]) -> bool {
        let w = &self.rule.when;
        let Some(kinds) = &w.kind else { return false };
        if !kinds.iter().any(|k| k == node.kind()) {
            return false;
        }
        if w.with
            .as_ref()
            .is_some_and(|ws| !ws.iter().all(|k| has_child(node, k)))
        {
            return false;
        }
        if w.without
            .as_ref()
            .is_some_and(|ws| ws.iter().any(|k| has_child(node, k)))
        {
            return false;
        }
        if self.text.is_none() && self.text_not.is_none() {
            return true;
        }
        let text = node.utf8_text(src).unwrap_or("");
        self.text.as_ref().is_none_or(|r| r.is_match(text))
            && self.text_not.as_ref().is_none_or(|r| !r.is_match(text))
    }
}

impl Rules<'_> {
    /// Call once every file has been seen: a language-less query rule that
    /// never compiled anywhere is reported now, when "nowhere" is finally known.
    pub fn finish(&mut self) {
        let (ours, never): (Vec<_>, Vec<_>) = self
            .tried
            .iter()
            .filter(|(_, (ok, _))| !ok)
            .map(|(i, (_, err))| {
                let name = &self.compiled[*i].rule.name;
                (
                    *i < self.catalog_len,
                    format!("rule `{name}`: query does not compile for any language in this change: {err}"),
                )
            })
            .partition(|(is_catalog, _)| *is_catalog);
        let sorted = |v: Vec<(bool, String)>| {
            let mut v: Vec<String> = v.into_iter().map(|(_, m)| m).collect();
            v.sort();
            v
        };
        self.problems.append(&mut sorted(never));
        self.catalog_problems.append(&mut sorted(ours));
    }
}

/// The caller's own rules among `hits`. Only those may reclassify a hunk as
/// noise or rank it: the lookup is by name, and the catalog shares the
/// finding list — without the source filter, a user rule named `goto` would
/// lend its `noise` to every hunk the *catalog's* `goto` fired on.
fn own_rules<'r>(hits: &'r [Finding], rules: &'r [Rule]) -> impl Iterator<Item = &'r Rule> {
    hits.iter()
        .filter(|h| h.source == FindingSource::Rule)
        .filter_map(|h| rules.iter().find(|r| r.name == h.name))
}

/// Whether any matching rule asks for this hunk to be treated as noise.
pub fn any_noise(hits: &[Finding], rules: &[Rule]) -> bool {
    own_rules(hits, rules).any(|r| r.noise)
}

/// The highest priority among matching rules — 0 when none has an opinion.
pub fn priority(hits: &[Finding], rules: &[Rule]) -> i64 {
    own_rules(hits, rules)
        .map(|r| r.priority)
        .max()
        .unwrap_or(0)
}

fn node_row(n: Node) -> usize {
    n.start_position().row + 1
}

/// The `enclosing_kind` name a rule is written against. `None` for both fields
/// means the hunk sits in no container at all, which is not the same as sitting
/// in a plain definition — conflating the two fired every
/// `enclosing_kind = "definition"` rule on top-level code.
fn kind_name(enclosing: Option<&str>, k: Option<ContainerKind>) -> &'static str {
    match k {
        None if enclosing.is_none() => "none",
        None => "definition",
        Some(ContainerKind::Definition) => "definition",
        Some(ContainerKind::Test) => "test",
        Some(ContainerKind::Region) => "region",
        Some(ContainerKind::Namespace) => "namespace",
        Some(ContainerKind::Preamble) => "preamble",
        Some(ContainerKind::FrontMatter) => "front-matter",
        Some(ContainerKind::Binding) => "binding",
        Some(ContainerKind::Call) => "call",
        Some(ContainerKind::Document) => "document",
    }
}
