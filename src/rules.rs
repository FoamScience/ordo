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
use std::collections::HashMap;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator};

/// A rule with its globs and query compiled once, plus whatever refused to
/// compile — reported rather than silently dropped, since a rule that never
/// fires looks exactly like a convention nobody breaks.
pub struct Compiled<'r> {
    pub rule: &'r Rule,
    path: Option<GlobMatcher>,
    defines: Option<GlobMatcher>,
    uses: Option<GlobMatcher>,
    imports: Option<GlobMatcher>,
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
                    defines: glob(&w.defines, "defines", &rule.name, &mut problems),
                    uses: glob(&w.uses, "uses", &rule.name, &mut problems),
                    imports: glob(&w.imports, "imports", &rule.name, &mut problems),
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

    /// Which rules match one hunk. `query_rows` holds, per rule name, the rows
    /// that rule's query matched in this file (see `query_hits`) — computed once
    /// per file rather than per hunk.
    #[allow(clippy::too_many_arguments)]
    pub fn hits(
        &self,
        path: &str,
        rows: (usize, usize),
        category: Category,
        enclosing_kind: Option<ContainerKind>,
        defines: &[String],
        uses: &[String],
        imports: &[String],
        noise: bool,
        comment: bool,
        query_rows: &HashMap<&str, Vec<usize>>,
    ) -> Vec<RuleHit> {
        let lang = lang::for_path(path).map(|s| s.name);
        self.compiled
            .iter()
            .filter(|c| {
                let w = &c.rule.when;
                let any = |m: &Option<GlobMatcher>, names: &[String]| match m {
                    Some(g) => names.iter().any(|n| g.is_match(n)),
                    None => true,
                };
                c.path.as_ref().is_none_or(|g| g.is_match(path))
                    && w.lang.as_deref().is_none_or(|l| lang == Some(l))
                    && w.category.is_none_or(|c2| c2 == category)
                    && w.enclosing_kind
                        .as_deref()
                        .is_none_or(|k| kind_name(enclosing_kind) == k)
                    && any(&c.defines, defines)
                    && any(&c.uses, uses)
                    && any(&c.imports, imports)
                    && w.noise.is_none_or(|n| n == noise)
                    && w.comment.is_none_or(|n| n == comment)
                    && w.query.as_ref().is_none_or(|_| {
                        query_rows
                            .get(c.rule.name.as_str())
                            .is_some_and(|rs| rs.iter().any(|r| rows.0 <= *r && *r <= rows.1))
                    })
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
        if queries.is_empty() {
            return out;
        }
        let language = (spec.language)();
        let Some(tree) = lang::parse(spec, content) else {
            return out;
        };
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
