//! Line-hunks (via `similar`) + per-hunk semantic extraction: category,
//! enclosing definition, defines/uses. Parses the *new* content once with
//! tree-sitter, exactly as gitplay's `order.lua` does.
use crate::lang::{self, LangSpec};
use crate::model::{Advisory, Category};
use similar::TextDiff;
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

pub struct RawHunk {
    pub old_range: [usize; 2],
    pub new_range: [usize; 2],
    /// 0-based first new row (None = pure deletion, nothing to classify)
    pub new_r0: Option<usize>,
    /// 0-based last new row, inclusive (valid only when new_r0 is Some)
    pub new_r1: usize,
}

pub struct HunkSem {
    pub category: Category,
    pub enclosing: Option<String>,
    pub defines: Vec<String>,
    /// subset of `defines` that a hunk introduces via *import* nodes — used for
    /// rationale wording so an import+def hunk doesn't call function names imports
    pub imports: Vec<String>,
    pub uses: Vec<String>,
    /// a type-def (class/struct/enum/…) starts in this hunk (#4 wording)
    pub is_type: bool,
    /// formatting-only / generated-file hunk — skippable for review (P12.2)
    pub noise: bool,
    /// structural smells for a def introduced here (P13.1)
    pub notes: Vec<String>,
    /// advanced-construct advisories in this hunk (P14)
    pub advisories: Vec<Advisory>,
    pub start_row: usize,
    /// 1-based inclusive old-line range (for #5/#7 removal matching)
    pub old_range: [usize; 2],
}

impl HunkSem {
    pub fn other(h: &RawHunk) -> Self {
        HunkSem {
            category: Category::Other,
            enclosing: None,
            defines: vec![],
            imports: vec![],
            uses: vec![],
            is_type: false,
            noise: false,
            notes: vec![],
            advisories: vec![],
            start_row: h.new_r0.unwrap_or_else(|| h.old_range[0].saturating_sub(1)),
            old_range: h.old_range,
        }
    }
}

/// Group contiguous changed lines into hunks (context 0). 1-based inclusive
/// line ranges; an empty side is `[start, start-1]` (start > end signals empty).
pub fn compute_hunks(old: &str, new: &str) -> Vec<RawHunk> {
    let diff = TextDiff::from_lines(old, new);
    let mut hunks = vec![];
    for group in diff.grouped_ops(0) {
        let os = group.first().unwrap().old_range().start;
        let oe = group.last().unwrap().old_range().end;
        let ns = group.first().unwrap().new_range().start;
        let ne = group.last().unwrap().new_range().end;
        let old_range = if oe > os { [os + 1, oe] } else { [os + 1, os] };
        let new_range = if ne > ns { [ns + 1, ne] } else { [ns + 1, ns] };
        let (new_r0, new_r1) = if ne > ns {
            (Some(ns), ne - 1)
        } else {
            (None, 0)
        };
        hunks.push(RawHunk {
            old_range,
            new_range,
            new_r0,
            new_r1,
        });
    }
    hunks
}

#[derive(Default)]
struct Collected {
    import_rows: HashSet<usize>,
    def_rows: HashSet<usize>,
    type_rows: HashSet<usize>,
    defs: Vec<DefRec>,
    decls: Vec<(usize, String)>,
    import_decls: Vec<(usize, String)>,
    uses: Vec<(usize, String)>,
}

struct DefRec {
    s: usize,
    e: usize,
    name: String,
    depth: usize,  // enclosing def count (nesting)
    params: usize, // parameter count
}

// Structural-smell thresholds (P13.1) — change-shape signals, not style rules.
const LARGE_LINES: usize = 60;
const DEEP_NESTING: usize = 4;
const MANY_PARAMS: usize = 6;

/// Parse `new`, walk once, then classify each hunk. Returns None when the
/// grammar can't parse (caller falls back to file order).
pub fn analyze(spec: &LangSpec, new: &str, hunks: &[RawHunk]) -> Option<Vec<HunkSem>> {
    let mut parser = Parser::new();
    parser.set_language(&(spec.language)()).ok()?;
    let tree = parser.parse(new, None)?;
    let src = new.as_bytes();
    let mut c = Collected::default();
    let mut stack: Vec<String> = vec![];
    walk(tree.root_node(), src, spec, &mut stack, &mut c);
    let adv = crate::advisories::advise(spec, tree.root_node(), src);

    let mut out = Vec::with_capacity(hunks.len());
    for h in hunks {
        let (r0, r1) = match h.new_r0 {
            Some(r0) => (r0, h.new_r1),
            None => {
                out.push(HunkSem::other(h));
                continue;
            }
        };
        let category = if (r0..=r1).any(|r| c.import_rows.contains(&r)) {
            Category::Import
        } else if (r0..=r1).any(|r| c.def_rows.contains(&r)) {
            Category::Definition
        } else {
            Category::Other
        };
        let enclosing = c
            .defs
            .iter()
            .filter(|d| d.s <= r0 && r0 <= d.e)
            .min_by_key(|d| d.e - d.s)
            .map(|d| d.name.clone());
        let mut defines: Vec<String> = c
            .decls
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, n)| n.clone())
            .collect();
        defines.sort();
        defines.dedup();
        let defset: HashSet<&String> = defines.iter().collect();
        let mut uses: Vec<String> = c
            .uses
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, n)| n.clone())
            .filter(|n| !defset.contains(n))
            .collect();
        uses.sort();
        uses.dedup();
        let mut imports: Vec<String> = c
            .import_decls
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, n)| n.clone())
            .collect();
        imports.sort();
        imports.dedup();
        let is_type = (r0..=r1).any(|r| c.type_rows.contains(&r));
        // P13.1: structural smells for a def introduced in this hunk
        let mut notes = vec![];
        for d in c.defs.iter().filter(|d| r0 <= d.s && d.s <= r1) {
            let lines = d.e - d.s + 1;
            if lines >= LARGE_LINES {
                notes.push(format!("large definition ({lines} lines)"));
            }
            if d.depth >= DEEP_NESTING {
                notes.push(format!("deeply nested (depth {})", d.depth));
            }
            if d.params >= MANY_PARAMS {
                notes.push(format!("{} params", d.params));
            }
        }
        let advisories: Vec<Advisory> = adv
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, a)| a.clone())
            .collect();
        out.push(HunkSem {
            category,
            enclosing,
            defines,
            imports,
            uses,
            is_type,
            noise: false,
            notes,
            advisories,
            start_row: r0,
            old_range: h.old_range,
        });
    }
    Some(out)
}

fn walk(node: Node, src: &[u8], spec: &LangSpec, stack: &mut Vec<String>, c: &mut Collected) {
    let kind = node.kind();
    let sr = node.start_position().row;
    if spec.is_import(kind) {
        c.import_rows.insert(sr);
        for name in ident_texts(node, src) {
            c.decls.push((sr, name.clone()));
            c.import_decls.push((sr, name));
        }
        return; // don't descend: import identifiers are declarations, not uses
    }
    if spec.is_def(kind) {
        let er = node.end_position().row;
        let own = node_name(node, src).unwrap_or_else(|| "<anonymous>".to_string());
        c.def_rows.insert(sr);
        if lang::is_type_kind(kind) {
            c.type_rows.insert(sr);
        }
        c.decls.push((sr, own.clone()));
        let depth = stack.len(); // enclosing defs before this one
        let params = count_params(node);
        // collapse runs of nested anonymous defs in the qualified enclosing name
        let anon_dup = own == "<anonymous>" && stack.last().is_some_and(|s| s == "<anonymous>");
        if !anon_dup {
            stack.push(own);
        }
        c.defs.push(DefRec {
            s: sr,
            e: er,
            name: stack.join("."),
            depth,
            params,
        });
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, stack, c);
        }
        if !anon_dup {
            stack.pop();
        }
        return;
    }
    if lang::is_ident(kind) {
        if let Ok(t) = node.utf8_text(src) {
            c.uses.push((sr, t.to_string()));
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        walk(ch, src, spec, stack, c);
    }
}

fn count_params(node: Node) -> usize {
    node.child_by_field_name("parameters")
        .map(|p| {
            let mut cur = p.walk();
            p.named_children(&mut cur).count()
        })
        .unwrap_or(0)
}

fn node_name(node: Node, src: &[u8]) -> Option<String> {
    // 1. own name (function foo, class Foo, local function foo, impl Foo, …)
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(src).ok().map(|s| s.to_string());
    }
    // 2. anonymous expression → the binding it's assigned to
    //    (local x = function…, x = function…, t.x = function…, x: fn)
    if let Some(name) = bound_name(node, src) {
        return Some(name);
    }
    // 3. first identifier-ish child (e.g. rust impl's type_identifier)
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if lang::is_ident(ch.kind()) {
            return ch.utf8_text(src).ok().map(|s| s.to_string());
        }
    }
    None
}

// Name an anonymous def from the assignment/declaration/field that binds it.
// Climbs a few levels; returns the first identifier on the left of the def node.
fn bound_name(node: Node, src: &[u8]) -> Option<String> {
    let mut child = node;
    for _ in 0..3 {
        let parent = child.parent()?;
        let k = parent.kind();
        let binds = k.contains("assignment")
            || k.contains("declaration")
            || k.contains("variable")
            || k.contains("pair")
            || k.contains("binding")
            || k.ends_with("field");
        if binds {
            for f in ["name", "left", "variable", "key", "property"] {
                if let Some(n) = parent.child_by_field_name(f) {
                    if let Ok(t) = n.utf8_text(src) {
                        return Some(t.trim().to_string());
                    }
                }
            }
            // else: first identifier appearing before the def's subtree
            let mut c = parent.walk();
            for ch in parent.named_children(&mut c) {
                if ch.byte_range().start >= node.byte_range().start {
                    break;
                }
                if let Some(id) = first_ident_text(ch, src) {
                    return Some(id);
                }
            }
        }
        child = parent;
    }
    None
}

fn first_ident_text(node: Node, src: &[u8]) -> Option<String> {
    if lang::is_ident(node.kind()) {
        return node.utf8_text(src).ok().map(|s| s.to_string());
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if let Some(t) = first_ident_text(ch, src) {
            return Some(t);
        }
    }
    None
}

/// All def names and import names present in `content` (whole file). Used for
/// old-side comparison: add-vs-edit (#3), import removal (#5), rename (#7).
pub fn symbol_sets(spec: &LangSpec, content: &str) -> (HashSet<String>, HashSet<String>) {
    let mut defs = HashSet::new();
    let mut imports = HashSet::new();
    let mut parser = Parser::new();
    if parser.set_language(&(spec.language)()).is_err() {
        return (defs, imports);
    }
    let Some(tree) = parser.parse(content, None) else {
        return (defs, imports);
    };
    collect_syms(
        tree.root_node(),
        content.as_bytes(),
        spec,
        &mut defs,
        &mut imports,
    );
    (defs, imports)
}

/// Like `symbol_sets` but with each symbol's 1-based start row, for locating a
/// removed symbol against a deletion hunk's old range (#5 remove / #7 delete).
pub fn symbol_rows(spec: &LangSpec, content: &str) -> (Vec<(String, usize)>, Vec<(String, usize)>) {
    let mut defs = vec![];
    let mut imports = vec![];
    let mut parser = Parser::new();
    if parser.set_language(&(spec.language)()).is_err() {
        return (defs, imports);
    }
    let Some(tree) = parser.parse(content, None) else {
        return (defs, imports);
    };
    collect_rows(
        tree.root_node(),
        content.as_bytes(),
        spec,
        &mut defs,
        &mut imports,
    );
    (defs, imports)
}

/// Each def's name paired with its whitespace-normalized body text, for rename
/// detection by body similarity (#7 / P11.2). Body = the def's `body` field
/// (excludes the signature/name), else the whole node text.
pub fn symbol_bodies(spec: &LangSpec, content: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut parser = Parser::new();
    if parser.set_language(&(spec.language)()).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(content, None) else {
        return out;
    };
    collect_bodies(tree.root_node(), content.as_bytes(), spec, &mut out);
    out
}

fn collect_bodies(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<(String, String)>) {
    let kind = node.kind();
    if spec.is_def(kind) {
        if let Some(name) = node_name(node, src) {
            let text = node
                .child_by_field_name("body")
                .and_then(|b| b.utf8_text(src).ok())
                .or_else(|| node.utf8_text(src).ok())
                .unwrap_or("");
            let key = text.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push((name, key));
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_bodies(ch, src, spec, out);
    }
}

fn collect_rows(
    node: Node,
    src: &[u8],
    spec: &LangSpec,
    defs: &mut Vec<(String, usize)>,
    imports: &mut Vec<(String, usize)>,
) {
    let kind = node.kind();
    let row = node.start_position().row + 1; // 1-based
    if spec.is_import(kind) {
        for n in ident_texts(node, src) {
            imports.push((n, row));
        }
        return;
    }
    if spec.is_def(kind) {
        if let Some(n) = node_name(node, src) {
            defs.push((n, row));
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            collect_rows(ch, src, spec, defs, imports);
        }
        return;
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_rows(ch, src, spec, defs, imports);
    }
}

fn collect_syms(
    node: Node,
    src: &[u8],
    spec: &LangSpec,
    defs: &mut HashSet<String>,
    imports: &mut HashSet<String>,
) {
    let kind = node.kind();
    if spec.is_import(kind) {
        for n in ident_texts(node, src) {
            imports.insert(n);
        }
        return;
    }
    if spec.is_def(kind) {
        if let Some(n) = node_name(node, src) {
            defs.insert(n);
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            collect_syms(ch, src, spec, defs, imports);
        }
        return;
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_syms(ch, src, spec, defs, imports);
    }
}

fn ident_texts(node: Node, src: &[u8]) -> Vec<String> {
    let mut out = vec![];
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if lang::is_ident(ch.kind()) {
            if let Ok(t) = ch.utf8_text(src) {
                out.push(t.to_string());
            }
        }
        out.extend(ident_texts(ch, src));
    }
    out
}
