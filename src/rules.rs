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
use crate::model::{Category, ContainerKind, Rule, RuleHit};
use globset::{Glob, GlobMatcher};
use regex::Regex;
use std::collections::HashMap;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

/// A rule with its globs and query compiled once, plus whatever refused to
/// compile — reported rather than silently dropped, since a rule that never
/// fires looks exactly like a convention nobody breaks.
pub struct Compiled<'r> {
    pub rule: &'r Rule,
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
    /// `rule: what was wrong` — surfaced in `Output.problems`
    pub problems: Vec<String>,
    /// per language-less query rule: did it ever compile, and what did the
    /// first failure say. A query written for one grammar legitimately fails to
    /// parse under another, but one that parses under *none* of the languages
    /// in the change is broken, and saying so is the whole point of reporting.
    tried: HashMap<&'r str, (bool, String)>,
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

impl<'r> Rules<'r> {
    pub fn new(rules: &'r [Rule]) -> Rules<'r> {
        let mut problems = vec![];
        let compiled = rules
            .iter()
            .map(|rule| {
                let w = &rule.when;
                Compiled {
                    rule,
                    path: glob(&w.path, "path", &rule.name, &mut problems),
                    path_not: glob(&w.path_not, "path_not", &rule.name, &mut problems),
                    defines: glob(&w.defines, "defines", &rule.name, &mut problems),
                    uses: glob(&w.uses, "uses", &rule.name, &mut problems),
                    imports: glob(&w.imports, "imports", &rule.name, &mut problems),
                    container_with: glob(
                        &w.container_with,
                        "container_with",
                        &rule.name,
                        &mut problems,
                    ),
                    container_without: glob(
                        &w.container_without,
                        "container_without",
                        &rule.name,
                        &mut problems,
                    ),
                    text: regex(&w.text, "text", &rule.name, &mut problems),
                    text_not: regex(&w.text_not, "text_not", &rule.name, &mut problems),
                }
            })
            .collect();
        Rules {
            compiled,
            problems,
            tried: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.compiled.is_empty()
    }

    /// Which rules match one hunk. `pattern_rows` holds, per rule name, the
    /// rows that rule's query or kind pattern matched in this file (see
    /// `query_rows`) — computed once per file rather than per hunk.
    pub fn hits(&self, f: &HunkFacts, pattern_rows: &HashMap<&str, Vec<usize>>) -> Vec<RuleHit> {
        let lang = lang::for_path(f.path).map(|s| s.name);
        let (r0, r1) = f.rows;
        let (old_lines, new_lines) = f.file_lines;
        self.compiled
            .iter()
            .filter(|c| {
                let w = &c.rule.when;
                let any = |m: &Option<GlobMatcher>, names: &[String]| match m {
                    Some(g) => names.iter().any(|n| g.is_match(n)),
                    None => true,
                };
                c.path.as_ref().is_none_or(|g| g.is_match(f.path))
                    && c.path_not.as_ref().is_none_or(|g| !g.is_match(f.path))
                    && w.lang.as_deref().is_none_or(|l| lang == Some(l))
                    && w.category.is_none_or(|c2| c2 == f.category)
                    && w.enclosing_kind.as_deref().is_none_or(|k| kind_name(f.enclosing_kind) == k)
                    && any(&c.defines, f.defines)
                    && any(&c.uses, f.uses)
                    && any(&c.imports, f.imports)
                    && w.noise.is_none_or(|n| n == f.noise)
                    && w.comment.is_none_or(|n| n == f.comment)
                    && w.max_params.is_none_or(|m| f.def_params > m)
                    && w.max_lines.is_none_or(|m| f.def_lines > m)
                    && w.max_nesting.is_none_or(|m| f.nesting > m)
                    // the file crossed the limit in this change — not every
                    // hunk of a file that was already over it
                    && w.max_file_lines
                        .is_none_or(|m| new_lines > m && old_lines.is_none_or(|o| o <= m))
                    && w.recursive.is_none_or(|r| r == f.recursive)
                    && c.container_with.as_ref().is_none_or(|g| f.container_members.iter().any(|n| g.is_match(n)))
                    && c.container_without.as_ref().is_none_or(|g| !f.container_members.iter().any(|n| g.is_match(n)))
                    && w.member_uninitialized.is_none_or(|b| b == !f.uninit_members.is_empty())
                    && (w.query.is_none() && w.kind.is_none()
                        || pattern_rows
                            .get(c.rule.name.as_str())
                            .is_some_and(|rs| rs.iter().any(|r| r0 <= *r && *r <= r1)))
            })
            .flat_map(|c| {
                let mut out = vec![];
                if let Some(m) = &c.rule.note {
                    out.push(RuleHit {
                        rule: c.rule.name.clone(),
                        message: m.clone(),
                        level: "note",
                    });
                }
                if let Some(m) = &c.rule.warn {
                    out.push(RuleHit {
                        rule: c.rule.name.clone(),
                        message: m.clone(),
                        level: "warn",
                    });
                }
                // a rule that only sets `noise` or `priority` still reports
                // itself: otherwise a hunk sorts oddly with nothing to explain it
                if out.is_empty() {
                    let what = match (c.rule.noise, c.rule.priority) {
                        (true, 0) => "marked skippable".to_string(),
                        (true, p) => format!("marked skippable, priority {p}"),
                        (false, p) => format!("priority {p}"),
                    };
                    out.push(RuleHit {
                        rule: c.rule.name.clone(),
                        message: what,
                        level: "note",
                    });
                }
                out
            })
            .collect()
    }

    /// Whether any matching rule asks for this hunk to be treated as noise.
    pub fn any_noise(hits: &[RuleHit], rules: &'r [Rule]) -> bool {
        hits.iter()
            .filter_map(|h| rules.iter().find(|r| r.name == h.rule))
            .any(|r| r.noise)
    }

    /// The highest priority among matching rules — 0 when none has an opinion.
    pub fn priority(hits: &[RuleHit], rules: &'r [Rule]) -> i64 {
        hits.iter()
            .filter_map(|h| rules.iter().find(|r| r.name == h.rule))
            .map(|r| r.priority)
            .max()
            .unwrap_or(0)
    }

    /// Run every query rule against one file, returning the rows each matched.
    /// Rows are 1-based, to line up with hunk ranges.
    pub fn query_rows(&mut self, spec: &LangSpec, content: &str) -> HashMap<&'r str, Vec<usize>> {
        let mut out: HashMap<&str, Vec<usize>> = HashMap::new();
        // A query is written against one grammar. A rule that names its
        // language is only tried there — and a failure there is the author's
        // bug, so it is reported. A rule that names none is tried everywhere,
        // and a grammar it doesn't parse simply doesn't match: complaining
        // that a Rust query "does not compile for markdown" would bury the
        // real errors in noise.
        let queries: Vec<(&str, &str, bool)> = self
            .compiled
            .iter()
            .filter(|c| c.rule.when.lang.as_deref().is_none_or(|l| l == spec.name))
            .filter_map(|c| {
                let q = c.rule.when.query.as_deref()?;
                Some((c.rule.name.as_str(), q, c.rule.when.lang.is_some()))
            })
            .collect();
        let kind_rules: Vec<&Compiled> = self
            .compiled
            .iter()
            .filter(|c| c.rule.when.kind.is_some())
            .filter(|c| c.rule.when.lang.as_deref().is_none_or(|l| l == spec.name))
            .collect();
        if queries.is_empty() && kind_rules.is_empty() {
            return out;
        }
        let language = (spec.language)();
        let Some(tree) = lang::parse(spec, content) else {
            return out;
        };
        // kind rules: "this hunk introduces a node of kind K, with children X
        // and without children Y" — one walk of the tree, every rule checked
        // at every node. Children include anonymous tokens, so `without =
        // "virtual"` reads a keyword an anchor never could.
        if !kind_rules.is_empty() {
            let mut kind_hits: Vec<Vec<usize>> = vec![vec![]; kind_rules.len()];
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                for (i, c) in kind_rules.iter().enumerate() {
                    if c.introduces(node, content.as_bytes()) {
                        kind_hits[i].push(node_row(node));
                    }
                }
                let mut cur = node.walk();
                for ch in node.named_children(&mut cur) {
                    stack.push(ch);
                }
            }
            for (c, mut rows) in kind_rules.iter().zip(kind_hits) {
                rows.sort_unstable();
                rows.dedup();
                out.entry(c.rule.name.as_str()).or_default().extend(rows);
            }
        }
        if queries.is_empty() {
            return out;
        }
        for (name, src, explicit) in queries {
            let query = match Query::new(&language, src) {
                Ok(q) => q,
                Err(e) => {
                    if explicit {
                        self.problems.push(format!(
                            "rule `{name}`: query does not compile for {}: {e}",
                            spec.name
                        ));
                    } else {
                        self.tried.entry(name).or_insert((false, e.to_string()));
                    }
                    continue;
                }
            };
            if !explicit {
                self.tried.insert(name, (true, String::new()));
            }
            let mut cursor = QueryCursor::new();
            let mut rows = vec![];
            let mut it = cursor.matches(&query, tree.root_node(), content.as_bytes());
            while let Some(m) = it.next() {
                for cap in m.captures {
                    rows.push(node_row(cap.node));
                }
            }
            rows.sort_unstable();
            rows.dedup();
            out.entry(name).or_default().extend(rows);
        }
        out
    }
}

impl Compiled<'_> {
    /// Does `node` satisfy this rule's `kind` / `with` / `without` / `text`?
    fn introduces(&self, node: Node, src: &[u8]) -> bool {
        let w = &self.rule.when;
        let Some(kinds) = &w.kind else { return false };
        if !kinds.iter().any(|k| k == node.kind()) {
            return false;
        }
        // a child by kind (`init_declarator`), by keyword token (`virtual`),
        // or by field name (`default_value`) — whichever the grammar exposes
        let has = |want: &str| {
            if node.child_by_field_name(want).is_some() {
                return true;
            }
            let mut cur = node.walk();
            let mut found = false;
            for ch in node.children(&mut cur) {
                if ch.kind() == want {
                    found = true;
                    break;
                }
            }
            found
        };
        if w.with.as_ref().is_some_and(|ws| !ws.iter().all(|k| has(k))) {
            return false;
        }
        if w.without
            .as_ref()
            .is_some_and(|ws| ws.iter().any(|k| has(k)))
        {
            return false;
        }
        if self.text.is_some() || self.text_not.is_some() {
            let text = node.utf8_text(src).unwrap_or("");
            if self.text.as_ref().is_some_and(|r| !r.is_match(text)) {
                return false;
            }
            if self.text_not.as_ref().is_some_and(|r| r.is_match(text)) {
                return false;
            }
        }
        true
    }
}

impl Rules<'_> {
    /// Call once every file has been seen: a language-less query rule that
    /// never compiled anywhere is reported now, when "nowhere" is finally known.
    pub fn finish(&mut self) {
        let mut never: Vec<String> = self
            .tried
            .iter()
            .filter(|(_, (ok, _))| !ok)
            .map(|(name, (_, err))| {
                format!(
                    "rule `{name}`: query does not compile for any language in this change: {err}"
                )
            })
            .collect();
        never.sort();
        self.problems.append(&mut never);
    }
}

fn node_row(n: Node) -> usize {
    n.start_position().row + 1
}

fn kind_name(k: Option<ContainerKind>) -> &'static str {
    match k {
        None => "definition",
        Some(ContainerKind::Definition) => "definition",
        Some(ContainerKind::Test) => "test",
        Some(ContainerKind::Region) => "region",
        Some(ContainerKind::Preamble) => "preamble",
        Some(ContainerKind::FrontMatter) => "front-matter",
        Some(ContainerKind::Binding) => "binding",
        Some(ContainerKind::Call) => "call",
    }
}
