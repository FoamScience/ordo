//! Line-hunks (via `similar`) + per-hunk semantic extraction: category,
//! enclosing definition, defines/uses. Parses the *new* content once with
//! tree-sitter, exactly as gitplay's `order.lua` does.
use crate::lang::{self, LangSpec};
use crate::model::{Advisory, Category, ContainerKind, Symbol};
use similar::TextDiff;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

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
    /// what `enclosing` names, when it is not a plain definition
    pub enclosing_kind: Option<ContainerKind>,
    pub defines: Vec<String>,
    /// subset of `defines` that a hunk introduces via *import* nodes — used for
    /// rationale wording so an import+def hunk doesn't call function names imports
    pub imports: Vec<String>,
    pub uses: Vec<String>,
    /// a type-def (class/struct/enum/…) starts in this hunk (#4 wording)
    pub is_type: bool,
    /// formatting-only / generated-file hunk — skippable for review (P12.2)
    pub noise: bool,
    /// named members of the enclosing container this hunk touches, new side, as
    /// `(name, normalized text)` (P15) — compared against the old side to
    /// compose `details`
    pub members: Vec<(String, String, Option<String>)>,
    /// what the hunk did to its container's members: adds / removes / changes
    pub details: Vec<String>,
    /// structural smells for a def introduced here (P13.1)
    pub notes: Vec<String>,
    /// advanced-construct advisories in this hunk (P14)
    pub advisories: Vec<Advisory>,
    /// symbol identity (name + tree-sitter kind + scope) for each def this
    /// hunk introduces — matches `defines`, minus imports
    pub symbols: Vec<Symbol>,
    /// the hunk is a pure-import hunk whose statements all existed in the old
    /// file: the import moved rather than arriving or changing
    pub import_moved: bool,
    /// ordering influence from a matching rule (`Options.rules`) — higher
    /// sorts earlier, but only among hunks the dependency graph has freed
    pub priority: i64,
    /// local-variable bindings this hunk introduces (not defs, not imports —
    /// see `lang::LangSpec::locals`), with where each is used elsewhere in the
    /// file. Rationale wording only; never added to `defines`/`symbols`.
    pub bindings: Vec<BindingUse>,
    pub start_row: usize,
    /// 1-based inclusive old-line range (for #5/#7 removal matching)
    pub old_range: [usize; 2],
    /// hunk adds no new lines (pure deletion) — drives removal wording
    pub new_empty: bool,
    /// longest definition the hunk starts, in lines, and the most parameters
    /// one takes — the facts a `max-lines` / `max-params` rule reads
    pub def_lines: usize,
    pub def_params: usize,
    /// deepest control-flow nesting any row of the hunk sits at
    pub nesting: usize,
    /// a definition starting in the hunk uses its own name
    pub recursive: bool,
    /// names the hunk's container already has — methods, fields, variants —
    /// so a rule can ask "defines `equals` in a class without `hashCode`"
    pub container_members: Vec<String>,
    /// data members the hunk adds that nothing in the change initializes
    pub uninit_members: Vec<String>,
}

/// A local binding introduced by this hunk and where its name is used
/// elsewhere in the file. `scope` is the dotted enclosing-def name the
/// binding lives in, or None at module/script level — matches `enclosing`'s
/// naming so the "no uses" wording can say where it looked.
pub struct BindingUse {
    pub name: String,
    pub scope: Option<String>,
    /// 1-based line numbers where the name is used elsewhere, sorted
    pub uses: Vec<usize>,
}

impl HunkSem {
    pub fn other(h: &RawHunk) -> Self {
        HunkSem {
            category: Category::Other,
            enclosing: None,
            enclosing_kind: None,
            defines: vec![],
            imports: vec![],
            uses: vec![],
            is_type: false,
            noise: false,
            members: vec![],
            details: vec![],
            notes: vec![],
            advisories: vec![],
            symbols: vec![],
            import_moved: false,
            priority: 0,
            bindings: vec![],
            start_row: h.new_r0.unwrap_or_else(|| h.old_range[0].saturating_sub(1)),
            old_range: h.old_range,
            new_empty: h.new_r0.is_none(),
            def_lines: 0,
            def_params: 0,
            nesting: 0,
            recursive: false,
            container_members: vec![],
            uninit_members: vec![],
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
    /// (row, name, tree-sitter kind, enclosing scope) for each real definition
    /// — the raw material for `symbols` (name+kind+scope identity)
    sym_decls: Vec<(usize, String, String, Option<String>)>,
    import_decls: Vec<(usize, String)>,
    uses: Vec<(usize, String)>,
    /// parameter names (local bindings) seen anywhere in the file
    bound: HashSet<String>,
    /// named members of a container (enum variant, struct field, object
    /// property) as `(row, name, normalized text)` — the detail layer's raw
    /// material (P15). The text tells a member that merely shares a line with a
    /// change from one that actually changed.
    member_rows: Vec<MemberRow>,
    /// (row, name) of every local-variable binding target (P17: "where is
    /// this binding used?") — raw material for `HunkSem::bindings`
    local_binds: Vec<(usize, String)>,
    /// tree-sitter node ids of binding-target identifiers (the `local_binds`
    /// occurrences themselves), so the use-search below can exclude them
    /// without guessing from row/text alone
    bind_ids: HashSet<usize>,
    /// every identifier node in the file as (row, text, node id) — the use
    /// search's raw material, kept separate from `uses` (which already feeds
    /// the def→use edge graph and must not gain binding-target entries)
    all_idents: Vec<(usize, String, usize)>,
    /// deepest control-flow construct each row sits inside (0 = none)
    nest_rows: HashMap<usize, usize>,
    nest_depth: usize,
    /// (row, name) of every data member declared without an initializer
    uninit_fields: Vec<(usize, String)>,
    /// every member name an initializer list (c++) or `this.x = …` (java)
    /// in this file initializes
    field_inits: HashSet<String>,
}

/// Node kinds that open a level of control flow, across the grammars ordo
/// ships. A kind another grammar doesn't have simply never matches.
const CONTROL_KINDS: &[&str] = &[
    "if_statement",
    "for_statement",
    "while_statement",
    "do_statement",
    "switch_statement",
    "for_range_loop",
    "for_in_statement",
    "try_statement",
    "with_statement",
    "match_expression",
    "if_expression",
    "loop_expression",
    "while_expression",
    "for_expression",
    "if_let_expression",
    "repeat_statement",
    "elif_clause",
];

struct DefRec {
    s: usize,
    e: usize,
    name: String,
    depth: usize,  // enclosing def count (nesting)
    params: usize, // parameter count
    /// what this container is: a declaration, or a region that merely holds
    /// code (see `ContainerKind`)
    kind: ContainerKind,
}

// Structural-smell thresholds (P13.1) — change-shape signals, not style rules.
const LARGE_LINES: usize = 60;
const DEEP_NESTING: usize = 4;
const MANY_PARAMS: usize = 6;

/// Parse `new`, walk once, then classify each hunk. Returns None when the
/// grammar can't parse (caller falls back to file order).
pub fn analyze(spec: &LangSpec, new: &str, hunks: &[RawHunk], path: &str) -> Option<Vec<HunkSem>> {
    let tree = lang::parse(spec, new)?;
    let src = new.as_bytes();
    let mut c = Collected::default();
    let mut stack: Vec<String> = vec![];
    walk(tree.root_node(), src, spec, &mut stack, &mut c);
    let adv = crate::advisories::advise(spec, tree.root_node(), src, path);

    let lines: Vec<&str> = new.lines().collect();
    let mut out = Vec::with_capacity(hunks.len());
    for h in hunks {
        let (r0, r1) = match h.new_r0 {
            Some(r0) => (r0, h.new_r1),
            None => {
                out.push(HunkSem::other(h));
                continue;
            }
        };
        // Definition wins over Import: a hunk that adds real defs (e.g. a whole
        // new file, or an import block followed by functions) is a definition
        // hunk, not an import hunk — only a hunk that is *only* imports is Import.
        let category = if (r0..=r1).any(|r| c.def_rows.contains(&r)) {
            Category::Definition
        } else if (r0..=r1).any(|r| c.import_rows.contains(&r)) {
            Category::Import
        } else {
            Category::Other
        };
        // prose: containment is checked against r1, not r0. A markdown
        // section includes its trailing blank line up to the next sibling
        // heading, so a hunk that (say) adds a whole new subsection commonly
        // starts on that blank line — a row still owned by the *previous*
        // sibling, not the new subsection's actual parent. r1 lands inside
        // real content. Any def whose own heading starts within the hunk is
        // what the hunk *defines* (e.g. that new subsection), not its
        // container, so it's excluded — "enclosing" names the parent
        // instead of resolving to itself. Code languages are unaffected:
        // they keep the original r0-based, self-inclusive pick.
        let container = if spec.prose {
            c.defs
                .iter()
                .filter(|d| d.s <= r1 && r1 <= d.e)
                // a *definition* that starts inside the hunk is what the hunk
                // adds, not what contains it. A region can't be added that way
                // — a document has one preamble whether or not this hunk
                // touched its first line — so it stays eligible.
                .filter(|d| d.kind != ContainerKind::Definition || !(r0 <= d.s && d.s <= r1))
                .min_by_key(|d| d.e - d.s)
        } else {
            let at = |row: usize| {
                c.defs
                    .iter()
                    .filter(|d| d.s <= row && row <= d.e)
                    .min_by_key(|d| d.e - d.s)
            };
            // a hunk that starts on a blank line between two containers owns
            // no row of either; its first line with something on it does
            at(r0).or_else(|| {
                // rows here are 0-based (see `RawHunk::new_r0`)
                let first_real =
                    (r0..=r1).find(|r| lines.get(*r).is_some_and(|l| !l.trim().is_empty()))?;
                at(first_real)
            })
        };
        let enclosing = container.map(|d| d.name.clone());
        // a plain definition is the default and says nothing extra; only a
        // region (see `ContainerKind`) is worth reporting
        let enclosing_kind = container
            .map(|d| d.kind)
            .filter(|k| *k != ContainerKind::Definition);
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
            .filter(|n| !defset.contains(n) && !c.bound.contains(n))
            .collect();
        uses.sort();
        uses.dedup();
        let mut members: Vec<(String, String, Option<String>)> = c
            .member_rows
            .iter()
            .filter(|(row, _, _, _)| r0 <= *row && *row <= r1)
            .map(|(_, n, t, ctr)| (n.clone(), t.clone(), ctr.clone()))
            .collect();
        members.sort();
        members.dedup();
        let mut imports: Vec<String> = c
            .import_decls
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, n)| n.clone())
            .collect();
        imports.sort();
        imports.dedup();
        // an import is not a definition: exclude imported names from `defines` so
        // an importing hunk can't act as a def→use edge source (`defset` above
        // still suppresses them from `uses`). Real defs remain.
        let importset: HashSet<&String> = imports.iter().collect();
        defines.retain(|d| !importset.contains(d));
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
        // the same measurements, as facts a rule can put its own limit on
        let started: Vec<&DefRec> = c
            .defs
            .iter()
            .filter(|d| r0 <= d.s && d.s <= r1 && d.kind == ContainerKind::Definition)
            .collect();
        let def_lines = started.iter().map(|d| d.e - d.s + 1).max().unwrap_or(0);
        let def_params = started.iter().map(|d| d.params).max().unwrap_or(0);
        let nesting = (r0..=r1)
            .filter_map(|r| c.nest_rows.get(&r))
            .copied()
            .max()
            .unwrap_or(0);
        // a definition that names itself inside its own body — over and above
        // the declaring identifier, which `uses` also carries
        let recursive = started.iter().any(|d| {
            let declared = c
                .decls
                .iter()
                .filter(|(row, n)| *row == d.s && *n == d.name)
                .count();
            let named = c
                .uses
                .iter()
                .filter(|(row, n)| d.s <= *row && *row <= d.e && *n == d.name)
                .count();
            named > declared
        });
        // the container a defined symbol lives in (its scope), else the hunk's
        // enclosing one; its members are every symbol declared with that scope
        // plus the detail layer's members of it
        let container: Option<String> = defines
            .first()
            .and_then(|d| {
                c.sym_decls
                    .iter()
                    .find(|(row, n, _, _)| r0 <= *row && *row <= r1 && n == d)
                    .and_then(|(_, _, _, scope)| scope.clone())
            })
            .or_else(|| enclosing.clone());
        // data members this hunk declares that nothing in this file
        // initializes; `lib` widens the check to every file in the change
        let uninit_members: Vec<String> = c
            .uninit_fields
            .iter()
            .filter(|(row, n)| r0 <= *row && *row <= r1 && !c.field_inits.contains(n))
            .map(|(_, n)| n.clone())
            .collect();
        let container_members: Vec<String> = match &container {
            None => vec![],
            Some(cn) => {
                let mut m: Vec<String> = c
                    .sym_decls
                    .iter()
                    .filter(|(_, _, _, scope)| scope.as_deref() == Some(cn.as_str()))
                    .map(|(_, n, _, _)| n.clone())
                    .chain(
                        c.member_rows
                            .iter()
                            .filter(|(_, _, _, key)| key.as_deref() == Some(cn.as_str()))
                            .map(|(_, n, _, _)| n.clone()),
                    )
                    .collect();
                m.sort();
                m.dedup();
                m
            }
        };
        let advisories: Vec<Advisory> = adv
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, a)| a.clone())
            .collect();
        let mut symbols: Vec<Symbol> = c
            .sym_decls
            .iter()
            .filter(|(row, _, _, _)| r0 <= *row && *row <= r1)
            .map(|(_, name, kind, scope)| Symbol {
                name: name.clone(),
                kind: kind.clone(),
                scope: scope.clone(),
            })
            .collect();
        symbols.sort();
        symbols.dedup();
        // P17: local bindings this hunk introduces, and where each is used
        // elsewhere in the file. A binding whose row also holds a real def
        // (e.g. lua's `local f = function() end`, named via `bound_name`)
        // is already reported as that def — skip it here to avoid saying
        // the same thing twice. `_` (and other placeholder-only names) carry
        // no navigational signal, same reasoning `rationale_for` already
        // applies to `defines` — drop them here too rather than passing a
        // dead entry through to the rationale layer.
        let decl_at_row: HashSet<(usize, &str)> =
            c.decls.iter().map(|(r, n)| (*r, n.as_str())).collect();
        // several `locals`-kind nodes reassigning the same name within one
        // hunk (a variable rebound across a loop body, tuple-unpacked twice,
        // …) must collapse into one entry — otherwise the same name/use-list
        // gets reported multiple times, ballooning the rationale.
        let mut bindings: Vec<BindingUse> = vec![];
        for (row, name) in c
            .local_binds
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
        {
            if name == "_" || decl_at_row.contains(&(*row, name.as_str())) {
                continue;
            }
            let scope_def = c
                .defs
                .iter()
                .filter(|d| d.s <= *row && *row <= d.e)
                .min_by_key(|d| d.e - d.s);
            let (lo, hi) = scope_def.map(|d| (d.s, d.e)).unwrap_or((0, usize::MAX));
            let uses_here = c
                .all_idents
                .iter()
                .filter(|(r, n, id)| n == name && *r >= lo && *r <= hi && !c.bind_ids.contains(id))
                .map(|(r, _, _)| r + 1);
            match bindings.iter_mut().find(|b| &b.name == name) {
                Some(b) => b.uses.extend(uses_here),
                None => bindings.push(BindingUse {
                    name: name.clone(),
                    scope: scope_def.map(|d| d.name.clone()),
                    uses: uses_here.collect(),
                }),
            }
        }
        for b in &mut bindings {
            b.uses.sort();
            b.uses.dedup();
        }
        out.push(HunkSem {
            category,
            enclosing,
            enclosing_kind,
            defines,
            imports,
            uses,
            is_type,
            noise: false,
            members,
            details: vec![],
            notes,
            advisories,
            symbols,
            import_moved: false,
            priority: 0,
            bindings,
            start_row: r0,
            old_range: h.old_range,
            // a hunk whose new side is nothing but blank lines has as little
            // to say for itself as a pure deletion, and the same wording fits:
            // what a reviewer wants to know is what left
            new_empty: (r0..=r1).all(|r| lines.get(r).is_none_or(|l| l.trim().is_empty())),
            def_lines,
            def_params,
            nesting,
            recursive,
            container_members,
            uninit_members,
        });
    }
    Some(out)
}

/// An `export` that declares nothing of its own — `export * from "./x"`,
/// `export {a, b} from "./y"`, or the bare `export {}` module marker. It is
/// module bookkeeping in exactly the way an import is: it forwards or re-lists
/// names defined elsewhere, and follows from the real change rather than being
/// it. `export const foo = …` / `export function f()` carry a `declaration`
/// field and are NOT this — they are the definition they contain.
///
/// `export_statement` exists only in the javascript/typescript/tsx grammars
/// (verified against each one's node-types.json), so no other language's kinds
/// can collide with the check.
fn is_bookkeeping_export(node: Node) -> bool {
    if node.kind() != "export_statement" {
        return false;
    }
    // `export default <expression>` puts the exported object/array/call in the
    // `value` field rather than `declaration` (verified against
    // tree-sitter-typescript-0.23.2's node-types.json), and it is emphatically
    // NOT bookkeeping: `export default {…}` is the whole body of a config file
    // or a component. Reading it as an import swept every hunk inside it out of
    // the review.
    node.child_by_field_name("declaration").is_none() && node.child_by_field_name("value").is_none()
}

/// Import-like for classification: a real import, or an export that only moves
/// names around (see `is_bookkeeping_export`).
fn import_like(node: Node, src: &[u8], spec: &LangSpec) -> bool {
    // cmake names its imports rather than spelling them as distinct node
    // kinds: `include(Utils)` and `find_package(Boost)` are ordinary commands
    // bash sources a file with a command, not a keyword: `source x.sh` and its
    // POSIX spelling `. x.sh`
    if spec.name == "bash" && node.kind() == "command" {
        return node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(src).ok())
            .is_some_and(|t| matches!(t.trim(), "source" | "."));
    }
    // nix spells an import as an ordinary application of a function named
    // `import`, so the kind alone cannot tell one from any other call
    if spec.name == "nix" && node.kind() == "apply_expression" {
        return node
            .child_by_field_name("function")
            .filter(|f| f.kind() == "variable_expression")
            .and_then(|f| f.utf8_text(src).ok())
            .is_some_and(|t| t.trim() == "import");
    }
    if spec.name == "cmake" && node.kind() == "normal_command" {
        return cmake_command(node, src).is_some_and(|c| {
            matches!(c.as_str(), "include" | "find_package" | "add_subdirectory")
        });
    }
    spec.is_import(node.kind()) || is_bookkeeping_export(node)
}

/// `describe("adds two numbers", () => …)` — a call that names a block of code
/// the way a definition names one. The whole js/ts test-runner family (jest,
/// vitest, mocha, ava, node:test) shares this shape, and it is the container a
/// reviewer actually navigates by: without it every hunk in a test file sits at
/// file scope with nothing to attribute it to.
///
/// Requires all three of: a callee whose first segment is in `spec.test_blocks`
/// (so `test.serial`, `it.only` and `describe.each` count), a string first
/// argument, and a function argument to hold the body. A bare `test(name)` with
/// no body is a call, not a block, and is left alone.
fn test_block_label(node: Node, src: &[u8], spec: &LangSpec) -> Option<String> {
    if spec.test_blocks.is_empty() {
        return None;
    }
    // rust names tests through a macro rather than a call: `rgtest!(name, |…| {…})`,
    // `test_case!(…)`. The name is an identifier, not a string, and the entry is
    // matched as a *substring* of the macro name so one entry ("test") covers the
    // family. Verified against tree-sitter-rust-0.23.3: `macro_invocation` has a
    // `macro` field and a `token_tree` holding the arguments.
    if node.kind() == "macro_invocation" {
        let name = node.child_by_field_name("macro")?.utf8_text(src).ok()?;
        if !spec.test_blocks.iter().any(|t| name.contains(t)) {
            return None;
        }
        let args = node
            .named_children(&mut node.walk())
            .find(|n| n.kind() == "token_tree")?;
        let first = args.named_children(&mut args.walk()).next()?;
        if !lang::is_ident(first.kind()) {
            return None;
        }
        let label = first
            .utf8_text(src)
            .ok()
            .map(tidy_ident)
            .filter(|t| !t.is_empty())?;
        return Some(format!("{name}! {label}"));
    }
    if !matches!(node.kind(), "call_expression" | "function_call") {
        return None;
    }
    let callee = callee_text(node, src)?;
    let base = callee.split('.').next()?;
    if !spec.test_blocks.contains(&base) {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    let mut cur = args.walk();
    let children: Vec<Node> = args.named_children(&mut cur).collect();
    let has_body = children.iter().any(|n| {
        matches!(
            n.kind(),
            // js/ts | lua (busted's `describe("…", function() … end)`)
            "arrow_function" | "function_expression" | "generator_function" | "function_definition"
        )
    });
    if !has_body {
        return None;
    }
    // read the name straight off the node rather than through `tidy_ident`,
    // which collapses whitespace: a test name is prose, and "parses flags"
    // must not become "parsesflags". Only line breaks and runs of spaces are
    // normalised, so a wrapped name still reads as one line.
    let first = children.first()?;
    if !matches!(first.kind(), "string" | "template_string") {
        return None;
    }
    let name = first
        .utf8_text(src)
        .ok()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.chars().filter(|c| c.is_alphanumeric()).count() == 0 {
        return None; // an empty or punctuation-only name names nothing
    }
    // the label keeps the quotes the source wrote, so `describe "parses flags"`
    // reads as a name and can never be confused with an identifier
    Some(format!("{base} {name}"))
}

/// Whether a stack entry is a test-block label rather than a code scope — used
/// to join nested blocks with ` > ` instead of the language's scope separator,
/// so a nested suite reads `describe "cli" > it "parses flags"`.
fn is_test_label(s: &str) -> bool {
    s.split_once(' ')
        .is_some_and(|(head, rest)| rest.starts_with(['"', '\'', '`']) && !head.is_empty())
}

/// Templating over another format: a `values.yaml.j2` is yaml everywhere
/// except its `{% … %}` statements and `{# … #}` comments, which are not yaml
/// at all — one `{% for %}` is enough to make the whole document a parse
/// error. Blanking those regions (space for space, newlines kept) leaves text
/// that parses as the underlying format at **identical byte, row and column
/// offsets**, so every hunk range, every node position and every downstream
/// parse lines up with the file the reviewer is looking at.
///
/// An interpolation (`{{ … }}`, `<%= … %>`) is deliberately *not* blanked: it
/// sits where a scalar does and every format here already tolerates one, so
/// `web:\n  image: {{ tag }}` keeps both its key and a name worth reporting.
/// Which kinds are literal text and which are interpolations comes from the
/// templating grammar's own `Template` entry, so the pass is not jinja's.
///
/// Returns the rewritten text and the 0-based rows it blanked, so a hunk that
/// touches nothing but template syntax can still be told apart from one that
/// touches nothing at all. `None` when there was nothing to mask (the common
/// case for a path that merely ends in `.j2`), so the caller keeps the original.
pub fn mask_template(spec: &LangSpec, content: &str) -> Option<(String, HashSet<usize>)> {
    let t = spec.template?;
    let tree = lang::parse(spec, content)?;
    let mut keep: Vec<(usize, usize)> = vec![];
    collect_template_text(tree.root_node(), t, &mut keep);
    let mut out = content.as_bytes().to_vec();
    let len = out.len();
    let mut blank = vec![true; len];
    for (a, b) in keep {
        blank[a.min(len)..b.min(len)].fill(false);
    }
    let mut rows = HashSet::new();
    let mut row = 0;
    for (i, b) in blank.iter().enumerate() {
        if out[i] == b'\n' {
            row += 1;
            continue;
        }
        if *b {
            out[i] = b' ';
            rows.insert(row);
        }
    }
    if rows.is_empty() {
        return None;
    }
    let text = String::from_utf8(out).expect("blanking ASCII keeps UTF-8 valid");
    Some((text, rows))
}

// Byte ranges that are *not* template syntax: the host format's own text,
// plus the interpolations kept for its grammar (see `mask_template`).
fn collect_template_text(node: Node, t: &lang::Template, out: &mut Vec<(usize, usize)>) {
    if t.literal.contains(&node.kind()) || t.interpolation.contains(&node.kind()) {
        out.push((node.start_byte(), node.end_byte()));
        return;
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_template_text(ch, t, out);
    }
}

/// Every identifier a template's own syntax reads — `{{ db_host }}`,
/// `{% if tls %}` — as `(row, name)`. Recorded as **uses only**, like an
/// injected code fence: a template consumes variables defined elsewhere (an
/// inventory, a `group_vars` file) and defines none of them itself.
///
/// Empty for a grammar whose directives are one opaque blob rather than parsed
/// identifiers — ERB's ruby, say. That falls out rather than being special
/// cased: there are no identifier nodes to find.
pub fn template_uses(spec: &LangSpec, content: &str) -> Vec<(usize, String)> {
    let Some(t) = spec.template else {
        return vec![];
    };
    let Some(tree) = lang::parse(spec, content) else {
        return vec![];
    };
    let mut c = Collected::default();
    collect_template_uses(tree.root_node(), content.as_bytes(), t, &mut c);
    c.uses
}

fn collect_template_uses(node: Node, src: &[u8], t: &lang::Template, c: &mut Collected) {
    // literal text is the *host* format's business, not the template's
    if t.literal.contains(&node.kind()) {
        return;
    }
    if lang::is_ident(node.kind()) {
        if let Ok(t) = node.utf8_text(src).map(str::trim) {
            if !t.is_empty() {
                c.uses.push((node.start_position().row, t.to_string()));
            }
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_template_uses(ch, src, t, c);
    }
}

/// Language injection: a fenced code block in a prose file holds real code in
/// another language, and tree-sitter's own injection story says to parse it
/// with that language's grammar rather than as opaque text.
///
/// What comes back is recorded as **uses only, never definitions**. A ```python
/// block in a README demonstrates the project's API; it does not define it. So
/// a doc change that starts calling `parse_cfg` links to wherever `parse_cfg`
/// is defined (P2, across files) — while a sample that happens to write
/// `def foo(): …` never claims to define `foo` and can never be mistaken for
/// the real thing.
fn inject_fence(node: Node, src: &[u8], c: &mut Collected) {
    let info = node
        .named_children(&mut node.walk())
        .find(|n| n.kind() == "info_string")
        .and_then(|n| n.utf8_text(src).ok().map(str::to_string));
    let Some(inner) = info.as_deref().and_then(lang::for_lang_name) else {
        return;
    };
    let Some(content) = node
        .named_children(&mut node.walk())
        .find(|n| n.kind() == "code_fence_content")
    else {
        return;
    };
    let Ok(text) = content.utf8_text(src) else {
        return;
    };
    let Some(tree) = lang::parse(inner, text) else {
        return;
    };
    // rows inside the fence are relative to the fence; report them in the
    // enclosing file's coordinates so a hunk lines up with them
    let offset = content.start_position().row;
    collect_injected_uses(tree.root_node(), text.as_bytes(), offset, c);
}

fn collect_injected_uses(node: Node, src: &[u8], offset: usize, c: &mut Collected) {
    if lang::is_ident(node.kind()) {
        if let Ok(t) = node.utf8_text(src).map(str::trim) {
            if !t.is_empty() {
                c.uses
                    .push((node.start_position().row + offset, t.to_string()));
            }
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_injected_uses(ch, src, offset, c);
    }
}

/// The last row a container actually covers.
///
/// tree-sitter ends a node at the position *after* its last byte, so a node
/// whose text ends in a newline reports `end_position().row` one past its own
/// content, at column 0 — a zero-width boundary that belongs to the next
/// sibling. `#define GUARD_H` is the case that matters: it spans rows 1..=2 by
/// that reckoning, so the line *after* an include guard's define would be
/// attributed to the macro (verified against tree-sitter-c-0.23.4). Markdown
/// `section` has the same shape for a different reason — its end lands exactly
/// on the next sibling heading's row.
fn end_row(node: Node, spec: &LangSpec) -> usize {
    let e = node.end_position();
    if spec.prose {
        return e.row.saturating_sub(1);
    }
    if e.column == 0 && e.row > node.start_position().row {
        e.row - 1
    } else {
        e.row
    }
}

/// A top-level binding whose value spans more than one line — a settings dict,
/// an allow-list, a lookup table. The lines *inside* that value have no
/// definition around them, so without this they read as a bare "change"; with
/// it they belong to the binding whose value they are.
///
/// Scoped deliberately: only at file scope (a local inside a function already
/// has that function as its container) and only when the value is genuinely
/// multi-line (a one-line binding is its own hunk, and naming it would add
/// nothing the rationale doesn't already say). The binding is a *container*,
/// not a definition — it records no symbol, so nothing here can seed a def→use
/// edge or key a review mark.
fn binding_container(node: Node, src: &[u8], spec: &LangSpec, stack: &[String]) -> Option<String> {
    if !stack.is_empty() || !spec.is_local(node.kind()) || node.has_error() {
        return None;
    }
    if end_row(node, spec) <= node.start_position().row {
        return None;
    }
    let ident = binding_idents(node, node.kind()).into_iter().next()?;
    ident
        .utf8_text(src)
        .ok()
        .map(tidy_ident)
        .filter(|n| !n.is_empty())
}

/// The elements of a multi-line literal, as members of the binding that holds
/// it — so the detail layer can say *which* entry was added rather than only
/// that the table changed. A list element has no name of its own, so its own
/// text is its name (`"typing_extensions"`), which is exactly how a reviewer
/// refers to it.
fn literal_elements<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let value = ["right", "value"]
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .or_else(|| node.named_children(&mut node.walk()).last());
    let Some(v) = value else { return vec![] };
    if !matches!(
        v.kind(),
        "list" | "set" | "tuple" | "array" | "dictionary" | "object" | "table_constructor"
    ) {
        return vec![];
    }
    v.named_children(&mut v.walk()).collect()
}

/// A top-level call whose arguments span several lines — a fixture, a config
/// object, a big options literal. The lines inside those arguments have no
/// definition around them; the call is what they belong to.
///
/// Named the way the detail layer already names a call container:
/// `execa('unicorns')` — callee plus its first literal argument, which is what
/// distinguishes one such call from the next in a file full of them. Scoped to
/// file scope and to multi-line calls, for the same reasons as
/// `binding_container`, and it never applies to a test block (those are
/// recognised first and named by their own label).
fn call_statement_container(
    node: Node,
    src: &[u8],
    spec: &LangSpec,
    stack: &[String],
) -> Option<String> {
    if !stack.is_empty() || !matches!(node.kind(), "call_expression" | "call" | "function_call") {
        return None;
    }
    if end_row(node, spec) <= node.start_position().row {
        return None;
    }
    let callee = callee_text(node, src)?;
    if callee.is_empty() {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    Some(match first_literal_arg(args, src) {
        Some(lit) => format!("{callee}({lit})"),
        None => callee,
    })
}

/// A *region*: a container that holds code without declaring anything — a
/// conditional-compilation block, a document's preamble or its front matter.
/// Naming it is the difference between "change" and "edits code under
/// `#ifdef CURL_DISABLE_HTTP`", but it must never become a definition:
/// `CURL_DISABLE_HTTP` is *tested* there, not defined, and putting it in
/// `defines` would seed a def→use edge to whoever really defines it.
///
/// Node kinds verified against tree-sitter-{c,cpp}-0.23.4 (`preproc_ifdef` has
/// a `name` field, `preproc_if` a `condition`) and tree-sitter-md-0.5.3
/// (`minus_metadata`/`plus_metadata` for front matter; a `section` with no
/// heading child is the content before the document's first heading).
/// The leading words of a xonsh command line: every `subprocess_word`
/// argument up to the first flag, quoted string or python interpolation, so
/// `git remote add @(name) @(url)` names itself `git remote add`.
fn subprocess_label(node: Node, src: &[u8]) -> Option<String> {
    let mut cur = node.walk();
    let cmd = node
        .named_children(&mut cur)
        .find(|c| c.kind() == "subprocess_body")?
        .named_child(0)
        .filter(|c| c.kind() == "subprocess_command")?;
    let mut words = vec![];
    let mut c2 = cmd.walk();
    for arg in cmd.named_children(&mut c2) {
        let Some(word) = arg.named_child(0).filter(|w| w.kind() == "subprocess_word") else {
            break;
        };
        let Ok(text) = word.utf8_text(src) else { break };
        if text.starts_with('-') {
            break;
        }
        words.push(text.to_string());
    }
    (!words.is_empty()).then(|| words.join(" "))
}

fn region_label(
    node: Node,
    src: &[u8],
    spec: &LangSpec,
    top_level: bool,
) -> Option<(String, ContainerKind)> {
    let text_of = |n: Node| {
        n.utf8_text(src)
            .ok()
            .map(tidy_ident)
            .filter(|t| !t.is_empty())
    };
    match node.kind() {
        // `#ifdef X` and `#ifndef X` share a node kind; the directive token
        // itself says which, and a reviewer reads them very differently
        "preproc_ifdef" => {
            let name = text_of(node.child_by_field_name("name")?)?;
            let directive = node
                .child(0)
                .and_then(|d| d.utf8_text(src).ok())
                .map(|t| t.trim().to_string())
                .unwrap_or_else(|| "#ifdef".to_string());
            Some((format!("{directive} {name}"), ContainerKind::Region))
        }
        "preproc_if" => {
            // a condition is an expression, not an identifier: collapse runs of
            // whitespace but keep the single spaces that make it readable
            let cond = node.child_by_field_name("condition")?.utf8_text(src).ok()?;
            let cond = cond.split_whitespace().collect::<Vec<_>>().join(" ");
            (!cond.is_empty()).then(|| (format!("#if {cond}"), ContainerKind::Region))
        }
        // a `with` block at file scope: a script's real work often lives in
        // one (a pushd, an open file, a lock), and a hunk inside it otherwise
        // has no container at all. Nested inside a definition the definition
        // is the better name, so this claims only the top level.
        // Fields verified against tree-sitter-python-0.23.6 and
        // tree-sitter-xonsh-0.2.3's node-types.json.
        "with_statement" if top_level => {
            let item = node
                .named_child(0)
                .filter(|c| c.kind() == "with_clause")?
                .named_child(0)?;
            // an expression, not an identifier: collapse runs of whitespace
            // but keep the single spaces that make it readable, as `#if` does
            let text = item.child_by_field_name("value")?.utf8_text(src).ok()?;
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then(|| (format!("with {text}"), ContainerKind::Region))
        }
        // xonsh: a command line at file scope is the script's actual work, and
        // it is not a definition, a binding or a call node any other language
        // has. Name it by the command and its subcommand words —
        // `pip cache remove '*x*' || true` reads as "edits pip cache remove".
        "bare_subprocess" | "uncaptured_subprocess" if top_level => {
            let label = subprocess_label(node, src)?;
            Some((label, ContainerKind::Call))
        }
        // yaml: a stream can hold several `---` documents whose top-level keys
        // collide — two k8s objects each own a `spec`, and `spec.replicas`
        // alone does not say which. Name each document by its position, but
        // only when there is more than one: a single-document file keeps the
        // paths it has always had. A region, not a definition: an ordinal is
        // where a thing sits, never a symbol anything can use or define.
        "document" if node.parent().is_some_and(|p| p.kind() == "stream") => {
            let parent = node.parent()?;
            let mut cur = parent.walk();
            let docs: Vec<usize> = parent
                .named_children(&mut cur)
                .filter(|c| c.kind() == "document")
                .map(|c| c.id())
                .collect();
            if docs.len() < 2 {
                return None;
            }
            let n = docs.iter().position(|id| *id == node.id())? + 1;
            Some((format!("document {n}"), ContainerKind::Document))
        }
        // `@media (min-width: 700px)` is `#ifdef` in a different hat: a real
        // container worth naming that declares nothing.
        "media_statement" | "supports_statement" => {
            let head = node
                .named_children(&mut node.walk())
                .find(|c| !matches!(c.kind(), "block"))
                .and_then(|c| c.utf8_text(src).ok())
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|t| !t.is_empty())?;
            let at = if node.kind() == "media_statement" {
                "@media"
            } else {
                "@supports"
            };
            Some((format!("{at} {head}"), ContainerKind::Region))
        }
        "minus_metadata" | "plus_metadata" if spec.prose => {
            Some(("front matter".to_string(), ContainerKind::FrontMatter))
        }
        // a section with no heading of its own is everything before the first
        // heading: badges, a logo, an intro paragraph
        "section"
            if spec.prose
                && !node
                    .named_child(0)
                    .is_some_and(|h| matches!(h.kind(), "atx_heading" | "setext_heading")) =>
        {
            Some(("preamble".to_string(), ContainerKind::Preamble))
        }
        _ => None,
    }
}

fn walk(node: Node, src: &[u8], spec: &LangSpec, stack: &mut Vec<String>, c: &mut Collected) {
    let control = CONTROL_KINDS.contains(&node.kind());
    if control {
        c.nest_depth += 1;
        for r in node.start_position().row..=node.end_position().row {
            let d = c.nest_rows.entry(r).or_insert(0);
            *d = (*d).max(c.nest_depth);
        }
    }
    walk_node(node, src, spec, stack, c);
    if control {
        c.nest_depth -= 1;
    }
}

fn walk_node(node: Node, src: &[u8], spec: &LangSpec, stack: &mut Vec<String>, c: &mut Collected) {
    let kind = node.kind();
    let sr = node.start_position().row;
    if let Some(n) = field_init_name(node, src, spec) {
        c.field_inits.insert(n);
    }
    if let Some(name) = uninit_field(node, src, spec) {
        c.uninit_fields.push((sr, name));
    }
    if import_like(node, src, spec) {
        // the whole statement's rows count as import — a hunk that lands
        // anywhere in a multi-line `from x import (\n  a,\n  b,\n)` (tail,
        // middle, or head) is still an import hunk, not a bare "change".
        let er = end_row(node, spec);
        for r in sr..=er {
            c.import_rows.insert(r);
        }
        // A name is attributed to every row of its statement, not just the row
        // it is written on: a hunk that touches the tail of a multi-line import
        // list (`} from './y'`) still has the statement's names to report, and
        // "import" with nothing after it tells a reviewer nothing.
        for (_, name) in
            import_bound_names(node, src, spec).unwrap_or_else(|| ident_text_rows(node, src))
        {
            for r in sr..=er {
                c.decls.push((r, name.clone()));
                c.import_decls.push((r, name.clone()));
            }
        }
        return; // don't descend: import identifiers are declarations, not uses
    }
    if let Some(name) = binding_container(node, src, spec, stack) {
        // container only: no `def_rows`, no `decls`, no symbol — see
        // `binding_container`. The value's own elements become members so the
        // detail layer can name what changed inside it.
        for el in literal_elements(node) {
            // an element the language already treats as a member (a js/py
            // `pair`) is registered by the member branch below, by its key —
            // adding its whole text here as a second member would report the
            // same change twice, once named and once as raw text
            if spec.is_member(el.kind()) {
                continue;
            }
            if let Ok(text) = el.utf8_text(src) {
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !text.is_empty() {
                    c.member_rows.push((
                        el.start_position().row,
                        text.clone(),
                        text,
                        Some(name.clone()),
                    ));
                }
            }
        }
        c.defs.push(DefRec {
            s: sr,
            e: end_row(node, spec),
            name,
            depth: stack.len(),
            params: 0,
            kind: ContainerKind::Binding,
        });
        // fall through: the value still holds locals, uses and nested defs
    }
    if let Some((label, kind)) = region_label(node, src, spec, stack.is_empty()) {
        // a region names itself and nothing else: no `def_rows` (it declares
        // nothing, so a hunk in it is never a definition hunk), no `decls`
        // (nothing to add to `defines`), no symbol identity. Its own name is
        // still walked for uses below, so `#ifdef CURL_DISABLE_HTTP` counts as
        // a use of that macro — which is exactly what it is.
        let er = end_row(node, spec);
        let depth = stack.len();
        // A document is a namespace; every other region names only itself. An
        // `#ifdef` must not prefix the defs inside it — the definition is what
        // a reviewer navigates to, and the region is a fact about where it
        // sits. A `---` document is the opposite: the two `spec` keys of two
        // k8s objects are different keys, and the path has to say so.
        let scopes = kind == ContainerKind::Document;
        if scopes {
            stack.push(label.clone());
        }
        c.defs.push(DefRec {
            s: sr,
            e: er,
            name: if scopes {
                stack.join(lang::scope_sep(spec))
            } else {
                label
            },
            depth,
            params: 0,
            kind,
        });
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, stack, c);
        }
        if scopes {
            stack.pop();
        }
        return;
    }
    if let Some(label) = test_block_label(node, src, spec) {
        let er = end_row(node, spec);
        let depth = stack.len();
        // a nested block replaces its parent's entry for the duration, so the
        // qualified name reads `describe "cli" > it "parses flags"` rather than
        // repeating the parent through the language's scope separator
        let parent = stack.last().filter(|s| is_test_label(s)).cloned();
        let label = match &parent {
            Some(p) => {
                stack.pop();
                format!("{p} > {label}")
            }
            None => label,
        };
        // a named block, not a declaration: it gets a `defines` entry (so
        // adding or renaming one reads as such) but no symbol identity — a test
        // name is not a symbol another file can reference, and must never seed
        // a def→use edge or key a persisted review mark.
        c.def_rows.insert(sr);
        c.decls.push((sr, label.clone()));
        stack.push(label);
        c.defs.push(DefRec {
            s: sr,
            e: er,
            name: stack.join(lang::scope_sep(spec)),
            depth,
            params: 0,
            kind: ContainerKind::Test,
        });
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, stack, c);
        }
        stack.pop();
        if let Some(p) = parent {
            stack.push(p);
        }
        return;
    }
    if let Some(name) = call_statement_container(node, src, spec, stack) {
        c.defs.push(DefRec {
            s: sr,
            e: end_row(node, spec),
            name,
            depth: stack.len(),
            params: 0,
            kind: ContainerKind::Call,
        });
        // fall through: the arguments still hold uses, members and defs
    }
    if spec.is_def(kind) {
        let er = end_row(node, spec);
        // A def with no name of its own names no container, so it is
        // transparent: descend without pushing a scope. This covers a c++
        // anonymous `namespace {`, a lambda, and the content before a markdown
        // document's first heading — all of which would otherwise contribute an
        // `<anonymous>` segment to every enclosing name beneath them, and reach
        // the rationale. Their contents still nest under the nearest *named*
        // def, which is what a reviewer can actually navigate to.
        let Some(own) = node_name(node, src) else {
            let mut cur = node.walk();
            for ch in node.named_children(&mut cur) {
                walk(ch, src, spec, stack, c);
            }
            return;
        };
        // parameter names are local bindings, not references to outer symbols —
        // record them so a param that shadows a def elsewhere (e.g. a `lane`
        // fixture used only via param injection) can't seed a def→use edge.
        if let Some(p) = node.child_by_field_name("parameters") {
            for name in param_names(p, src) {
                c.bound.insert(name);
            }
        }
        // a jinja macro hangs its parameters off the same `function_call` that
        // carries its name — everything after that leading identifier
        for name in jinja_macro_params(node, src) {
            c.bound.insert(name);
        }
        // cmake spells a parameter list as the command's remaining arguments:
        // `function(my_helper arg)` binds `arg`, so `${arg}` in the body is not
        // read as a use of whatever else happens to be called `arg`
        for name in cmake_params(node, src) {
            c.bound.insert(name);
        }
        c.def_rows.insert(sr);
        if lang::is_type_kind(kind) {
            c.type_rows.insert(sr);
        }
        c.decls.push((sr, own.clone()));
        // prose and config only: a def kind that is *also* a member kind
        // (markdown's `section`, a config format's key) registers itself as a
        // member of its enclosing container too, so a new subsection or key
        // shows up in the P15 detail layer. Gated on the language shape rather
        // than on the overlap alone, because javascript's `method_definition`
        // *is* in both sets and must keep today's behavior — see lang.rs's
        // java comment on that same trap.
        if (spec.prose || spec.data) && spec.is_member(kind) {
            let text = node
                .utf8_text(src)
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            // neither prose nor config has calls: the container is always
            // the enclosing def (`stack`, not yet pushed with `own` here).
            let container = (!stack.is_empty()).then(|| stack.join(lang::scope_sep(spec)));
            c.member_rows.push((sr, own.clone(), text, container));
        }
        let depth = stack.len(); // enclosing defs before this one
        let params = count_params(node);
        // collapse runs of nested defs sharing a name in the qualified enclosing
        // name: nested anonymous defs, and python's decorated_definition wrapper
        // whose resolved name (via the `definition` field) duplicates the
        // class/function it wraps.
        let dup = stack.last().is_some_and(|s| s == &own);
        // a wrapper that delegates its name to an inner def (python's
        // decorated_definition -> `definition` field) isn't itself the
        // defining node — the inner def it wraps gets the symbol entry.
        let delegates = node
            .child_by_field_name("definition")
            .is_some_and(|d| spec.is_def(d.kind()));
        if !delegates {
            // scope excludes a duplicate trailing entry (the wrapper's own
            // push for this same symbol, not a genuine enclosing scope)
            let scope_stack = if dup {
                &stack[..stack.len() - 1]
            } else {
                &stack[..]
            };
            let scope = (!scope_stack.is_empty()).then(|| scope_stack.join(lang::scope_sep(spec)));
            c.sym_decls.push((sr, own.clone(), kind.to_string(), scope));
        }
        if !dup {
            stack.push(own);
        }
        c.defs.push(DefRec {
            s: sr,
            e: er,
            name: stack.join(lang::scope_sep(spec)),
            depth,
            params,
            kind: ContainerKind::Definition,
        });
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, stack, c);
        }
        if !dup {
            stack.pop();
        }
        return;
    }
    if spec.is_member(kind) {
        if let Some(name) = member_name(node, src) {
            let text = node
                .utf8_text(src)
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let container = member_container(node, src, stack, spec);
            c.member_rows.push((sr, name, text, container));
        }
        // fall through: a member's value can still hold defs and uses
    }
    // A declaration the parser could not make sense of yields nonsense names —
    // a macro-heavy c++ header (VTK's vtkTypeMacro family, say) parses with
    // ERROR nodes and hands back `virtual`/`override` as if they were bound
    // names. Nothing harvested from a failed parse is trustworthy.
    if spec.is_local(kind) && !node.has_error() {
        for id in binding_idents(node, kind) {
            if let Ok(name) = id.utf8_text(src).map(tidy_ident) {
                if name.is_empty() {
                    continue; // a macro-shaped declaration with no real name
                }
                c.local_binds.push((id.start_position().row, name));
                c.bind_ids.insert(id.id());
            }
        }
        // fall through: the bound value can still hold defs and uses
    }
    if spec.prose && kind == "fenced_code_block" {
        inject_fence(node, src, c);
        // fall through: the fence's own prose structure is still walked
    }
    // nix: an `attrpath` is a name being bound (`meta.description = …`) or
    // selected (`pkgs.gcc`) — never a free reference to something defined
    // elsewhere, so its identifiers are not uses. The binding's own name is
    // already filtered out of `uses` per hunk, but a dotted path is not.
    if spec.name == "nix" && kind == "attrpath" {
        return;
    }
    // a nix lambda binds its parameters: `{ pkgs, lib, ... }:` and `x: …`.
    // Not a definition, so the def branch's parameter handling never sees it.
    if kind == "function_expression" {
        if let Some(f) = node.child_by_field_name("formals") {
            for name in param_names(f, src) {
                c.bound.insert(name);
            }
        }
        if let Some(u) = node.child_by_field_name("universal") {
            if let Ok(t) = u.utf8_text(src) {
                c.bound.insert(t.to_string());
            }
        }
        // fall through: the body still holds bindings and uses
    }
    // bash: a command *is* a call, so `deploy main` uses the function `deploy`.
    // The name is a bare `word` — a kind make also uses for its targets — so
    // it is read here rather than through IDENT_KINDS. A builtin (`echo`,
    // `set`) resolves to no definition and costs nothing.
    if spec.name == "bash" && kind == "command_name" {
        if let Ok(t) = node.utf8_text(src).map(str::trim) {
            if !t.is_empty() {
                c.uses.push((sr, t.to_string()));
                c.all_idents.push((sr, t.to_string(), node.id()));
            }
        }
        return;
    }
    // make: a prerequisite names another target, and `$(CC)` names a variable.
    // Both are bare `word` nodes — a kind too generic to put in IDENT_KINDS,
    // so they are read from the two parents that make one mean a reference.
    if matches!(kind, "prerequisites" | "variable_reference") {
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            if ch.kind() == "word" {
                if let Ok(t) = ch.utf8_text(src).map(str::trim) {
                    if !t.is_empty() {
                        c.uses.push((sr, t.to_string()));
                        c.all_idents.push((sr, t.to_string(), ch.id()));
                    }
                }
            } else {
                walk(ch, src, spec, stack, c);
            }
        }
        return;
    }
    // css: a selector list is a *name*, never a set of references. Its
    // `class_name`/`id_name` wrap a plain `identifier`, which IDENT_KINDS
    // matches — so without this a stylesheet emits bare uses of `btn`,
    // `card`, `root` and `hover` into the union symbol table every other file
    // is ordered against, and starts drawing edges to python functions.
    if spec.name == "css" && kind == "selectors" {
        return;
    }
    // css custom properties: `--brand: #0af` declares a name and `var(--brand)`
    // uses it — the one def→use pair a stylesheet has, and so the only thing
    // that lets a css hunk be ordered rather than merely described. Both are
    // spelled as ordinary declarations and values, told apart by the `--`
    // every custom property must start with.
    if spec.name == "css" {
        if kind == "declaration" {
            let mut cur = node.walk();
            let prop = node
                .named_children(&mut cur)
                .find(|c| c.kind() == "property_name")
                .and_then(|c| c.utf8_text(src).ok())
                .map(str::trim)
                .filter(|t| t.starts_with("--"));
            if let Some(name) = prop {
                c.decls.push((sr, name.to_string()));
                c.def_rows.insert(sr);
                let scope = (!stack.is_empty()).then(|| stack.join(lang::scope_sep(spec)));
                c.sym_decls
                    .push((sr, name.to_string(), kind.to_string(), scope));
            }
            // fall through: the declaration is still a member, and its value
            // can still hold a `var(--other)`
        }
        if kind == "plain_value" {
            if let Ok(t) = node.utf8_text(src).map(str::trim) {
                if t.starts_with("--") {
                    c.uses.push((sr, t.to_string()));
                    c.all_idents.push((sr, t.to_string(), node.id()));
                    return;
                }
            }
        }
    }
    // yaml anchors: `&base` declares a name and `*base` uses it — the one real
    // def→use pair a config format has, and the only thing that lets a yaml
    // hunk be *ordered* rather than merely described. Both kinds are unique to
    // that grammar, so neither can shadow another language's identifiers.
    if matches!(kind, "anchor_name" | "alias_name") {
        if let Ok(t) = node.utf8_text(src).map(str::trim) {
            if !t.is_empty() {
                if kind == "anchor_name" {
                    c.decls.push((sr, t.to_string()));
                    c.def_rows.insert(sr);
                    c.sym_decls
                        .push((sr, t.to_string(), kind.to_string(), None));
                } else {
                    c.uses.push((sr, t.to_string()));
                    c.all_idents.push((sr, t.to_string(), node.id()));
                }
            }
        }
        return;
    }
    if lang::is_ident(kind) {
        // A zero-width identifier node is a parse artifact (C++ template and
        // macro constructs produce them); an empty name would surface in the
        // rationale as a stray comma. An all-digits one is a shell positional
        // parameter (`$1`, `$2`) — no language has a numeric symbol, so it can
        // never resolve to a definition and only clutters `uses`.
        if let Ok(t) = node.utf8_text(src).map(str::trim) {
            if !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()) {
                c.uses.push((sr, t.to_string()));
                c.all_idents.push((sr, t.to_string(), node.id()));
            }
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        walk(ch, src, spec, stack, c);
    }
}

// Identifier nodes bound (given a new value) by a `locals`-kind node — the
// name(s) it introduces, as nodes (not just text) so the caller can record
// their tree-sitter node id and exclude that exact occurrence from a later
// use-search. One field per grammar (verified against node-types.json);
// unlisted kinds yield nothing rather than guessing.
fn binding_idents<'t>(node: Node<'t>, kind: &str) -> Vec<Node<'t>> {
    let field = match kind {
        // xonsh `$FOO = …`: `left` is an `env_variable` wrapping the plain
        // identifier, so the bound name matches a use of `$FOO` elsewhere
        "assignment" | "short_var_declaration" | "env_assignment" => "left",
        // bash `APP_DIR=/srv/app`, and the same node inside a `local` /
        // `readonly` / `declare` wrapper the walk descends through
        "variable_assignment" => "name",
        "let_declaration" => "pattern",
        "var_spec" | "variable_declarator" => "name",
        // java local_variable_declaration / c/cpp declaration: one or more
        // `declarator` fields (`int x = 1, y = 2;`), each possibly wrapping
        // the name a level or two down (pointer/init declarator).
        "local_variable_declaration" | "declaration" => {
            let mut cur = node.walk();
            return node
                .children_by_field_name("declarator", &mut cur)
                .filter_map(declarator_ident)
                .collect();
        }
        // lua: `local x = …` is `variable_declaration` wrapping
        // `assignment_statement` -> `variable_list` (field `name`, one per
        // bound name) — no single field reaches the name from the top node.
        "variable_declaration" => return lua_binding_idents(node),
        // lua `M.defaults = { … }`: the bound name is a `variable_list` entry,
        // which may be a plain identifier or a `dot_index_expression`
        "assignment_statement" => {
            let mut cur = node.walk();
            return node
                .named_children(&mut cur)
                .filter(|c| c.kind() == "variable_list")
                .flat_map(|list| {
                    let mut c2 = list.walk();
                    list.named_children(&mut c2).collect::<Vec<_>>()
                })
                .collect();
        }
        _ => return vec![],
    };
    match node.child_by_field_name(field) {
        Some(target) => target_idents(target),
        None => vec![],
    }
}

// Identifier(s) inside an assignment/let/var target subtree, skipping into
// `attribute`/`subscript` targets (`obj.x = …`, `arr[0] = …` mutate an
// existing binding, not introduce one) — everything else (bare identifier,
// tuple/destructuring pattern) is a genuine new local name.
fn target_idents(node: Node) -> Vec<Node> {
    if matches!(node.kind(), "attribute" | "subscript") {
        return vec![];
    }
    if lang::is_ident(node.kind()) {
        return vec![node];
    }
    let mut out = vec![];
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        out.extend(target_idents(ch));
    }
    out
}

// c/cpp/java: unwrap a `declarator` field chain (pointer/init/array
// declarator, java's variable_declarator) down to the leaf name identifier.
fn declarator_ident(node: Node) -> Option<Node> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(n);
    }
    if let Some(d) = node.child_by_field_name("declarator") {
        return declarator_ident(d);
    }
    lang::is_ident(node.kind()).then_some(node)
}

fn lua_binding_idents(node: Node) -> Vec<Node> {
    let mut out = vec![];
    let mut cur = node.walk();
    for stmt in node
        .named_children(&mut cur)
        .filter(|c| c.kind() == "assignment_statement")
    {
        let mut c2 = stmt.walk();
        for list in stmt
            .named_children(&mut c2)
            .filter(|c| c.kind() == "variable_list")
        {
            let mut c3 = list.walk();
            out.extend(list.children_by_field_name("name", &mut c3));
        }
    }
    out
}

// A member's own name. `name` covers rust/go/c fields and enum variants, `key`
// covers object properties (js `pair`, ts `enum_assignment`); a `declarator`
// field covers java fields, whose name sits one level down in a nested
// `variable_declarator`; otherwise the first identifier leaf, which is the
// name in every remaining member kind.
fn member_name(node: Node, src: &[u8]) -> Option<String> {
    for field in ["name", "key"] {
        if let Some(n) = node.child_by_field_name(field) {
            if let Ok(t) = n.utf8_text(src) {
                let name = tidy_ident(unquote(t.trim()));
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    if let Some(d) = node.child_by_field_name("declarator") {
        if let Some(name) = declarator_name(d, src) {
            return Some(name);
        }
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        // `property_name` is css's: a declaration names itself with one, and
        // no other grammar here produces that kind
        if lang::is_ident(ch.kind()) || ch.kind() == "property_name" {
            return ch.utf8_text(src).ok().map(str::to_string);
        }
    }
    None
}

// Container identity for a member (P15's attribution fix): a member that
// sits directly under a call's argument list — python's `keyword_argument`
// is the current example — belongs to that call, not to whatever definition
// happens to enclose it. `add_argument("--sample", required=True)` inside
// `main` is a call `main` merely contains; the call, named by its callee
// plus first literal argument, is the real container. Everything else (a
// class body, an enum, a struct, an object/dict literal, a markdown section)
// keeps the enclosing-definition behavior member_rows always had — `stack`
// is exactly that definition, since a member is always visited before the
// def it names would be pushed onto it (see `walk`), which also means a
// member can never resolve to its own def through this path: the js
// `{ run: () => {} }` self-reference case is subsumed structurally, not
// just guarded against.
//
// KNOWN LIMITATION: two calls sharing both callee and first literal argument
// within one hunk collide onto the same key — the same class of limitation
// documented at `symbol_identity_key` (src/bin/ordo.rs).
fn member_container(node: Node, src: &[u8], stack: &[String], spec: &LangSpec) -> Option<String> {
    call_container(node, src).or_else(|| {
        // a test block's label is a sentence, and a nested one is two: naming
        // the innermost is what a detail line can afford. `describe "compiler:
        // transform v-bind" > test "error on invalid static argument"` becomes
        // `test "error on invalid static argument"`, which still identifies the
        // container while leaving room for what actually changed.
        let last = stack.last()?;
        if is_test_label(last) {
            return last.rsplit(" > ").next().map(str::to_string);
        }
        (!stack.is_empty()).then(|| stack.join(lang::scope_sep(spec)))
    })
}

// A member is a call's container only when it is a *direct* child of that
// call's argument list — one nested inside an object/dict literal passed as
// an argument is not (that object literal is not a call; today's def-based
// behavior applies, per spec).
fn call_container(node: Node, src: &[u8]) -> Option<String> {
    let args = node.parent()?;
    let call = args.parent()?;
    if call.child_by_field_name("arguments") != Some(args) {
        return None;
    }
    let callee = callee_text(call, src)?;
    match first_literal_arg(args, src) {
        Some(lit) => Some(format!("{callee}({lit})")),
        // no literal first argument: callee alone (documented fallback)
        None => Some(callee),
    }
}

// The callee's own text. `function` is the field name shared by
// call/call_expression across python/js/ts/rust/go/c/cpp (verified against
// each grammar's node-types.json); java's `method_invocation` has no
// `function` field — it names the callee via `name`, with an optional
// `object` receiver, instead.
fn callee_text(call: Node, src: &[u8]) -> Option<String> {
    if let Some(f) = call.child_by_field_name("function") {
        return f.utf8_text(src).ok().map(tidy_ident);
    }
    let name = call.child_by_field_name("name")?.utf8_text(src).ok()?;
    match call
        .child_by_field_name("object")
        .and_then(|o| o.utf8_text(src).ok())
    {
        Some(obj) => Some(format!("{}.{}", tidy_ident(obj), tidy_ident(name))),
        None => Some(tidy_ident(name)),
    }
}

// The call's first positional argument, only when it is a literal — judged by
// node kind, never by text.
fn first_literal_arg(args: Node, src: &[u8]) -> Option<String> {
    let mut cur = args.walk();
    let first = args.named_children(&mut cur).next()?;
    is_literal_kind(first.kind())
        .then(|| first.utf8_text(src).ok().map(tidy_ident))
        .flatten()
}

// Literal node kinds, verified per grammar against its own node-types.json —
// python (string/concatenated_string/integer/float/true/false/none), js & ts
// (string/number/true/false/null), rust (string_literal/raw_string_literal/
// char_literal/integer_literal/float_literal/boolean_literal), go
// (interpreted_string_literal/raw_string_literal/int_literal/float_literal/
// imaginary_literal/rune_literal/true/false/nil), c & cpp (string_literal/
// concatenated_string/char_literal/number_literal/true/false/null, cpp adds
// raw_string_literal), java (string_literal/character_literal/
// decimal_integer_literal/decimal_floating_point_literal/hex_integer_literal/
// hex_floating_point_literal/octal_integer_literal/binary_integer_literal/
// true/false/null_literal).
fn is_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "concatenated_string"
            | "integer"
            | "float"
            | "none"
            | "number"
            | "null"
            | "nil"
            | "true"
            | "false"
            | "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "character_literal"
            | "integer_literal"
            | "float_literal"
            | "boolean_literal"
            | "interpreted_string_literal"
            | "int_literal"
            | "imaginary_literal"
            | "rune_literal"
            | "number_literal"
            | "decimal_integer_literal"
            | "decimal_floating_point_literal"
            | "hex_integer_literal"
            | "hex_floating_point_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "null_literal"
    )
}

// Binding names in a parameter list — the name of each parameter, not its type
// annotation. Each direct child of the parameter container yields one name:
// a bare identifier is the name; otherwise the child's `name` field, else its
// first identifier leaf (which precedes any annotation). Type idents that sit
// after the name (under a `type` field / later children) are left out.
fn param_names(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = vec![];
    let mut cur = params.walk();
    for ch in params.named_children(&mut cur) {
        if lang::is_ident(ch.kind()) {
            if let Ok(t) = ch.utf8_text(src) {
                out.push(t.to_string());
            }
        } else if let Some(n) = ch.child_by_field_name("name") {
            if let Ok(t) = n.utf8_text(src) {
                out.push(t.to_string());
            }
        } else if let Some(id) = first_ident_text(ch, src) {
            out.push(id);
        }
    }
    out
}

// `{% macro row(a, b) %}` parses as `macro_statement -> function_call`, whose
// first identifier is the macro's own name and whose `arg`s are its
// parameters. Empty for every other node kind.
fn jinja_macro_params(node: Node, src: &[u8]) -> Vec<String> {
    if node.kind() != "macro_block" {
        return vec![];
    }
    let Some(call) = node
        .named_child(0)
        .and_then(|st| st.named_child(0))
        .filter(|n| n.kind() == "function_call")
    else {
        return vec![];
    };
    let mut cur = call.walk();
    call.named_children(&mut cur)
        .filter(|n| n.kind() == "arg")
        .filter_map(|n| first_ident_text(n, src))
        .collect()
}

// `function(my_helper arg …)` / `macro(m a b)`: every argument after the first
// is a parameter. Empty for every other node kind.
fn cmake_params(node: Node, src: &[u8]) -> Vec<String> {
    if !matches!(node.kind(), "function_def" | "macro_def") {
        return vec![];
    }
    let Some(args) = node.named_child(0).and_then(|head| {
        let mut cur = head.walk();
        let found = head
            .named_children(&mut cur)
            .find(|c| c.kind() == "argument_list");
        found
    }) else {
        return vec![];
    };
    let mut cur = args.walk();
    args.named_children(&mut cur)
        .skip(1)
        .filter_map(|a| a.utf8_text(src).ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Parameters of a definition. Most grammars put a `parameters` field on the
/// definition itself; C and C++ hang it off the declarator chain
/// (`declarator: (function_declarator parameters: …)`), so follow that.
fn count_params(node: Node) -> usize {
    let mut n = node;
    loop {
        if let Some(p) = n.child_by_field_name("parameters") {
            let mut cur = p.walk();
            return p.named_children(&mut cur).count();
        }
        match n.child_by_field_name("declarator") {
            Some(d) => n = d,
            None => return 0,
        }
    }
}

/// A code identifier with any internal whitespace removed. C++ (OpenFOAM's
/// house style especially) wraps a qualified name across lines —
/// `Foam::frictionalStressModels::\nJohnsonJacksonSchaeffer::nu` — and that
/// newline would otherwise reach `defines`, `symbols`, `enclosing` and the
/// rationale, which is contractually one line.
fn tidy_ident(s: &str) -> String {
    if s.chars().any(char::is_whitespace) {
        s.split_whitespace().collect()
    } else {
        s.to_string()
    }
}

fn node_name(node: Node, src: &[u8]) -> Option<String> {
    // markdown headings are prose: their spacing is meaningful, so they are
    // named by `heading_name` and never passed through `tidy_ident`.
    // a markdown heading and a css selector list are both punctuation-and-
    // spacing, not identifiers: their own naming paths normalize them, and
    // `tidy_ident` would glue `.btn, .btn-primary` into `.btn,.btn-primary`.
    if matches!(node.kind(), "section" | "rule_set") {
        return node_name_inner(node, src);
    }
    node_name_inner(node, src)
        .map(|n| tidy_ident(&n))
        .filter(|n| !n.is_empty())
}

fn node_name_inner(node: Node, src: &[u8]) -> Option<String> {
    // 0. markdown `section`: no name/declarator/identifier field exists (a
    // heading is prose, not an identifier) — name it from the heading text
    // instead. Scoped to this exact kind, which no other grammar in this
    // crate produces (verified against each grammar's node-types.json), so
    // it can't shadow any other language's naming path.
    if node.kind() == "section" {
        // a section's first child is only sometimes a heading: content
        // before the document's first heading is its own headless section
        // (e.g. an HTML comment or a stray paragraph at the top of a file).
        // Naming it after that raw content reads badly, so it stays
        // anonymous rather than borrowing the wrong node's text.
        if let Some(h) = node
            .named_child(0)
            .filter(|h| matches!(h.kind(), "atx_heading" | "setext_heading"))
        {
            return heading_name(h, src);
        }
        // ini spells `[user]` as a `section` too — same kind name, a different
        // grammar, told apart by the child that carries the name. It falls
        // through to the config-key path below; a *markdown* section with no
        // heading finds nothing there either (that grammar has no `*_name`
        // child and no identifier kind) and stays anonymous, as before.
    }
    // 1. own name (function foo, class Foo, local function foo, impl Foo, …).
    // Unquoted: a few grammars name a construct with a string literal rather
    // than an identifier — `{{ define "mychart.labels" }}` — and the quotes
    // are the grammar's, not part of the name.
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(src).ok().map(|t| unquote(t.trim()).to_string());
    }
    // 1b. name nested one or more levels down a `declarator` field — java
    // `field_declaration` -> `variable_declarator`, c/cpp `declaration` ->
    // `pointer_declarator`/`init_declarator`/… -> identifier.
    if let Some(d) = node.child_by_field_name("declarator") {
        if let Some(name) = declarator_name(d, src) {
            return Some(name);
        }
    }
    // 1c. python `decorated_definition` -> the class/function it wraps, under
    // field `definition`.
    if let Some(d) = node.child_by_field_name("definition") {
        if let Some(name) = node_name(d, src) {
            return Some(name);
        }
    }
    // 1c-css. A rule set is named by its whole selector list — `.btn` and
    // `#nav a:hover` as written, sigils kept, because the sigil is what makes
    // a css symbol unable to collide with a code one. Runs of whitespace
    // collapse to one space (a selector list is punctuation, not an
    // identifier, so `node_name` leaves it out of `tidy_ident`).
    if node.kind() == "rule_set" {
        let mut cur = node.walk();
        let sel = node
            .named_children(&mut cur)
            .find(|c| c.kind() == "selectors")?;
        let text = sel.utf8_text(src).ok()?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        return (!text.is_empty()).then_some(text);
    }
    if node.kind() == "keyframes_statement" {
        let mut cur = node.walk();
        let n = node
            .named_children(&mut cur)
            .find(|c| c.kind() == "keyframes_name")?;
        let t = n.utf8_text(src).ok()?.trim();
        return (!t.is_empty()).then(|| format!("@keyframes {t}"));
    }
    // 1c-make. A rule is named by its first target. A *special* target
    // (`.PHONY`, `.SUFFIXES`) names no recipe anyone navigates to, so it
    // stays anonymous — its prerequisites are still read as uses of the real
    // targets it lists, which is exactly what a `.PHONY` line is.
    if node.kind() == "rule" {
        // `targets` is a node kind here, not a field (unlike `normal:` for
        // prerequisites) — verified against tree-sitter-make-1.1.1
        let mut cur = node.walk();
        let targets = node
            .named_children(&mut cur)
            .find(|c| c.kind() == "targets")?;
        let text = targets.named_child(0)?.utf8_text(src).ok()?.trim();
        return (!text.is_empty() && !text.starts_with('.')).then(|| text.to_string());
    }
    // 1c-cmake. Every cmake construct is a command whose name is its first
    // argument: `function(my_helper …)`, `set(SOURCES …)`. A command that
    // introduces nothing resolves to no name and stays transparent, which is
    // why `normal_command` can sit in `defs` without every `message()` call
    // becoming a definition.
    if matches!(node.kind(), "function_def" | "macro_def") {
        return cmake_first_arg(node, src);
    }
    if node.kind() == "normal_command" {
        let cmd = cmake_command(node, src)?;
        if !matches!(cmd.as_str(), "set" | "option") {
            return None;
        }
        return cmake_first_arg(node, src);
    }
    // 1c-jinja. `{% block server %}` / `{% macro row(a) %}`: the name lives in
    // the opening statement, and for a macro one level further down inside a
    // `function_call` (jinja spells a parameter list the same way it spells a
    // call). Both kinds are unique to this grammar, so the deep search for the
    // first identifier can't reach into another language's shapes.
    if matches!(node.kind(), "block_block" | "macro_block") {
        return node.named_child(0).and_then(|st| first_ident_text(st, src));
    }
    // 1d. config formats: a key-value pair (and a toml `[table]` header) is
    // named by its key. json and yaml label it with a `key` field; toml-ng
    // labels no fields at all, so its key is the first `*_key` child. Reached
    // only for kinds the config specs declare as defs — python's and js's own
    // `pair` is a member, never a def, so it never enters `node_name`.
    if let Some(name) = config_key_name(node, src) {
        return Some(name);
    }
    // 2. anonymous expression → the binding it's assigned to
    //    (local x = function…, x = function…, t.x = function…, x: fn)
    if let Some(name) = bound_name(node, src) {
        return Some(name);
    }
    // 2b. the `type` field, for a def named after a type rather than an
    // identifier of its own — rust `impl<'s> Worker<'s>`, whose first named
    // child is the lifetime list, and `impl Display for Work`, where the first
    // identifier is the *trait*. Only when there is no `declarator`, so c/cpp's
    // `type` (a return type) and java's (a field type) can never be reached:
    // those shapes are named by 1b above.
    if node.child_by_field_name("declarator").is_none() {
        if let Some(t) = node.child_by_field_name("type") {
            if let Some(name) = type_name(t, src) {
                return Some(name);
            }
        }
    }
    // 3. first identifier-ish child (e.g. rust impl's type_identifier).
    // js/ts `arrow_function` is the one def kind whose own single bare
    // parameter (`x => …`, field `parameter`) is itself a direct identifier
    // child — without this guard it would be picked up here and misname an
    // anonymous callback after its own parameter instead of staying nameless.
    if node.kind() != "arrow_function" {
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            if lang::is_ident(ch.kind()) {
                return ch.utf8_text(src).ok().map(|s| s.to_string());
            }
        }
    }
    None
}

// Strip one matched pair of surrounding quotes. Matched, not `trim_matches`:
// a gitconfig subsection is `remote "origin"`, whose quotes are part of the
// name and whose leading character is not one — trimming from both ends
// independently would leave `remote "origin`.
fn unquote(s: &str) -> &str {
    let mut ch = s.chars();
    match (ch.next(), ch.next_back()) {
        (Some(a), Some(b)) if a == b && (a == '"' || a == '\'') => &s[a.len_utf8()..s.len() - a.len_utf8()],
        _ => s,
    }
}

/// cmake's command name — the identifier a `normal_command` leads with, or the
/// keyword a `function_def`/`macro_def` opens with. Every cmake construct is a
/// command, so this is what tells `set()` from `include()` from a call.
fn cmake_command(node: Node, src: &[u8]) -> Option<String> {
    let head = match node.kind() {
        "normal_command" => node,
        "function_def" | "macro_def" => node.named_child(0)?,
        _ => return None,
    };
    let mut cur = head.walk();
    let ident = head
        .named_children(&mut cur)
        .find(|c| c.kind() == "identifier")?;
    ident.utf8_text(src).ok().map(|t| t.to_ascii_lowercase())
}

/// The first argument of a cmake command — the name a `function`, `macro`,
/// `set` or `option` introduces, and the module an `include` pulls in.
fn cmake_first_arg(node: Node, src: &[u8]) -> Option<String> {
    let head = match node.kind() {
        "normal_command" => node,
        "function_def" | "macro_def" => node.named_child(0)?,
        _ => return None,
    };
    let mut cur = head.walk();
    let args = head
        .named_children(&mut cur)
        .find(|c| c.kind() == "argument_list")?;
    let first = args.named_child(0)?;
    let text = unquote(first.utf8_text(src).ok()?.trim());
    (!text.is_empty()).then(|| text.to_string())
}

// The key naming a config entry: a json/yaml pair (field `key`), a toml pair
// or `[table]` header, or an ini `[section]` / `setting` — the last two
// grammars label no fields, so their key is the first `*_key`/`*_name` child.
// Quotes are stripped so `"image"` and `image` name the same key.
fn config_key_name(node: Node, src: &[u8]) -> Option<String> {
    let key = node.child_by_field_name("key").or_else(|| {
        let mut cur = node.walk();
        let found = node
            .named_children(&mut cur)
            .find(|c| {
                matches!(
                    c.kind(),
                    // toml
                    "bare_key" | "quoted_key" | "dotted_key"
                    // ini: `[user]` and `name = A B`
                    | "section_name" | "setting_name"
                    // nix: `meta.description = …` — the whole dotted path
                    | "attrpath"
                )
            });
        found
    })?;
    // ini wraps the name in its delimiters — `section_name` spans `[user]\n`,
    // with the bare name under a `text` child
    let key = key
        .named_child(0)
        .filter(|c| c.kind() == "text")
        .unwrap_or(key);
    let text = unquote(key.utf8_text(src).ok()?.trim());
    // yaml's merge key: `<<: *defaults` is not a key a reviewer navigates by,
    // and "edits <<" says nothing. The pair stays anonymous, so the alias in
    // its value is still read as a use of the anchor it merges in.
    if text.is_empty() || text == "<<" {
        return None;
    }
    Some(text.to_string())
}

// The bare name of a type node, unwrapping a generic application so
// `Worker<'s>` reads as `Worker`.
fn type_name(node: Node, src: &[u8]) -> Option<String> {
    if lang::is_ident(node.kind()) {
        return node.utf8_text(src).ok().map(|s| s.to_string());
    }
    if let Some(t) = node.child_by_field_name("type") {
        return type_name(t, src);
    }
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
        // a statement/argument container sits between the def and any real
        // binding above it — a callback passed to a call (`arr.map(x => …)`)
        // or a value returned/nested inside a function body must not borrow
        // the name of whatever the call result or outer function is bound
        // to. Stop the climb rather than crossing into that unrelated scope.
        if matches!(
            k,
            "arguments" | "statement_block" | "class_body" | "program" | "block"
        ) {
            return None;
        }
        let binds = k.contains("assignment")
            || k.contains("declaration")
            || k.contains("variable")
            || k.contains("pair")
            || k.contains("binding")
            || k.contains("field");
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

// A markdown heading's title text: an atx heading's `heading_content` field is
// the `inline` node directly; a setext heading's is a `paragraph` wrapping an
// `inline`. Falls back to the heading's own text if neither shape matches.
fn heading_name(heading: Node, src: &[u8]) -> Option<String> {
    let content = heading.child_by_field_name("heading_content");
    let inline = match content {
        Some(c) if c.kind() == "inline" => Some(c),
        Some(c) => {
            let mut cur = c.walk();
            let found = c.named_children(&mut cur).find(|ch| ch.kind() == "inline");
            found
        }
        None => None,
    };
    let raw = inline
        .and_then(|n| n.utf8_text(src).ok())
        .or_else(|| heading.utf8_text(src).ok())?;
    normalize_heading(raw)
}

// strip leading `#` markers (belt-and-braces — heading_content already
// excludes them), collapse internal whitespace (a heading can wrap across
// lines), and cap the length: this name flows straight into a one-line
// rationale, and a heading can be a whole sentence.
const HEADING_NAME_MAX: usize = 80;

fn normalize_heading(raw: &str) -> Option<String> {
    let stripped = raw.trim_start_matches('#').trim();
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() > HEADING_NAME_MAX {
        let capped: String = collapsed.chars().take(HEADING_NAME_MAX).collect();
        return Some(format!("{capped}…"));
    }
    Some(collapsed)
}

// Follow a chain of declarator wrappers (c/cpp pointer/array/init/function
// declarators, java variable_declarator) down to the leaf name.
fn declarator_name(node: Node, src: &[u8]) -> Option<String> {
    declarator_ident(node)?
        .utf8_text(src)
        .ok()
        .map(str::to_string)
}

// The first quoted string anywhere under `node`, unquoted.
fn first_string_literal(node: Node, src: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "string_literal" | "interpreted_string_literal" | "raw_string_literal"
    ) {
        let t = node.utf8_text(src).ok()?.trim_matches(['"', '\'']);
        return (!t.is_empty()).then(|| t.to_string());
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if let Some(t) = first_string_literal(ch, src) {
            return Some(t);
        }
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

/// All local-binding names present anywhere in `content` (whole file) — the
/// old-side comparison set for P17: a name already locally bound in the old
/// file isn't "introduced" by a hunk that only edits its value.
pub fn local_names(spec: &LangSpec, content: &str) -> HashSet<String> {
    let Some(tree) = lang::parse(spec, content) else {
        return HashSet::new();
    };
    let mut c = Collected::default();
    let mut stack: Vec<String> = vec![];
    walk(
        tree.root_node(),
        content.as_bytes(),
        spec,
        &mut stack,
        &mut c,
    );
    c.local_binds.into_iter().map(|(_, n)| n).collect()
}

/// All def names and import names present in `content` (whole file). Used for
/// old-side comparison: add-vs-edit (#3), import removal (#5), rename (#7).
pub fn symbol_sets(spec: &LangSpec, content: &str) -> (HashSet<String>, HashSet<String>) {
    let (defs, imports) = symbol_rows(spec, content);
    let set = |v: Vec<(String, usize)>| v.into_iter().map(|(n, _)| n).collect();
    (set(defs), set(imports))
}

/// Like `symbol_sets` but with each symbol's 1-based start row, for locating a
/// removed symbol against a deletion hunk's old range (#5 remove / #7 delete).
/// A container member: `(0-based row, name, normalized text, container key)`.
/// The container key is `None` for a top-level member with no enclosing
/// definition and no call it's a direct argument of (P15's attribution fix
/// — see `member_container`).
pub type MemberRow = (usize, String, String, Option<String>);

/// Every container member in `content` — the old-side counterpart of what
/// `analyze` collects for the new one.
pub fn member_rows(spec: &LangSpec, content: &str) -> Vec<MemberRow> {
    let Some(tree) = lang::parse(spec, content) else {
        return vec![];
    };
    let mut c = Collected::default();
    let mut stack: Vec<String> = vec![];
    walk(
        tree.root_node(),
        content.as_bytes(),
        spec,
        &mut stack,
        &mut c,
    );
    c.member_rows
}

/// Names bound at file scope, with their 1-based rows — a module constant, a
/// lookup table, an exported literal. They are not definitions (see
/// `binding_container`), but a *removed* one is worth naming: "removes 1 line"
/// is what a reviewer gets otherwise. Function-local bindings are deliberately
/// excluded: their removal is part of editing the function that holds them.
pub fn top_level_bindings(spec: &LangSpec, content: &str) -> Vec<(String, usize)> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    collect_top_binds(tree.root_node(), content.as_bytes(), spec, &mut out);
    out
}

/// Walks the file scope only: it descends through wrappers (`export const …`
/// nests the declaration two levels down) but never into a definition, whose
/// bindings are locals belonging to it rather than to the file.
fn collect_top_binds(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<(String, usize)>) {
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        if spec.is_def(child.kind()) {
            continue;
        }
        if spec.is_local(child.kind()) && !child.has_error() {
            for id in binding_idents(child, child.kind()) {
                if let Ok(name) = id.utf8_text(src).map(tidy_ident) {
                    if !name.is_empty() {
                        out.push((name, child.start_position().row + 1));
                    }
                }
            }
            continue;
        }
        collect_top_binds(child, src, spec, out);
    }
}

/// Declared names with their 1-based rows: `(definitions, imports)`.
pub type SymbolRows = (Vec<(String, usize)>, Vec<(String, usize)>);

pub fn symbol_rows(spec: &LangSpec, content: &str) -> SymbolRows {
    let mut defs = vec![];
    let mut imports = vec![];
    let Some(tree) = lang::parse(spec, content) else {
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

/// Each def's `(name, normalized signature/header, normalized whole body,
/// substantial body lines)`. The header (node text before the `body` field)
/// drives signature-change vs body-only-edit wording (#4). The whole body drives
/// exact rename/move matching (#7 / P11.2); the line set drives line-overlap
/// relocation detection (P16). Body = the def's `body` field, else the node text.
pub type Body = (String, String, String, Vec<String>);

pub fn symbol_bodies(spec: &LangSpec, content: &str) -> Vec<Body> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    collect_bodies(tree.root_node(), content.as_bytes(), spec, &mut out);
    out
}

fn collect_bodies(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<Body>) {
    let kind = node.kind();
    if spec.is_def(kind) {
        if let Some(name) = node_name(node, src) {
            let full = node.utf8_text(src).unwrap_or("");
            // `value` is the body under another name: a macro (`preproc_def`,
            // `preproc_function_def`) and a rust `const_item`/`static_item`
            // hold theirs there, and without this every change to one reads as
            // a signature change because the header would be the whole node.
            let body = node
                .child_by_field_name("body")
                .or_else(|| node.child_by_field_name("value"))
                // css labels no field: a `rule_set`'s declarations are a
                // `block` child. Without this the header is the whole rule, so
                // every declaration edit reads as a change to the selector
                // itself and a renamed selector never matches its old body.
                // Gated to css so no shipped language moves — cmake's
                // `function_def` has an unlabelled `body` child too.
                .or_else(|| {
                    if spec.name != "css" {
                        return None;
                    }
                    let mut cur = node.walk();
                    let found = node.named_children(&mut cur).find(|c| c.kind() == "block");
                    found
                });
            // header = everything before the body (the signature); body text drives
            // rename/relocation matching. Fall back to the whole node when unsplit.
            let header = match body {
                Some(b) => &full[..(b.start_byte() - node.start_byte()).min(full.len())],
                None => full,
            };
            let text = body.and_then(|b| b.utf8_text(src).ok()).unwrap_or(full);
            let header = header.split_whitespace().collect::<Vec<_>>().join(" ");
            let whole = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let lines = text
                .lines()
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| l.chars().filter(|c| c.is_alphanumeric()).count() >= 3)
                .collect();
            out.push((name, header, whole, lines));
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
    if import_like(node, src, spec) {
        match import_bound_names(node, src, spec) {
            Some(names) => imports.extend(names.into_iter().map(|(_, n)| (n, row))),
            None => imports.extend(ident_texts(node, src).into_iter().map(|n| (n, row))),
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

// like `ident_texts`, but paired with each identifier's own row — an import
// statement spans several lines, and a hunk touching only one of them must
// find the names that actually sit on that line, not the statement's first.
/// The 1-based rows every import statement in a file covers. A hunk that only
/// *deletes* has no new side to classify from, so without this a removed import
/// reads as a plain `other` hunk — the same asymmetry that made an added import
/// invisible, one level down.
pub fn import_row_set(spec: &LangSpec, content: &str) -> HashSet<usize> {
    let mut out = HashSet::new();
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    each_import(tree.root_node(), content.as_bytes(), spec, &mut |n| {
        for r in n.start_position().row..=n.end_position().row {
            out.insert(r + 1);
        }
    });
    out
}

/// Visits every import statement, without descending into one.
fn each_import(node: Node, src: &[u8], spec: &LangSpec, f: &mut impl FnMut(Node)) {
    if import_like(node, src, spec) {
        f(node);
        return;
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        each_import(ch, src, spec, f);
    }
}

/// Every import statement in a file, as normalized text. An import that appears
/// in both sides of a change *moved*; one that does not is new or changed —
/// which is the difference between "moves import pg" and "changes import pg",
/// and the reason a reordered import block does not read as a pile of edits.
pub fn import_statements(spec: &LangSpec, content: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    each_import(tree.root_node(), content.as_bytes(), spec, &mut |n| {
        if let Ok(t) = n.utf8_text(src) {
            out.insert(t.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    });
    out
}

/// The names an import statement actually *binds*, rather than every identifier
/// in it. `from ppump.diagnostics import degrade` binds `degrade`; the module
/// path is how it was found, not what the file now has. Reporting all three
/// ("changes import degrade, diagnostics, ppump") both reads badly and decides
/// add-vs-change on the wrong evidence, since `ppump` is imported all over.
///
/// Fields verified against tree-sitter-python-0.23.6 and
/// tree-sitter-{javascript-0.23.1,typescript-0.23.2}'s node-types.json.
/// Languages whose imports are already one name per statement (rust `use`, go,
/// java, c `#include`) fall through to every identifier, which is that same
/// answer.
fn import_bound_names(node: Node, src: &[u8], spec: &LangSpec) -> Option<Vec<(usize, String)>> {
    // `import_statement` is python's kind *and* javascript/typescript's, and
    // they are shaped nothing alike: python has `name:` children, js has an
    // `import_clause` and a `source`. Gate on the language rather than trust a
    // shared kind name.
    let row = node.start_position().row;
    let text = |n: Node| n.utf8_text(src).ok().map(|t| (row, t.to_string()));
    // an include or a go import names a *path*, not an identifier, so the
    // identifier fallback finds nothing and the hunk binds no name at all —
    // `imports = "boost/**"` could never match. Bind the path text instead:
    // the include as written, a go package by the name code refers to it by.
    match (spec.name, node.kind()) {
        ("c" | "cpp", "preproc_include") => {
            let path = node.child_by_field_name("path")?.utf8_text(src).ok()?;
            let path = path.trim().trim_matches(|c| matches!(c, '<' | '>' | '"'));
            return Some(vec![(row, path.to_string())]);
        }
        // bash: the script `source ./lib/common.sh` pulls in
        ("bash", "command") => {
            let arg = node.child_by_field_name("argument")?;
            let text = unquote(arg.utf8_text(src).ok()?.trim());
            return (!text.is_empty()).then(|| vec![(row, text.to_string())]);
        }
        // nix: the path `import ./overlays.nix` pulls in
        ("nix", "apply_expression") => {
            let arg = node.child_by_field_name("argument")?;
            let text = arg.utf8_text(src).ok()?.trim();
            return (!text.is_empty()).then(|| vec![(row, text.to_string())]);
        }
        // make: `include common.mk` names the makefiles it pulls in
        ("make", "include_directive") => {
            let list = node.child_by_field_name("filenames")?;
            let mut cur = list.walk();
            let out: Vec<(usize, String)> = list
                .named_children(&mut cur)
                .filter_map(|n| n.utf8_text(src).ok())
                .map(|t| (row, t.trim().to_string()))
                .filter(|(_, t)| !t.is_empty())
                .collect();
            return (!out.is_empty()).then_some(out);
        }
        // cmake: the module, package or subdirectory the command names
        ("cmake", _) => return Some(vec![(row, cmake_first_arg(node, src)?)]),
        // a jinja `{% include 'tls.j2' %}` / `{% extends 'base.j2' %}` names a
        // template path, same shape as a c include: bind the path as written
        // so the hunk says which template arrived rather than bare "import".
        // `{% from 'c.j2' import d %}` also binds `d`, which the identifier
        // fallback below already picks up — so only the path is added here.
        ("jinja", _) => {
            let mut cur = node.walk();
            let lit = node
                .named_children(&mut cur)
                .find_map(|n| first_string_literal(n, src))?;
            return Some(vec![(row, lit)]);
        }
        ("go", "import_spec") => return Some(go_import_name(node, src).into_iter().collect()),
        ("go", "import_declaration") => {
            let mut cur = node.walk();
            let mut out = vec![];
            for spec_node in node.named_children(&mut cur) {
                match spec_node.kind() {
                    "import_spec" => out.extend(go_import_name(spec_node, src)),
                    "import_spec_list" => {
                        let mut c2 = spec_node.walk();
                        for sp in spec_node
                            .named_children(&mut c2)
                            .filter(|n| n.kind() == "import_spec")
                        {
                            out.extend(go_import_name(sp, src));
                        }
                    }
                    _ => {}
                }
            }
            return Some(out);
        }
        _ => {}
    }
    if !matches!(spec.name, "python" | "xonsh") {
        return None;
    }
    match node.kind() {
        // python: `import a.b` binds `a`; `from a.b import c, d as e` binds c, e
        "import_statement" | "import_from_statement" => {
            let from = node.kind() == "import_from_statement";
            let mut cur = node.walk();
            let out: Vec<(usize, String)> = node
                .children_by_field_name("name", &mut cur)
                .filter_map(|n| match n.kind() {
                    "aliased_import" => text(n.child_by_field_name("alias")?),
                    // a dotted name binds its last segment when imported *from*
                    // a module, and its first when the module itself is imported
                    "dotted_name" => {
                        let mut c2 = n.walk();
                        let parts: Vec<Node> = n.named_children(&mut c2).collect();
                        text(*(if from { parts.last()? } else { parts.first()? }))
                    }
                    _ => text(n),
                })
                .collect();
            Some(out)
        }
        _ => None,
    }
}

/// `import "go.uber.org/zap"` binds `zap`; `import z "go.uber.org/zap"` binds
/// `z`; a blank or dot import binds nothing a hunk could be said to use.
/// Fields verified against tree-sitter-go-0.23's node-types.json.
fn go_import_name(spec: Node, src: &[u8]) -> Option<(usize, String)> {
    let row = spec.start_position().row;
    if let Some(alias) = spec.child_by_field_name("name") {
        let a = alias.utf8_text(src).ok()?;
        return (a != "_" && a != ".").then(|| (row, a.to_string()));
    }
    let path = spec.child_by_field_name("path")?.utf8_text(src).ok()?;
    let last = path.trim_matches('"').rsplit('/').next()?;
    (!last.is_empty()).then(|| (row, last.to_string()))
}

/// A data member declared without an initializer — `int a;`, `int* p;` in C++
/// (a `field_identifier` under the declarator chain, no `default_value`),
/// `int a;` in Java (a `variable_declarator` with no `value`). A member
/// *function* is a `field_declaration` in C++ too and is not data. C structs
/// have no constructors to initialize in, so C is left alone.
/// Shapes verified against tree-sitter-cpp-0.23 / tree-sitter-java-0.23.
fn uninit_field(node: Node, src: &[u8], spec: &LangSpec) -> Option<String> {
    if node.kind() != "field_declaration" {
        return None;
    }
    let text = |n: Node| n.utf8_text(src).ok().map(str::to_string);
    match spec.name {
        "cpp" => {
            if node.child_by_field_name("default_value").is_some() {
                return None;
            }
            let mut d = node.child_by_field_name("declarator")?;
            // pointer/array declarators name their inner declarator as a
            // field; a reference declarator holds it as a bare child
            while matches!(
                d.kind(),
                "pointer_declarator" | "reference_declarator" | "array_declarator"
            ) {
                d = d
                    .child_by_field_name("declarator")
                    .or_else(|| d.named_child(0))?;
            }
            (d.kind() == "field_identifier").then(|| text(d)).flatten()
        }
        "java" => {
            let d = node.child_by_field_name("declarator")?;
            if d.kind() != "variable_declarator" || d.child_by_field_name("value").is_some() {
                return None;
            }
            text(d.child_by_field_name("name")?)
        }
        _ => None,
    }
}

/// The member an initializer names: `: a(x)` in a C++ constructor's
/// initializer list, `this.a = x` in a Java constructor body.
fn field_init_name(node: Node, src: &[u8], spec: &LangSpec) -> Option<String> {
    let text = |n: Node| n.utf8_text(src).ok().map(str::to_string);
    match (spec.name, node.kind()) {
        ("cpp", "field_initializer") => text(
            node.named_child(0)
                .filter(|n| n.kind() == "field_identifier")?,
        ),
        ("java", "assignment_expression") => {
            let left = node.child_by_field_name("left")?;
            if left.kind() != "field_access" || left.child_by_field_name("object")?.kind() != "this"
            {
                return None;
            }
            text(left.child_by_field_name("field")?)
        }
        _ => None,
    }
}

/// Every member name the constructors in `content` initialize — the other
/// half of "a member added in this change with no initializer", which may
/// live in the `.cpp` while the member lives in the header.
pub fn field_initializers(spec: &LangSpec, content: &str) -> HashSet<String> {
    let Some(tree) = lang::parse(spec, content) else {
        return HashSet::new();
    };
    let mut c = Collected::default();
    let mut stack: Vec<String> = vec![];
    walk(
        tree.root_node(),
        content.as_bytes(),
        spec,
        &mut stack,
        &mut c,
    );
    c.field_inits
}

fn ident_text_rows(node: Node, src: &[u8]) -> Vec<(usize, String)> {
    let mut out = vec![];
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        if lang::is_ident(ch.kind()) {
            if let Ok(t) = ch.utf8_text(src) {
                out.push((ch.start_position().row, t.to_string()));
            }
        }
        out.extend(ident_text_rows(ch, src));
    }
    out
}
