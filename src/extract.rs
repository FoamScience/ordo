//! Line-hunks (via `similar`) + per-hunk semantic extraction: category,
//! enclosing definition, defines/uses. Parses the *new* content once with
//! tree-sitter, exactly as gitplay's `order.lua` does.
use crate::lang::{self, LangSpec};
use crate::model::{Category, ContainerKind, Finding, Symbol};
use similar::TextDiff;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

#[derive(Clone)]
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
    /// the hunk lies entirely in `enclosing`'s header — above its body — so
    /// what it touched is the signature, even though the `def` line itself is
    /// outside the hunk (a multi-line parameter list, a return annotation)
    pub in_header: bool,
    pub defines: Vec<String>,
    /// subset of `defines` that a hunk introduces via *import* nodes — used for
    /// rationale wording so an import+def hunk doesn't call function names
    /// imports. On a deletion-only import hunk (no new side) it holds instead
    /// the names that left, filled in by `classify_imports`.
    pub imports: Vec<String>,
    pub uses: Vec<String>,
    /// the subset of `uses` this hunk only ever wrote as `obj.name` — see
    /// `lang::is_member_ident`. Never grounds a cross-file def→use edge.
    pub member_uses: Vec<String>,
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
    pub advisories: Vec<Finding>,
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
    /// the hunk covers the old file from its first line to its last: with an
    /// empty new side, the file is gone rather than trimmed
    pub whole_old_file: bool,
    /// how many new-side lines the hunk covers, so a hunk that introduces no
    /// construct can still say what it did instead of the bare word "change"
    pub new_len: usize,
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
            in_header: false,
            defines: vec![],
            imports: vec![],
            uses: vec![],
            member_uses: vec![],
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
            whole_old_file: false,
            new_len: h.new_r0.map_or(0, |r0| h.new_r1.saturating_sub(r0) + 1),
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
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            unreachable!("grouped_ops yields no empty group");
        };
        let (os, oe) = (first.old_range().start, last.old_range().end);
        let (ns, ne) = (first.new_range().start, last.new_range().end);
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

/// A pure insertion is one contiguous run of added lines, so a whole added file
/// arrives as a single hunk however much it holds: every construct in it shares
/// one card, one rationale line and one reviewed mark. Cut such a hunk at the
/// definitions inside it, so an added file reads construct by construct the way
/// an edited one does.
///
/// Language-independent: a new file is one diff hunk in every language.
fn split_insertions(defs: &[DefRec], hunks: &[RawHunk]) -> Vec<RawHunk> {
    let mut out = Vec::with_capacity(hunks.len());
    for h in hunks {
        // an edit's hunks already follow the change, and an insertion short
        // enough to read in one sitting is fine as one card — only a large one
        // needs cutting (LARGE_LINES is the same "too big to take in at once"
        // threshold the structural notes use).
        let inserted = h.old_range[0] > h.old_range[1];
        let Some(r0) = h
            .new_r0
            .filter(|r0| inserted && h.new_r1 - r0 > LARGE_LINES)
        else {
            out.push(h.clone());
            continue;
        };
        // every definition that starts inside the hunk, at any nesting depth: a
        // c++ namespace or a class holds the ones a reviewer actually reads, and
        // cutting only at the outermost construct would hand back one card.
        let mut cuts: Vec<usize> = defs
            .iter()
            .filter(|d| d.s > r0 && d.s <= h.new_r1)
            .map(|d| d.s)
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        if cuts.is_empty() {
            out.push(h.clone());
        } else {
            out.extend(split_at(h, r0, &cuts));
        }
    }
    out
}

/// Cut one hunk into consecutive pieces at `cuts` (0-based rows, all inside the
/// hunk). Every piece keeps the original's empty old range: each is still an
/// insertion at the same point in the old file.
fn split_at(h: &RawHunk, r0: usize, cuts: &[usize]) -> Vec<RawHunk> {
    let mut starts = vec![r0];
    starts.extend_from_slice(cuts);
    let mut out = Vec::with_capacity(starts.len());
    for (i, s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).map_or(h.new_r1, |next| next - 1);
        out.push(RawHunk {
            old_range: h.old_range,
            new_range: [s + 1, e + 1],
            new_r0: Some(*s),
            new_r1: e,
        });
    }
    out
}

#[derive(Default)]
struct Collected {
    import_rows: HashSet<usize>,
    def_rows: HashSet<usize>,
    type_rows: HashSet<usize>,
    defs: Vec<DefRec>,
    /// (start_row, end_row, name); a plain declaration has start == end, an
    /// import spans its whole statement (see `import_like`) stored once
    /// rather than once per row
    decls: Vec<(usize, usize, String)>,
    /// (row, name, tree-sitter kind, enclosing scope) for each real definition
    /// — the raw material for `symbols` (name+kind+scope identity)
    sym_decls: Vec<(usize, String, String, Option<String>)>,
    /// (start_row, end_row, name) per import name, spanning its whole
    /// statement: what a hunk anywhere in the statement can still report
    import_decls: Vec<(usize, usize, String)>,
    /// (row, name) for the languages that write one import per row (go), so a
    /// hunk that swaps one line of a ten-line block names that import alone
    import_rows_of: Vec<(usize, String)>,
    uses: Vec<(usize, String)>,
    /// (row, name) of the uses that were the member half of an access — see
    /// `lang::is_member_ident`. A subset of `uses`, kept apart so the edge
    /// graph can refuse a cross-file definition for a name that only ever
    /// appeared as `obj.name`
    member_uses: Vec<(usize, String)>,
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
    /// every identifier node in the file as (row, index into `uses`, node id)
    /// — the use search's raw material, kept separate from `uses` (which
    /// already feeds the def→use edge graph and must not gain binding-target
    /// entries). Always pushed immediately after the matching `uses` entry,
    /// so the index is `uses.len() - 1` at push time.
    all_idents: Vec<(usize, usize, usize)>,
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
    /// 0-based last row of the header, for a def that has a body: the
    /// declarator's end where the grammar has one (c, c++ — a constructor's
    /// initializer list sits between declarator and body and is neither),
    /// else the row before the `body` field. A hunk that stays within it
    /// touched the signature (parameters, return annotation, storage class).
    header_e: Option<usize>,
    /// `lang::has_signature` for the defining node — only a callable's header
    /// is a signature
    callable: bool,
}

// Structural-smell thresholds (P13.1) — change-shape signals, not style rules.
const LARGE_LINES: usize = 60;
const DEEP_NESTING: usize = 4;
const MANY_PARAMS: usize = 6;

/// Parse `new`, walk once, then classify each hunk. Returns None when the
/// grammar can't parse (caller falls back to file order).
/// Returns the hunks the review is built from — the caller's, cut where an
/// insertion covers more than one construct (see `split_insertions`) — paired
/// with one `HunkSem` each.
pub fn analyze(
    spec: &LangSpec,
    new: &str,
    hunks: &[RawHunk],
    path: &str,
) -> Option<(Vec<RawHunk>, Vec<HunkSem>)> {
    let tree = lang::parse(spec, new)?;
    let src = new.as_bytes();
    let mut w = Walker::new(spec, src);
    w.walk(tree.root_node());
    let adv = crate::advisories::advise(spec, tree.root_node(), src, path);
    let hunks = split_insertions(&w.c.defs, hunks);
    let parsed = Parsed {
        c: &w.c,
        spec,
        lines: new.lines().collect(),
        adv: &adv,
        decl_at_row: w
            .c
            .decls
            .iter()
            .flat_map(|(s, e, n)| (*s..=*e).map(move |r| (r, n.as_str())))
            .collect(),
    };
    let out = hunks
        .iter()
        .map(|h| match h.new_r0 {
            Some(r0) => parsed.hunk_sem(h, r0, h.new_r1),
            None => HunkSem::other(h),
        })
        .collect();
    Some((hunks, out))
}

/// One parsed file after the walk: what every hunk's facts are read from.
struct Parsed<'a> {
    c: &'a Collected,
    spec: &'a LangSpec,
    lines: Vec<&'a str>,
    /// (row, finding) from the advisory pass
    adv: &'a [(usize, Finding)],
    /// (row, name) for every row a declaration spans, so a binding on a def's
    /// own row is not reported a second time as a local
    decl_at_row: HashSet<(usize, &'a str)>,
}

/// The measurements a rule can put its own limit on, for the definitions a
/// hunk starts.
struct DefShape {
    lines: usize,
    params: usize,
    nesting: usize,
    recursive: bool,
}

fn sorted_unique<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v.dedup();
    v
}

impl Parsed<'_> {
    /// the facts for the hunk covering new-side rows `r0..=r1`
    fn hunk_sem(&self, h: &RawHunk, r0: usize, r1: usize) -> HunkSem {
        let container = self.enclosing_def(r0, r1);
        let enclosing = container.map(|d| d.name.clone());
        let (defines, imports) = self.declared(r0, r1);
        let shape = self.def_shape(r0, r1);
        HunkSem {
            category: self.category(r0, r1),
            // a plain definition is the default and says nothing extra; only a
            // region (see `ContainerKind`) is worth reporting
            enclosing_kind: container
                .map(|d| d.kind)
                .filter(|k| *k != ContainerKind::Definition),
            in_header: container.is_some_and(|d| d.callable && d.header_e.is_some_and(|e| r1 <= e)),
            uses: self.uses(r0, r1, &defines, &imports),
            member_uses: self.member_uses(r0, r1),
            is_type: (r0..=r1).any(|r| self.c.type_rows.contains(&r)),
            noise: false,
            members: sorted_unique(
                self.c
                    .member_rows
                    .iter()
                    .filter(|(row, _, _, _)| r0 <= *row && *row <= r1)
                    .map(|(_, n, t, ctr)| (n.clone(), t.clone(), ctr.clone()))
                    .collect(),
            ),
            details: vec![],
            notes: self.notes(r0, r1),
            advisories: self.advisories(r0, r1),
            symbols: sorted_unique(
                self.c
                    .sym_decls
                    .iter()
                    .filter(|(row, _, _, _)| r0 <= *row && *row <= r1)
                    .map(|(_, name, kind, scope)| Symbol {
                        name: name.clone(),
                        kind: kind.clone(),
                        scope: scope.clone(),
                    })
                    .collect(),
            ),
            import_moved: false,
            priority: 0,
            bindings: self.bindings(r0, r1),
            start_row: r0,
            old_range: h.old_range,
            // a hunk whose new side is nothing but blank lines has as little
            // to say for itself as a pure deletion, and the same wording fits:
            // what a reviewer wants to know is what left
            // A hunk whose new side is nothing but blank lines has as little
            // to say for itself as a pure deletion. With no new side to read
            // at all — a context-limited patch the caller called complete —
            // every hunk looked empty, and a file of real edits reported
            // itself removed line by line.
            new_empty: r1 < r0
                || (!self.lines.is_empty()
                    && (r0..=r1).all(|r| self.lines.get(r).is_none_or(|l| l.trim().is_empty()))),
            // filled in by `build_change`, which knows the old side's length
            whole_old_file: false,
            new_len: r1 - r0 + 1,
            def_lines: shape.lines,
            def_params: shape.params,
            nesting: shape.nesting,
            recursive: shape.recursive,
            container_members: self.container_members(r0, r1, &defines, &enclosing),
            // data members this hunk declares that nothing in this file
            // initializes; `lib` widens the check to every file in the change
            uninit_members: self
                .c
                .uninit_fields
                .iter()
                .filter(|(row, n)| r0 <= *row && *row <= r1 && !self.c.field_inits.contains(n))
                .map(|(_, n)| n.clone())
                .collect(),
            defines,
            imports,
            enclosing,
        }
    }

    // Definition wins over Import: a hunk that adds real defs (e.g. a whole
    // new file, or an import block followed by functions) is a definition
    // hunk, not an import hunk — only a hunk that is *only* imports is Import.
    fn category(&self, r0: usize, r1: usize) -> Category {
        if (r0..=r1).any(|r| self.c.def_rows.contains(&r)) {
            Category::Definition
        } else if (r0..=r1).any(|r| self.c.import_rows.contains(&r)) {
            Category::Import
        } else {
            Category::Other
        }
    }

    /// the innermost container the hunk sits in
    fn enclosing_def(&self, r0: usize, r1: usize) -> Option<&DefRec> {
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
        if self.spec.prose {
            return self
                .c
                .defs
                .iter()
                .filter(|d| d.s <= r1 && r1 <= d.e)
                // a *definition* that starts inside the hunk is what the hunk
                // adds, not what contains it. A region can't be added that way
                // — a document has one preamble whether or not this hunk
                // touched its first line — so it stays eligible.
                .filter(|d| d.kind != ContainerKind::Definition || !(r0 <= d.s && d.s <= r1))
                .min_by_key(|d| d.e - d.s);
        }
        let at = |row: usize| {
            self.c
                .defs
                .iter()
                .filter(|d| d.s <= row && row <= d.e)
                .min_by_key(|d| d.e - d.s)
        };
        // a hunk that starts on a blank line between two containers owns
        // no row of either; its first line with something on it does
        at(r0).or_else(|| {
            // rows here are 0-based (see `RawHunk::new_r0`)
            let first_real =
                (r0..=r1).find(|r| self.lines.get(*r).is_some_and(|l| !l.trim().is_empty()))?;
            at(first_real)
        })
    }

    /// what the hunk defines and what it imports, the first without the
    /// second: an import is not a definition, so an importing hunk can't act
    /// as a def→use edge source (`uses` still excludes imported names)
    fn declared(&self, r0: usize, r1: usize) -> (Vec<String>, Vec<String>) {
        let names = |rows: &[(usize, usize, String)]| {
            sorted_unique(
                rows.iter()
                    .filter(|(s, e, _)| *s <= r1 && r0 <= *e)
                    .map(|(_, _, n)| n.clone())
                    .collect(),
            )
        };
        let mut defines = names(&self.c.decls);
        // Every name the statement binds, whichever row the hunk touched: an
        // imported name is never a definition, and leaving the ones this hunk
        // does not report in `defines` made a file that imports `createApp`
        // the definer of it for every other file in the change.
        let statement = names(&self.c.import_decls);
        defines.retain(|d| !statement.contains(d));
        // What the hunk reports is narrower: an import written on its own row
        // is what a hunk touching that row is about, and the statement's
        // other names are context. With none of those rows in range — a hunk
        // on the block's brace, or on a comment inside it — the statement
        // answers instead, rather than a bare "import" that tells a reviewer
        // nothing.
        let own_rows = sorted_unique(
            self.c
                .import_rows_of
                .iter()
                .filter(|(row, _)| (r0..=r1).contains(row))
                .map(|(_, n)| n.clone())
                .collect(),
        );
        let imports = if own_rows.is_empty() {
            statement
        } else {
            own_rows
        };
        (defines, imports)
    }

    // what the hunk refers to, less what it declares itself — imports
    // included, an imported name is not a use of it — and the file's
    // parameter names
    fn uses(&self, r0: usize, r1: usize, defines: &[String], imports: &[String]) -> Vec<String> {
        // An imported name is not a use of it: `import x` says where x comes
        // from, not that this hunk calls it. The hunk that imports AND calls
        // it in one insertion is linked on the edge side instead, from its
        // `imports` — see `order::group_symbols`.
        let declared: HashSet<&String> = defines.iter().chain(imports).collect();
        sorted_unique(
            self.c
                .uses
                .iter()
                .filter(|(row, _)| (r0..=r1).contains(row))
                .map(|(_, n)| n.clone())
                .filter(|n| !declared.contains(n) && !self.c.bound.contains(n))
                .collect(),
        )
    }

    /// Names this hunk wrote only as the member half of an access. A name it
    /// also wrote bare is left out: one plain mention is enough for the edge
    /// graph to take it as a real reference.
    fn member_uses(&self, r0: usize, r1: usize) -> Vec<String> {
        // every member occurrence is also in `uses`, so a name is member-only
        // when the two counts agree
        fn count(v: &[(usize, String)], r0: usize, r1: usize) -> HashMap<&str, usize> {
            let mut m: HashMap<&str, usize> = HashMap::new();
            for (_, n) in v.iter().filter(|(row, _)| (r0..=r1).contains(row)) {
                *m.entry(n.as_str()).or_default() += 1;
            }
            m
        }
        let all = count(&self.c.uses, r0, r1);
        sorted_unique(
            count(&self.c.member_uses, r0, r1)
                .into_iter()
                .filter(|(n, k)| all.get(n) == Some(k))
                .map(|(n, _)| n.to_string())
                .collect(),
        )
    }

    // P13.1: structural smells for a def introduced in this hunk — one note
    // per kind, carrying the worst measurement, because a hunk that
    // introduces forty definitions has one nesting problem to report, not
    // forty of them. A data or prose format has no code shape to measure:
    // every JSON key is a `pair`, so an object literal used to say "deeply
    // nested" once per key.
    fn notes(&self, r0: usize, r1: usize) -> Vec<String> {
        let mut notes = vec![];
        if self.spec.data || self.spec.prose {
            return notes;
        }
        let (mut lines, mut depth, mut params) = (0, 0, 0);
        for d in self.c.defs.iter().filter(|d| r0 <= d.s && d.s <= r1) {
            lines = lines.max(d.e - d.s + 1);
            depth = depth.max(d.depth);
            params = params.max(d.params);
        }
        if lines >= LARGE_LINES {
            notes.push(format!("large definition ({lines} lines)"));
        }
        if depth >= DEEP_NESTING {
            notes.push(format!("deeply nested (depth {depth})"));
        }
        if params >= MANY_PARAMS {
            notes.push(format!("{params} params"));
        }
        notes
    }

    // the same measurements as `notes`, as facts a rule can put its own limit on
    fn def_shape(&self, r0: usize, r1: usize) -> DefShape {
        let started: Vec<&DefRec> = self
            .c
            .defs
            .iter()
            .filter(|d| r0 <= d.s && d.s <= r1 && d.kind == ContainerKind::Definition)
            .collect();
        DefShape {
            lines: started.iter().map(|d| d.e - d.s + 1).max().unwrap_or(0),
            params: started.iter().map(|d| d.params).max().unwrap_or(0),
            nesting: (r0..=r1)
                .filter_map(|r| self.c.nest_rows.get(&r))
                .copied()
                .max()
                .unwrap_or(0),
            // a definition that names itself inside its own body — over and
            // above the declaring identifier, which `uses` also carries
            recursive: started.iter().any(|d| {
                let declared = self
                    .c
                    .decls
                    .iter()
                    .filter(|(s, e, n)| *s <= d.s && d.s <= *e && *n == d.name)
                    .count();
                let named = self
                    .c
                    .uses
                    .iter()
                    .filter(|(row, n)| d.s <= *row && *row <= d.e && *n == d.name)
                    .count();
                named > declared
            }),
        }
    }

    // the container a defined symbol lives in (its scope), else the hunk's
    // enclosing one; its members are every symbol declared with that scope
    // plus the detail layer's members of it
    fn container_members(
        &self,
        r0: usize,
        r1: usize,
        defines: &[String],
        enclosing: &Option<String>,
    ) -> Vec<String> {
        let container = defines
            .first()
            .and_then(|d| {
                self.c
                    .sym_decls
                    .iter()
                    .find(|(row, n, _, _)| r0 <= *row && *row <= r1 && n == d)
                    .and_then(|(_, _, _, scope)| scope.clone())
            })
            .or_else(|| enclosing.clone());
        let Some(cn) = container else {
            return vec![];
        };
        let declared = self
            .c
            .sym_decls
            .iter()
            .filter(|(_, _, _, scope)| scope.as_deref() == Some(cn.as_str()))
            .map(|(_, n, _, _)| n.clone());
        let members = self
            .c
            .member_rows
            .iter()
            .filter(|(_, _, _, key)| key.as_deref() == Some(cn.as_str()))
            .map(|(_, n, _, _)| n.clone());
        sorted_unique(declared.chain(members).collect())
    }

    // One hunk, one mention: a hunk holding eighteen `goto`s used to carry
    // the same advisory eighteen times, and the why pane printed all of
    // them. The construct is the finding; the count is not, and the rules
    // engine has always reported once per hunk.
    //
    // Identical in every field, message included — `metaclass` says
    // something different depending on what the class overrides, and two
    // of those in one hunk are two things to read, not one repeated.
    fn advisories(&self, r0: usize, r1: usize) -> Vec<Finding> {
        let mut seen: Vec<&Finding> = vec![];
        self.adv
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
            .map(|(_, a)| a)
            .filter(|a| {
                let fresh = !seen
                    .iter()
                    .any(|s| s.name == a.name && s.level == a.level && s.message == a.message);
                if fresh {
                    seen.push(a);
                }
                fresh
            })
            .cloned()
            .collect()
    }

    // P17: local bindings this hunk introduces, and where each is used
    // elsewhere in the file. A binding whose row also holds a real def
    // (e.g. lua's `local f = function() end`, named via `bound_name`)
    // is already reported as that def — skip it here to avoid saying
    // the same thing twice. `_` (and other placeholder-only names) carry
    // no navigational signal, same reasoning `rationale_for` already
    // applies to `defines` — drop them here too rather than passing a
    // dead entry through to the rationale layer.
    // several `locals`-kind nodes reassigning the same name within one
    // hunk (a variable rebound across a loop body, tuple-unpacked twice,
    // …) must collapse into one entry — otherwise the same name/use-list
    // gets reported multiple times, ballooning the rationale.
    fn bindings(&self, r0: usize, r1: usize) -> Vec<BindingUse> {
        let c = self.c;
        let mut bindings: Vec<BindingUse> = vec![];
        for (row, name) in c
            .local_binds
            .iter()
            .filter(|(row, _)| r0 <= *row && *row <= r1)
        {
            if name == "_" || self.decl_at_row.contains(&(*row, name.as_str())) {
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
                .filter(|(r, i, id)| {
                    c.uses[*i].1 == *name && *r >= lo && *r <= hi && !c.bind_ids.contains(id)
                })
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
        bindings
    }
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
    if node.child_by_field_name("declaration").is_some()
        || node.child_by_field_name("value").is_some()
    {
        return false;
    }
    // `export {};` re-exports nothing: it is the marker that makes a file a
    // module, and in a `.d.ts` sweep it is the whole change — a reviewer has
    // to see it, so it is neither import nor noise
    let mut cur = node.walk();
    let empty_clause = node
        .named_children(&mut cur)
        .any(|c| c.kind() == "export_clause" && c.named_child_count() == 0);
    !empty_clause
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
    if node.kind() == "macro_invocation" {
        return test_macro_label(node, src, spec);
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
// rust names tests through a macro rather than a call: `rgtest!(name, |…| {…})`,
// `test_case!(…)`. The name is an identifier, not a string, and the entry is
// matched as a *substring* of the macro name so one entry ("test") covers the
// family. Verified against tree-sitter-rust-0.23.3: `macro_invocation` has a
// `macro` field and a `token_tree` holding the arguments.
fn test_macro_label(node: Node, src: &[u8], spec: &LangSpec) -> Option<String> {
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
    Some(format!("{name}! {label}"))
}

fn is_test_label(s: &str) -> bool {
    s.split_once(' ')
        .is_some_and(|(head, rest)| rest.starts_with(['"', '\'', '`']) && !head.is_empty())
}

/// A single-file component's `<script>` block holds real code in another
/// language. It is parsed with that grammar and recorded as **uses only**, the
/// same contract as an injected markdown fence — see `inject_fence` for why
/// that contract exists, and `docs/document-languages-design.md` for why an
/// SFC does not get definitions out of it: `walk` reads file rows at some
/// twenty sites and eight of `extract`'s ten parse entry points are old-side
/// collectors, so teaching only `analyze` about injected defs would make every
/// function in every component read as newly added on every commit.
///
/// `<style>` is deliberately not injected. Injection harvests every identifier
/// as a use, and a stylesheet's identifiers are its *definitions* — doing it
/// would contribute nothing and would flood `uses` with exactly the
/// `class_name` leak the css selector guard exists to prevent.
fn inject_sfc_script(node: Node, src: &[u8], c: &mut Collected) {
    let mut cur = node.walk();
    let Some(body) = node
        .named_children(&mut cur)
        .find(|n| n.kind() == "raw_text")
    else {
        return;
    };
    // `<script lang="ts">` picks typescript; anything else is javascript,
    // which also parses the plain-js majority correctly
    let mut tc = node.walk();
    let declared = node
        .named_children(&mut tc)
        .find(|n| matches!(n.kind(), "start_tag" | "self_closing_tag"))
        .and_then(|t| {
            let mut ac = t.walk();
            let found = t.named_children(&mut ac).find_map(|a| {
                let mut pc = a.walk();
                let parts: Vec<Node> = a.named_children(&mut pc).collect();
                let is_lang = parts
                    .first()
                    .and_then(|n| n.utf8_text(src).ok())
                    .is_some_and(|t| t.eq_ignore_ascii_case("lang"));
                is_lang
                    .then(|| parts.get(1).and_then(|v| v.utf8_text(src).ok()))
                    .flatten()
            });
            found
        })
        .map(|t| unquote(t.trim()).to_string());
    let inner = declared
        .as_deref()
        .and_then(lang::for_lang_name)
        .or_else(|| lang::for_lang_name("javascript"));
    let (Some(inner), Ok(text)) = (inner, body.utf8_text(src)) else {
        return;
    };
    let Some(tree) = lang::parse(inner, text) else {
        return;
    };
    // rows inside the block are relative to it; report them in the file's own
    // coordinates so a hunk lines up with them
    let offset = body.start_position().row;
    collect_injected_uses(tree.root_node(), text.as_bytes(), offset, c);
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
    // Every blanked region is a whole tree-sitter node, so a multi-byte
    // character is never cut in half and the bytes stay valid UTF-8. That is an
    // invariant across two functions and a grammar, though, and this runs on
    // whatever text the caller sent: masking nothing beats aborting `run`.
    let text = String::from_utf8(out).ok()?;
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

/// The macro name an include guard introduces, when this `preproc_def` is the
/// `#define X` half of one: the conditional directly above it tested the same
/// name, at file scope. The pair is bookkeeping — `X` names nothing a reviewer
/// navigates to and nothing another file uses — so neither half becomes a
/// container, a definition or a use.
///
/// Keyed on the `#define` rather than the `#ifndef` because a header whose body
/// defeats the c++ parser comes back as one `ERROR` node with the guard's two
/// directives flattened into it, and the name equality still holds there.
///
/// c/c++ only in practice: no other grammar in this crate produces
/// `preproc_def` (verified against tree-sitter-{c,cpp}-0.23.4).
fn include_guard_name(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() != "preproc_def" {
        return None;
    }
    let parent = node.parent()?;
    let at_file_scope = match parent.kind() {
        "translation_unit" => true,
        // a header whose body defeats the parser yields one top-level `ERROR`
        // holding the flattened directives; deeper down, an ERROR says nothing
        // about scope
        "ERROR" => parent
            .parent()
            .is_some_and(|g| g.kind() == "translation_unit"),
        "preproc_ifdef" => parent
            .parent()
            .is_some_and(|g| g.kind() == "translation_unit"),
        _ => false,
    };
    if !at_file_scope {
        return None;
    }
    let text = |n: Node| n.utf8_text(src).ok().map(str::trim);
    let name = text(node.child_by_field_name("name")?)?;
    let tested = prev_directive(node)
        .filter(|s| s.kind() == "identifier")
        .and_then(text)?;
    (tested == name).then(|| name.to_string())
}

/// The named sibling before `node`, skipping comments: a guard commonly carries
/// one between its two halves, and a comment is a named node.
fn prev_directive(node: Node) -> Option<Node> {
    let mut prev = node.prev_named_sibling();
    while prev.is_some_and(|p| p.kind() == "comment") {
        prev = prev?.prev_named_sibling();
    }
    prev
}

/// Does this `preproc_ifdef` open an include guard? Its `#define` is the first
/// directive after the name it tests, not merely somewhere inside.
fn opens_include_guard(node: Node, src: &[u8]) -> bool {
    let mut cur = node.walk();
    let first_directive = node
        .named_children(&mut cur)
        .skip(1)
        .find(|c| c.kind() != "comment");
    first_directive.is_some_and(|d| include_guard_name(d, src).is_some())
}

/// c and c++ have no nested functions, so a `function_definition` inside a
/// function body is the grammar's reading of a call whose last argument is a
/// macro wrapping a lambda — `Kokkos::parallel_for(n, KOKKOS_LAMBDA(int i){…})`
/// parses as a definition named after the callee. A local class's methods are a
/// real nesting, so the climb stops at any class, struct or namespace body.
/// A def kind that is actually defining something here. C and C++ spell a
/// mention of a type with the same node as its definition — `struct Curl_easy
/// *data` in a parameter list is a `struct_specifier` too — and only the one
/// with a body defines anything; the rest used to make every prototype taking
/// a struct pointer "change type Curl_easy".
fn is_def_node(node: Node, spec: &LangSpec) -> bool {
    if !matches!(spec.name, "c" | "cpp") {
        return spec.is_def(node.kind());
    }
    match node.kind() {
        // a specifier with a body defines a type; one without is a mention —
        // `struct Curl_easy *data` in a parameter list — unless it stands as
        // a declaration of its own, which is a forward declaration
        "struct_specifier" | "enum_specifier" | "union_specifier" | "class_specifier" => {
            node.child_by_field_name("body").is_some() || is_forward_declaration(node)
        }
        // a file-scope prototype declares the function it names: a header's
        // `int f(struct S *s, int8_t i);` changing is a signature change of
        // `f`, which is what a reviewer reads it as
        "declaration" => is_prototype(node),
        kind => spec.is_def(kind),
    }
}

/// The kind a def records itself under. A c/cpp prototype is spelled
/// `declaration`, a kind css also uses for a style property and half a dozen
/// grammars use for something else again; it records what it is instead, so
/// the wording rules can read the kind without knowing the language.
fn def_kind(node: Node, spec: &LangSpec) -> Option<&'static str> {
    let prototype =
        matches!(spec.name, "c" | "cpp") && node.kind() == "declaration" && is_prototype(node);
    prototype.then_some("function_declaration")
}

/// `struct Opaque;` — a specifier standing as a statement of its own, naming
/// the type and nothing else. C parses it straight under the scope it sits
/// in; C++ wraps it in a `declaration`. `struct S x;` declares a variable and
/// `struct S *s` a parameter: those mention the type, they do not declare it.
fn is_forward_declaration(node: Node) -> bool {
    node.parent().is_some_and(|p| match p.kind() {
        "translation_unit"
        | "declaration_list"
        | "field_declaration_list"
        | "linkage_specification" => true,
        "declaration" | "field_declaration" => p.child_by_field_name("declarator").is_none(),
        _ => false,
    })
}

/// A `declaration` whose declarator is (or wraps) a `function_declarator`, at
/// file or namespace scope: a prototype, not a variable.
fn is_prototype(node: Node) -> bool {
    // file scope, seen through the `#ifndef` guard and `#if` blocks a header
    // wraps its prototypes in
    let mut p = node.parent();
    loop {
        match p.map(|n| n.kind()) {
            Some(
                "translation_unit"
                | "namespace_definition"
                | "declaration_list"
                | "linkage_specification",
            ) => break,
            Some(k) if k.starts_with("preproc_") => p = p.and_then(|n| n.parent()),
            _ => return false,
        }
    }
    let mut d = node.child_by_field_name("declarator");
    while let Some(n) = d {
        if n.kind() == "function_declarator" {
            return declares_parameters(n);
        }
        d = n.child_by_field_name("declarator").or_else(|| {
            (n.kind() == "reference_declarator")
                .then(|| n.named_child(0))
                .flatten()
        });
    }
    false
}

fn is_misparsed_call(node: Node, spec: &LangSpec) -> bool {
    if !matches!(spec.name, "c" | "cpp") || node.kind() != "function_definition" {
        return false;
    }
    let mut parent = node.parent();
    while let Some(n) = parent {
        match n.kind() {
            "compound_statement" => return true,
            "field_declaration_list"
            | "declaration_list"
            | "class_specifier"
            | "struct_specifier"
            | "namespace_definition"
            | "translation_unit" => return false,
            _ => parent = n.parent(),
        }
    }
    false
}

fn region_label(
    node: Node,
    src: &[u8],
    spec: &LangSpec,
    top_level: bool,
) -> Option<(String, ContainerKind)> {
    // the two halves match disjoint node kinds, so a kind the first knows but
    // cannot name never reaches the second
    code_region(node, src, top_level).or_else(|| markup_region(node, src, spec))
}

/// a region named by a piece of source text, when there is any
fn region(text: String) -> Option<(String, ContainerKind)> {
    (!text.is_empty()).then_some((text, ContainerKind::Region))
}

/// The regions a code language has: preprocessor conditionals, and a
/// script's top-level `with` block or command line.
fn code_region(node: Node, src: &[u8], top_level: bool) -> Option<(String, ContainerKind)> {
    match node.kind() {
        // `#ifdef X` and `#ifndef X` share a node kind; the directive token
        // itself says which, and a reviewer reads them very differently
        "preproc_ifdef" if !opens_include_guard(node, src) => {
            let name = node.child_by_field_name("name")?.utf8_text(src).ok()?;
            let name = tidy_ident(name);
            if name.is_empty() {
                return None;
            }
            let directive = node
                .child(0)
                .and_then(|d| d.utf8_text(src).ok())
                .map(|t| t.trim().to_string())
                .unwrap_or_else(|| "#ifdef".to_string());
            Some((format!("{directive} {name}"), ContainerKind::Region))
        }
        // a condition is an expression, not an identifier: collapse runs of
        // whitespace but keep the single spaces that make it readable
        "preproc_if" => {
            let cond = node.child_by_field_name("condition")?.utf8_text(src).ok()?;
            region(format!("#if {}", squeeze(cond)))
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
            region(format!("with {}", squeeze(text)))
        }
        // xonsh: a command line at file scope is the script's actual work, and
        // it is not a definition, a binding or a call node any other language
        // has. Name it by the command and its subcommand words —
        // `pip cache remove '*x*' || true` reads as "edits pip cache remove".
        "bare_subprocess" | "uncaptured_subprocess" if top_level => {
            let label = subprocess_label(node, src)?;
            Some((label, ContainerKind::Call))
        }
        _ => None,
    }
}

/// The regions a markup, stylesheet or config format has: a yaml document,
/// a svelte block, a `<script>`/`<style>` element, an at-rule, front matter,
/// a document's preamble.
fn markup_region(node: Node, src: &[u8], spec: &LangSpec) -> Option<(String, ContainerKind)> {
    match node.kind() {
        // yaml: a stream can hold several `---` documents whose top-level keys
        // collide — two k8s objects each own a `spec`, and `spec.replicas`
        // alone does not say which. Name each document by its position, but
        // only when there is more than one: a single-document file keeps the
        // paths it has always had. A region, not a definition: an ordinal is
        // where a thing sits, never a symbol anything can use or define.
        "document" if node.parent().is_some_and(|p| p.kind() == "stream") => {
            let n = yaml_document_ordinal(node)?;
            Some((format!("document {n}"), ContainerKind::Document))
        }
        // svelte's own block forms, named as written: `{#if n > 1}`,
        // `{#each items as it}`, `{:else}`. Regions, like `#ifdef` — they hold
        // markup but declare nothing. Gated on the language because
        // `if_statement` is a kind seven other grammars here also produce;
        // `{#snippet}` is deliberately absent, being a real definition.
        "if_statement" | "else_if_block" | "else_block" | "each_statement" | "await_statement"
        | "key_statement"
            if spec.name == "svelte" =>
        {
            let start = child_where(node, |c| c.kind().ends_with("_start"))?;
            region(squeeze(start.utf8_text(src).ok()?))
        }
        // `<script setup>`, `<style scoped>`, `<style module lang="scss">` —
        // what a reviewer actually calls these blocks. A region, not a
        // definition: the block declares nothing itself, whatever its contents
        // do. Both kinds are unique to html and svelte among shipped grammars.
        "script_element" | "style_element" => {
            let tag = child_where(node, |n| {
                matches!(n.kind(), "start_tag" | "self_closing_tag")
            })?;
            region(squeeze(tag.utf8_text(src).ok()?))
        }
        // `@media (min-width: 700px)` is `#ifdef` in a different hat: a real
        // container worth naming that declares nothing.
        "media_statement" | "supports_statement" => {
            let head = child_where(node, |c| c.kind() != "block")?;
            let head = squeeze(head.utf8_text(src).ok()?);
            let at = if node.kind() == "media_statement" {
                "@media"
            } else {
                "@supports"
            };
            (!head.is_empty()).then(|| (format!("{at} {head}"), ContainerKind::Region))
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

/// One pass over a parse tree. `stack` holds the names of the definitions
/// enclosing the current node, innermost last; `c` collects what every hunk
/// is later read against. Each `visit_*` below handles one shape a node can
/// have — an import, a region, a test block, a definition, a member, a
/// language's own way of spelling a reference. One that may end the walk at
/// this node returns whether it did (having descended itself, or having
/// decided the subtree holds nothing to collect); one that only records and
/// always falls through returns nothing. `walk_node` runs them in precedence
/// order and descends for whatever is left.
struct Walker<'a> {
    src: &'a [u8],
    spec: &'a LangSpec,
    stack: Vec<String>,
    c: Collected,
}

impl Collected {
    /// a reference to `name` on `row`, from identifier node `id`
    fn push_use(&mut self, row: usize, name: String, id: usize) {
        self.uses.push((row, name));
        self.all_idents.push((row, self.uses.len() - 1, id));
    }

    /// the same, for an identifier that was the member half of an access
    fn push_member_use(&mut self, row: usize, name: String, id: usize) {
        self.member_uses.push((row, name.clone()));
        self.push_use(row, name, id);
    }

    /// a one-row declaration of `name` with symbol identity
    fn push_decl(&mut self, row: usize, name: &str, kind: &str, scope: Option<String>) {
        self.decls.push((row, row, name.to_string()));
        self.def_rows.insert(row);
        self.sym_decls
            .push((row, name.to_string(), kind.to_string(), scope));
    }
}

/// A definition's (header, body, body lines): the header is everything before
/// the body (the signature); the body text drives rename/relocation matching.
/// Both fall back to the whole node when it does not split.
fn body_parts(node: Node, src: &[u8], spec: &LangSpec) -> (String, String, Vec<String>) {
    let full = node.utf8_text(src).unwrap_or("");
    // `value` is the body under another name: a macro (`preproc_def`,
    // `preproc_function_def`) and a rust `const_item`/`static_item` hold
    // theirs there, and without this every change to one reads as a
    // signature change because the header would be the whole node.
    let body = node
        .child_by_field_name("body")
        .or_else(|| node.child_by_field_name("value"))
        // css labels no field: a `rule_set`'s declarations are a `block`
        // child. Without this the header is the whole rule, so every
        // declaration edit reads as a change to the selector itself and a
        // renamed selector never matches its old body. Gated to css so no
        // shipped language moves — cmake's `function_def` has an unlabelled
        // `body` child too.
        .or_else(|| {
            if spec.name != "css" {
                return None;
            }
            let mut cur = node.walk();
            let found = node.named_children(&mut cur).find(|c| c.kind() == "block");
            found
        });
    let header = match body {
        Some(b) => &full[..(b.start_byte() - node.start_byte()).min(full.len())],
        None => full,
    };
    let text = body.and_then(|b| b.utf8_text(src).ok()).unwrap_or(full);
    let lines = text
        .lines()
        .map(squeeze)
        .filter(|l| l.chars().filter(|c| c.is_alphanumeric()).count() >= 3)
        .collect();
    (squeeze(header), squeeze(text), lines)
}

/// The node's text, trimmed, when it has any.
fn trimmed<'s>(node: Node, src: &'s [u8]) -> Option<&'s str> {
    node.utf8_text(src)
        .ok()
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// See `DefRec::header_e`.
fn header_end(node: Node) -> Option<usize> {
    node.child_by_field_name("declarator")
        .map(|d| d.end_position().row)
        .or_else(|| {
            node.child_by_field_name("body")
                .and_then(|b| b.start_position().row.checked_sub(1))
        })
}

/// Whether a `function_declarator`'s parentheses hold parameters rather than
/// constructor arguments. C++'s most vexing parse spells `std::mutex m(a, b);`
/// exactly like a prototype; its "parameters" are bare type identifiers with
/// nothing declared after them, where a real prototype either names what it
/// declares or spells a built-in type.
fn declares_parameters(declarator: Node) -> bool {
    let Some(params) = declarator.child_by_field_name("parameters") else {
        return false;
    };
    let mut cur = params.walk();
    let declared = params.named_children(&mut cur).all(|p| {
        p.kind() != "parameter_declaration"
            || p.child_by_field_name("declarator").is_some()
            || p.child_by_field_name("type")
                .is_some_and(|t| t.kind() != "type_identifier")
    });
    declared
}

/// The first named child `pred` accepts.
fn child_where<'t>(node: Node<'t>, pred: impl Fn(&Node<'t>) -> bool) -> Option<Node<'t>> {
    let mut cur = node.walk();
    let found = node.named_children(&mut cur).find(pred);
    found
}

/// yaml: a stream can hold several `---` documents whose top-level keys
/// collide — two k8s objects each own a `spec`, and `spec.replicas` alone
/// does not say which. Each document is numbered by its position, but only
/// when there is more than one: a single-document file keeps the paths it
/// has always had.
fn yaml_document_ordinal(node: Node) -> Option<usize> {
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
    docs.iter().position(|id| *id == node.id()).map(|i| i + 1)
}

/// Everything the walker collects over `content`, when it parses.
fn walk_file(spec: &LangSpec, content: &str) -> Option<Collected> {
    let tree = lang::parse(spec, content)?;
    let mut w = Walker::new(spec, content.as_bytes());
    w.walk(tree.root_node());
    Some(w.c)
}

impl<'a> Walker<'a> {
    fn new(spec: &'a LangSpec, src: &'a [u8]) -> Self {
        Walker {
            src,
            spec,
            stack: vec![],
            c: Collected::default(),
        }
    }

    fn walk(&mut self, node: Node) {
        let control = CONTROL_KINDS.contains(&node.kind());
        if control {
            self.c.nest_depth += 1;
            for r in node.start_position().row..=node.end_position().row {
                let d = self.c.nest_rows.entry(r).or_insert(0);
                *d = (*d).max(self.c.nest_depth);
            }
        }
        self.walk_node(node);
        if control {
            self.c.nest_depth -= 1;
        }
    }

    fn walk_children(&mut self, node: Node) {
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            self.walk(ch);
        }
    }

    /// the enclosing definitions as one qualified name, if there are any
    fn scope(&self) -> Option<String> {
        (!self.stack.is_empty()).then(|| self.stack.join(lang::scope_sep(self.spec)))
    }

    /// a container that declares nothing of its own: a region, a test block,
    /// a call's argument list, a literal a binding holds
    fn container(&self, node: Node, name: String, depth: usize, kind: ContainerKind) -> DefRec {
        DefRec {
            s: node.start_position().row,
            e: end_row(node, self.spec),
            name,
            depth,
            params: 0,
            kind,
            header_e: None,
            callable: false,
        }
    }

    fn walk_node(&mut self, node: Node) {
        let (spec, kind) = (self.spec, node.kind());
        self.note_fields(node);
        if self.visit_import(node) {
            return;
        }
        self.visit_binding_container(node);
        // before the region visit, which names the `<script>` block and returns
        self.visit_sfc_script(node);
        if self.visit_region(node) || self.visit_test_block(node) {
            return;
        }
        self.visit_call_container(node);
        // the `#define` half of an include guard defines nothing: not a symbol,
        // not a use of one. Its `#ifndef` is already transparent (see
        // `region_label`).
        if include_guard_name(node, self.src).is_some() || self.visit_def(node) {
            return;
        }
        self.visit_member(node);
        self.visit_locals(node);
        self.visit_fence(node);
        // nix: an `attrpath` is a name being bound (`meta.description = …`) or
        // selected (`pkgs.gcc`) — never a free reference to something defined
        // elsewhere, so its identifiers are not uses. The binding's own name
        // is already filtered out of `uses` per hunk, but a dotted path is not.
        if spec.name == "nix" && kind == "attrpath" {
            return;
        }
        self.bind_nix_params(node);
        // a language's own way of spelling a reference, first one to claim
        // the node wins
        let references = [
            Self::visit_bash_command,
            Self::visit_make_words,
            Self::visit_svelte_render,
            Self::visit_css,
            Self::visit_yaml_anchor,
            Self::visit_ident,
        ];
        if references.iter().any(|visit| visit(self, node)) {
            return;
        }
        self.walk_children(node);
    }

    // Both sides are keyed by the class that owns the member, not by the bare
    // name: the declaration sits in `class B` and the initializer in `B::B`,
    // and the walker's scope stack reads `B` for each, so the header and the
    // `.cpp` still meet. Bare names let one class's initializer answer for
    // every same-named member in the changeset.
    fn note_fields(&mut self, node: Node) {
        if let Some(n) = field_init_name(node, self.src, self.spec) {
            self.c.field_inits.insert(owned_member(&self.stack, &n));
        }
        if let Some(name) = uninit_field(node, self.src, self.spec) {
            let row = node.start_position().row;
            self.c
                .uninit_fields
                .push((row, owned_member(&self.stack, &name)));
        }
    }

    fn visit_import(&mut self, node: Node) -> bool {
        if !import_like(node, self.src, self.spec) {
            return false;
        }
        // the whole statement's rows count as import — a hunk that lands
        // anywhere in a multi-line `from x import (\n  a,\n  b,\n)` (tail,
        // middle, or head) is still an import hunk, not a bare "change".
        let sr = node.start_position().row;
        let er = end_row(node, self.spec);
        for r in sr..=er {
            self.c.import_rows.insert(r);
        }
        // A name is attributed to the row it is written on and to the
        // statement's head and tail: a hunk that touches the tail of a
        // multi-line import list (`} from './y'`) still has the statement's
        // names to report, and "import" with nothing after it tells a reviewer
        // nothing — while a hunk that swaps one line of a go import block
        // names that import, not the nine around it.
        let names = import_bound_names(node, self.src, self.spec)
            .unwrap_or_else(|| ident_text_rows(node, self.src));
        for (row, name) in names {
            self.c.decls.push((sr, er, name.clone()));
            self.c.import_decls.push((sr, er, name.clone()));
            // a name the language reports on the statement's own row has no
            // row of its own (python's `from x import a, b`, js's clause)
            if row != sr {
                self.c.import_rows_of.push((row, name));
            }
        }
        true // don't descend: import identifiers are declarations, not uses
    }

    // container only: no `def_rows`, no `decls`, no symbol — see
    // `binding_container`. The value's own elements become members so the
    // detail layer can name what changed inside it. Falls through: the value
    // still holds locals, uses and nested defs.
    fn visit_binding_container(&mut self, node: Node) {
        let Some(name) = binding_container(node, self.src, self.spec, &self.stack) else {
            return;
        };
        for el in literal_elements(node) {
            // an element the language already treats as a member (a js/py
            // `pair`) is registered by the member branch, by its key — adding
            // its whole text here as a second member would report the same
            // change twice, once named and once as raw text
            if self.spec.is_member(el.kind()) {
                continue;
            }
            if let Ok(text) = el.utf8_text(self.src) {
                let text = squeeze(text);
                if !text.is_empty() {
                    let row = el.start_position().row;
                    self.c
                        .member_rows
                        .push((row, text.clone(), text, Some(name.clone())));
                }
            }
        }
        let rec = self.container(node, name, self.stack.len(), ContainerKind::Binding);
        self.c.defs.push(rec);
    }

    // a region names itself and nothing else: no `def_rows` (it declares
    // nothing, so a hunk in it is never a definition hunk), no `decls`
    // (nothing to add to `defines`), no symbol identity. Its own name is
    // still walked for uses, so `#ifdef CURL_DISABLE_HTTP` counts as a use of
    // that macro — which is exactly what it is.
    fn visit_region(&mut self, node: Node) -> bool {
        let Some((label, kind)) = region_label(node, self.src, self.spec, self.stack.is_empty())
        else {
            return false;
        };
        // A document is a namespace; every other region names only itself. An
        // `#ifdef` must not prefix the defs inside it — the definition is what
        // a reviewer navigates to, and the region is a fact about where it
        // sits. A `---` document is the opposite: the two `spec` keys of two
        // k8s objects are different keys, and the path has to say so.
        let scopes = kind == ContainerKind::Document;
        let mut rec = self.container(node, label, self.stack.len(), kind);
        if scopes {
            self.stack.push(rec.name.clone());
            rec.name = self.stack.join(lang::scope_sep(self.spec));
        }
        self.c.defs.push(rec);
        self.walk_children(node);
        if scopes {
            self.stack.pop();
        }
        true
    }

    // a named block, not a declaration: it gets a `defines` entry (so adding
    // or renaming one reads as such) but no symbol identity — a test name is
    // not a symbol another file can reference, and must never seed a def→use
    // edge or key a persisted review mark.
    fn visit_test_block(&mut self, node: Node) -> bool {
        let Some(label) = test_block_label(node, self.src, self.spec) else {
            return false;
        };
        let sr = node.start_position().row;
        let depth = self.stack.len();
        // a nested block replaces its parent's entry for the duration, so the
        // qualified name reads `describe "cli" > it "parses flags"` rather than
        // repeating the parent through the language's scope separator
        let parent = self.stack.last().filter(|s| is_test_label(s)).cloned();
        let label = match &parent {
            Some(p) => {
                self.stack.pop();
                format!("{p} > {label}")
            }
            None => label,
        };
        self.c.def_rows.insert(sr);
        self.c.decls.push((sr, sr, label.clone()));
        self.stack.push(label);
        let name = self.stack.join(lang::scope_sep(self.spec));
        let rec = self.container(node, name, depth, ContainerKind::Test);
        self.c.defs.push(rec);
        self.walk_children(node);
        self.stack.pop();
        if let Some(p) = parent {
            self.stack.push(p);
        }
        true
    }

    // falls through: the arguments still hold uses, members and defs
    fn visit_call_container(&mut self, node: Node) {
        if let Some(name) = call_statement_container(node, self.src, self.spec, &self.stack) {
            let rec = self.container(node, name, self.stack.len(), ContainerKind::Call);
            self.c.defs.push(rec);
        }
    }

    /// A single-file component's `<script>` is real code in another language.
    fn visit_sfc_script(&mut self, node: Node) {
        if matches!(self.spec.name, "html" | "svelte") && node.kind() == "script_element" {
            inject_sfc_script(node, self.src, &mut self.c);
        }
    }

    /// A markdown fence's code, read in its own language; the walk then goes
    /// on through the fence's own prose structure.
    fn visit_fence(&mut self, node: Node) {
        if self.spec.prose && node.kind() == "fenced_code_block" {
            inject_fence(node, self.src, &mut self.c);
        }
    }

    fn visit_def(&mut self, node: Node) -> bool {
        let (src, spec, kind) = (self.src, self.spec, node.kind());
        if !is_def_node(node, spec) || is_misparsed_call(node, spec) {
            return false;
        }
        // A def with no name of its own names no container, so it is
        // transparent: descend without pushing a scope. This covers a c++
        // anonymous `namespace {`, a lambda, and the content before a markdown
        // document's first heading — all of which would otherwise contribute an
        // `<anonymous>` segment to every enclosing name beneath them, and reach
        // the rationale. Their contents still nest under the nearest *named*
        // def, which is what a reviewer can actually navigate to.
        let Some(own) = node_name(node, src) else {
            self.walk_children(node);
            return true;
        };
        self.bind_params(node);
        let sr = node.start_position().row;
        // A c++ namespace is a scope, not a declaration: it qualifies the names
        // under it (which is why it is a def at all) but nothing references
        // `particode` the way it references a function, and every added file in
        // a project reopens the same one — as a definition it filled the ledger
        // and paired every pair of new files with a def→use edge.
        //
        // Deliberately only c++: a rust `mod` or a python module is declared
        // once, is imported by name, and is navigated to, so those stay
        // definitions. (`namespace_definition` is unique to the c++ grammar,
        // verified against tree-sitter-cpp-0.23.4.)
        let scope_only = kind == "namespace_definition";
        if !scope_only {
            self.record_def_rows(node, sr, &own);
        }
        self.record_member_def(node, sr, &own);
        // enclosing defs before this one
        let depth = self.stack.len();
        // collapse runs of nested defs sharing a name in the qualified enclosing
        // name: nested anonymous defs, and python's decorated_definition wrapper
        // whose resolved name (via the `definition` field) duplicates the
        // class/function it wraps.
        let dup = self.stack.last().is_some_and(|s| s == &own);
        if !scope_only {
            self.record_symbol(node, sr, &own, dup);
        }
        if !dup {
            self.stack.push(own);
        }
        self.c.defs.push(DefRec {
            s: sr,
            e: end_row(node, spec),
            name: self.stack.join(lang::scope_sep(spec)),
            depth,
            params: count_params(node),
            kind: if scope_only {
                ContainerKind::Namespace
            } else {
                ContainerKind::Definition
            },
            header_e: header_end(node),
            callable: lang::has_signature(kind),
        });
        self.walk_children(node);
        if !dup {
            self.stack.pop();
        }
        true
    }

    /// The rows a definition claims: its own, and as a type when it is one.
    fn record_def_rows(&mut self, node: Node, sr: usize, own: &str) {
        self.c.def_rows.insert(sr);
        if lang::is_type_kind(node.kind()) {
            self.c.type_rows.insert(sr);
        }
        self.c.decls.push((sr, sr, own.to_string()));
    }

    /// Prose and config only: a def kind that is *also* a member kind
    /// (markdown's `section`, a config format's key) registers itself as a
    /// member of its enclosing container too, so a new subsection or key shows
    /// up in the P15 detail layer. Gated on the language shape rather than on
    /// the overlap alone, because javascript's `method_definition` *is* in
    /// both sets and must keep today's behavior — see lang.rs's java comment
    /// on that same trap.
    fn record_member_def(&mut self, node: Node, sr: usize, own: &str) {
        let spec = self.spec;
        if !((spec.prose || spec.data) && spec.is_member(node.kind())) {
            return;
        }
        let text = node.utf8_text(self.src).map(squeeze).unwrap_or_default();
        // neither prose nor config has calls: the container is always the
        // enclosing def (`stack`, not yet pushed with `own` here).
        let container = self.scope();
        self.c
            .member_rows
            .push((sr, own.to_string(), text, container));
    }

    /// The symbol entry for a definition — unless it is a wrapper that
    /// delegates its name to an inner def (python's decorated_definition ->
    /// `definition` field): that is not itself the defining node, and the
    /// inner def it wraps gets the entry.
    fn record_symbol(&mut self, node: Node, sr: usize, own: &str, dup: bool) {
        let spec = self.spec;
        let delegates = node
            .child_by_field_name("definition")
            .is_some_and(|d| spec.is_def(d.kind()));
        if delegates {
            return;
        }
        let kind = def_kind(node, spec).unwrap_or_else(|| node.kind());
        // scope excludes a duplicate trailing entry (the wrapper's own push
        // for this same symbol, not a genuine enclosing scope)
        let scope_stack = if dup {
            &self.stack[..self.stack.len() - 1]
        } else {
            &self.stack[..]
        };
        let scope = (!scope_stack.is_empty()).then(|| scope_stack.join(lang::scope_sep(spec)));
        self.c
            .sym_decls
            .push((sr, own.to_string(), kind.to_string(), scope));
    }

    // parameter names are local bindings, not references to outer symbols —
    // record them so a param that shadows a def elsewhere (e.g. a `lane`
    // fixture used only via param injection) can't seed a def→use edge.
    // Two grammars spell the list without a `parameters` field: a jinja macro
    // hangs its parameters off the same `function_call` that carries its
    // name, everything after that leading identifier; cmake makes them the
    // command's remaining arguments, so `function(my_helper arg)` binds `arg`
    // and `${arg}` in the body is not read as a use of whatever else happens
    // to be called `arg`.
    fn bind_params(&mut self, node: Node) {
        let src = self.src;
        let declared = node
            .child_by_field_name("parameters")
            .map(|p| param_names(p, src))
            .unwrap_or_default();
        let names = declared
            .into_iter()
            .chain(jinja_macro_params(node, src))
            .chain(cmake_params(node, src));
        self.c.bound.extend(names);
    }

    // falls through: a member's value can still hold defs and uses
    fn visit_member(&mut self, node: Node) {
        if !self.spec.is_member(node.kind()) {
            return;
        }
        if let Some(name) = member_name(node, self.src) {
            let text = node.utf8_text(self.src).map(squeeze).unwrap_or_default();
            let container = member_container(node, self.src, &self.stack, self.spec);
            self.c
                .member_rows
                .push((node.start_position().row, name, text, container));
        }
    }

    // falls through: the bound value can still hold defs and uses
    fn visit_locals(&mut self, node: Node) {
        for (id, name) in bound_names(node, self.src, self.spec) {
            self.c.local_binds.push((id.start_position().row, name));
            self.c.bind_ids.insert(id.id());
        }
    }

    // a nix lambda binds its parameters: `{ pkgs, lib, ... }:` and `x: …`.
    // Not a definition, so the def branch's parameter handling never sees it.
    // Falls through: the body still holds bindings and uses.
    fn bind_nix_params(&mut self, node: Node) {
        if node.kind() != "function_expression" {
            return;
        }
        if let Some(f) = node.child_by_field_name("formals") {
            self.c.bound.extend(param_names(f, self.src));
        }
        if let Some(t) = node
            .child_by_field_name("universal")
            .and_then(|u| u.utf8_text(self.src).ok())
        {
            self.c.bound.insert(t.to_string());
        }
    }

    // bash: a command *is* a call, so `deploy main` uses the function `deploy`.
    // The name is a bare `word` — a kind make also uses for its targets — so
    // it is read here rather than through IDENT_KINDS. A builtin (`echo`,
    // `set`) resolves to no definition and costs nothing.
    fn visit_bash_command(&mut self, node: Node) -> bool {
        if self.spec.name != "bash" || node.kind() != "command_name" {
            return false;
        }
        if let Some(t) = trimmed(node, self.src) {
            self.c
                .push_use(node.start_position().row, t.to_string(), node.id());
        }
        true
    }

    // make: a prerequisite names another target, and `$(CC)` names a variable.
    // Both are bare `word` nodes — a kind too generic to put in IDENT_KINDS,
    // so they are read from the two parents that make one mean a reference.
    fn visit_make_words(&mut self, node: Node) -> bool {
        if !matches!(node.kind(), "prerequisites" | "variable_reference") {
            return false;
        }
        let (src, sr) = (self.src, node.start_position().row);
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            if ch.kind() != "word" {
                self.walk(ch);
            } else if let Some(t) = trimmed(ch, src) {
                self.c.push_use(sr, t.to_string(), ch.id());
            }
        }
        true
    }

    // svelte: `{@render row(1)}` puts the call in raw text rather than an
    // identifier node, so the name is the leading word of that text.
    fn visit_svelte_render(&mut self, node: Node) -> bool {
        if self.spec.name != "svelte" || node.kind() != "render_tag" {
            return false;
        }
        let mut cur = node.walk();
        let raw = node
            .named_children(&mut cur)
            .find(|c| c.kind() == "svelte_raw_text")
            .and_then(|raw| raw.utf8_text(self.src).ok());
        if let Some(t) = raw {
            let name: String = t
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            if !name.is_empty() {
                self.c.push_use(node.start_position().row, name, node.id());
            }
        }
        true
    }

    fn visit_css(&mut self, node: Node) -> bool {
        if self.spec.name != "css" {
            return false;
        }
        let (src, kind, sr) = (self.src, node.kind(), node.start_position().row);
        // a selector list is a *name*, never a set of references. Its
        // `class_name`/`id_name` wrap a plain `identifier`, which IDENT_KINDS
        // matches — so without this a stylesheet emits bare uses of `btn`,
        // `card`, `root` and `hover` into the union symbol table every other
        // file is ordered against, and starts drawing edges to python functions.
        if kind == "selectors" {
            return true;
        }
        // custom properties: `--brand: #0af` declares a name and `var(--brand)`
        // uses it — the one def→use pair a stylesheet has, and so the only
        // thing that lets a css hunk be ordered rather than merely described.
        // Both are spelled as ordinary declarations and values, told apart by
        // the `--` every custom property must start with.
        if kind == "declaration" {
            let mut cur = node.walk();
            let prop = node
                .named_children(&mut cur)
                .find(|c| c.kind() == "property_name")
                .and_then(|c| trimmed(c, src))
                .filter(|t| t.starts_with("--"));
            if let Some(name) = prop {
                let scope = self.scope();
                self.c.push_decl(sr, name, kind, scope);
            }
            // fall through: the declaration is still a member, and its value
            // can still hold a `var(--other)`
        }
        if kind == "plain_value" {
            if let Some(t) = trimmed(node, src).filter(|t| t.starts_with("--")) {
                self.c.push_use(sr, t.to_string(), node.id());
                return true;
            }
        }
        false
    }

    // yaml anchors: `&base` declares a name and `*base` uses it — the one real
    // def→use pair a config format has, and the only thing that lets a yaml
    // hunk be *ordered* rather than merely described. Both kinds are unique to
    // that grammar, so neither can shadow another language's identifiers.
    fn visit_yaml_anchor(&mut self, node: Node) -> bool {
        let kind = node.kind();
        if !matches!(kind, "anchor_name" | "alias_name") {
            return false;
        }
        let sr = node.start_position().row;
        if let Some(t) = trimmed(node, self.src) {
            if kind == "anchor_name" {
                self.c.push_decl(sr, t, kind, None);
            } else {
                self.c.push_use(sr, t.to_string(), node.id());
            }
        }
        true
    }

    fn visit_ident(&mut self, node: Node) -> bool {
        if !lang::is_ident(node.kind()) {
            return false;
        }
        let src = self.src;
        // the guard name in `#ifndef X` is not a use of anything either: the
        // only thing that ever defines X is the `#define` right below it,
        // which this same pair makes transparent
        let guard = node
            .next_named_sibling()
            .and_then(|d| include_guard_name(d, src));
        if guard.is_some_and(|g| node.utf8_text(src).is_ok_and(|t| t.trim() == g)) {
            return true;
        }
        // A zero-width identifier node is a parse artifact (C++ template and
        // macro constructs produce them); an empty name would surface in the
        // rationale as a stray comma. An all-digits one is a shell positional
        // parameter (`$1`, `$2`) — no language has a numeric symbol, so it can
        // never resolve to a definition and only clutters `uses`.
        if let Some(t) = trimmed(node, src).filter(|t| !t.chars().all(|c| c.is_ascii_digit())) {
            let (row, id) = (node.start_position().row, node.id());
            if lang::is_member_ident(node.kind()) {
                self.c.push_member_use(row, t.to_string(), id);
            } else {
                self.c.push_use(row, t.to_string(), id);
            }
        }
        false
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
        "var_spec" | "const_spec" | "variable_declarator" => "name",
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
    // c++ `const T& f()`: a `reference_declarator` holds its declarator as a
    // bare child, not a field, so the chain used to stop here and the
    // function fell back to being named after its return type
    if node.kind() == "reference_declarator" {
        return node.named_child(0).and_then(declarator_ident);
    }
    // a c++ `operator()` or `~Foo` declarator is the method's own name, though
    // neither spells it with an identifier node. Without this the declarator
    // resolved to nothing and the definition fell back to being named after
    // its return type — which, for `KOKKOS_FUNCTION void operator()(…)`, is
    // the macro in front of it.
    if matches!(node.kind(), "operator_name" | "destructor_name") {
        return Some(node);
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
    let named = ["name", "key"].iter().find_map(|f| {
        let t = node.child_by_field_name(f)?.utf8_text(src).ok()?;
        Some(tidy_ident(unquote(t.trim()))).filter(|n| !n.is_empty())
    });
    let declared = || declarator_name(node.child_by_field_name("declarator")?, src);
    // `property_name` is css's: a declaration names itself with one, and no
    // other grammar here produces that kind
    let first_ident = || {
        let mut cur = node.walk();
        let ch = node
            .named_children(&mut cur)
            .find(|ch| lang::is_ident(ch.kind()) || ch.kind() == "property_name")?;
        ch.utf8_text(src).ok().map(str::to_string)
    };
    named.or_else(declared).or_else(first_ident)
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
// documented at `symbol_identity_key` (src/bin/ordo/marks.rs).
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

/// One definition's arity, for the call-site check (P23.2). Only definitions
/// this can describe *exactly* are returned — see `signatures`.
pub struct SigInfo {
    pub name: String,
    /// parameters that must be passed
    pub required: usize,
    /// every parameter; a call passing more than this is passing too many
    pub total: usize,
}

/// One call site's arity, for the same check.
pub struct CallSite {
    /// 0-based row
    pub row: usize,
    pub name: String,
    pub argc: usize,
}

/// Every definition in `content` whose arity can be stated without guessing.
///
/// Deliberately narrow, because a false "wrong number of arguments" is worse
/// than a missed one. A definition is skipped when it is:
///
/// - not callable (`has_signature`) — a value has no arity;
/// - variadic — `*args` makes the upper bound meaningless;
/// - a method, detected by a leading `self` / `cls` / `this` parameter — the
///   receiver is passed implicitly at the call site and comparing the two
///   counts would report every method call as short by one.
pub fn signatures(spec: &LangSpec, content: &str) -> Vec<SigInfo> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    fn walk(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<SigInfo>) {
        if is_def_node(node, spec)
            && (lang::has_signature(node.kind()) || def_kind(node, spec).is_some())
        {
            if let (Some(name), Some(params)) = (node_name(node, src), params_of(node)) {
                let mut cur = params.walk();
                // python's bare `*` ends what a call can pass by position, and
                // `/` is punctuation, not a parameter
                let kinds: Vec<&str> = params
                    .named_children(&mut cur)
                    .map(|c| c.kind())
                    .take_while(|k| *k != "keyword_separator")
                    .filter(|k| *k != "positional_separator")
                    .collect();
                let variadic = kinds
                    .iter()
                    .any(|k| lang::param_kind(k) == lang::ParamKind::Variadic);
                let receiver = params
                    .named_child(0)
                    .and_then(|p| p.utf8_text(src).ok())
                    .is_some_and(|t| matches!(t.trim(), "self" | "cls" | "this"));
                if !variadic && !receiver {
                    let required = kinds
                        .iter()
                        .filter(|k| lang::param_kind(k) == lang::ParamKind::Required)
                        .count();
                    out.push(SigInfo {
                        name,
                        required,
                        total: kinds.len(),
                    });
                }
            }
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, out);
        }
    }
    walk(tree.root_node(), src, spec, &mut out);
    out
}

/// The parameter list of a definition, through the declarator chain c and c++
/// hang theirs off (the same walk `count_params` does).
fn params_of(node: Node) -> Option<Node> {
    let mut n = node;
    loop {
        if let Some(p) = n.child_by_field_name("parameters") {
            return Some(p);
        }
        n = n.child_by_field_name("declarator")?;
    }
}

/// Every call in `content` whose arity can be compared to a definition's.
///
/// A call is skipped when its callee is not a bare name (`obj.method(...)` may
/// be passing a receiver), or when any argument is passed by name — a keyword
/// argument says nothing about positional arity.
pub fn call_sites(spec: &LangSpec, content: &str) -> Vec<CallSite> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    fn walk(node: Node, src: &[u8], out: &mut Vec<CallSite>) {
        if let Some(args) = node.child_by_field_name("arguments") {
            let callee = node
                .child_by_field_name("function")
                .filter(|f| lang::is_ident(f.kind()))
                .and_then(|f| f.utf8_text(src).ok());
            let mut cur = args.walk();
            let kids: Vec<Node> = args.named_children(&mut cur).collect();
            let named = kids
                .iter()
                .any(|k| matches!(k.kind(), "keyword_argument" | "named_argument"));
            if let (Some(name), false) = (callee, named) {
                out.push(CallSite {
                    row: node.start_position().row,
                    name: name.trim().to_string(),
                    argc: kids.len(),
                });
            }
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, out);
        }
    }
    walk(tree.root_node(), src, &mut out);
    out
}

/// What a callable promises its callers beyond its arity, for the contract
/// checks that compare a definition's old side with its new one.
#[derive(Clone)]
pub struct DefFacts {
    pub name: String,
    /// 0-based rows it spans, decorators included
    pub rows: (usize, usize),
    /// the class it is a method of
    pub owner: Option<String>,
    pub is_async: bool,
    /// read as an attribute rather than called: python's `@property` /
    /// `@cached_property`, a js `get x()`
    pub getter: bool,
    /// in order, the receiver (`self`, `cls`, `this`) left out
    pub params: Vec<Param>,
    /// `@abstractmethod`: a subclass that does not define it cannot be made
    pub is_abstract: bool,
    /// exception types its own body raises, last dotted part, sorted
    pub raises: Vec<String>,
    /// what its own body's `with` statements enter, as written
    pub withs: Vec<String>,
    /// its own body's calls, as written, split by whether they are awaited
    pub awaited: Vec<String>,
    pub not_awaited: Vec<String>,
}

#[derive(Clone, PartialEq)]
pub struct Param {
    pub name: String,
    /// the default's source text
    pub default: Option<String>,
    pub slot: Slot,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Slot {
    /// passable by position (and, in python, by name)
    Positional,
    /// after python's `*` or `*args`: by name only
    KeywordOnly,
    /// `*args`, `...rest`
    Rest,
    /// `**kwargs`: takes any name
    AnyKeyword,
}

/// Every named callable in `content`, with the facts `DefFacts` keeps.
pub fn def_facts(spec: &LangSpec, content: &str) -> Vec<DefFacts> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    fn walk(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<DefFacts>) {
        if is_def_node(node, spec) && lang::has_signature(node.kind()) {
            if let Some(name) = node_name(node, src) {
                let mut cur = node.walk();
                let keywords: Vec<&str> = node.children(&mut cur).map(|c| c.kind()).collect();
                let holder = node
                    .parent()
                    .filter(|p| p.kind() == "decorated_definition")
                    .unwrap_or(node);
                // what `raises`, `withs` and the awaits are read from: only the
                // languages that spell them pay for the walk
                let body = if matches!(
                    spec.name,
                    "python" | "xonsh" | "javascript" | "typescript" | "tsx"
                ) {
                    own_body(
                        node,
                        &[
                            "with_item",
                            "await",
                            "await_expression",
                            "call",
                            "call_expression",
                            "raise_statement",
                        ],
                    )
                } else {
                    vec![]
                };
                out.push(DefFacts {
                    name,
                    rows: (holder.start_position().row, holder.end_position().row),
                    owner: owning_class(node, src),
                    is_async: keywords.contains(&"async"),
                    getter: keywords.contains(&"get")
                        || decorated(node, src, &["property", "cached_property"]),
                    is_abstract: decorated(node, src, &["abstractmethod"]),
                    raises: raises(&body, src),
                    withs: body
                        .iter()
                        .filter(|n| n.kind() == "with_item")
                        .filter_map(|w| w.named_child(0))
                        .map(|v| flat(v, src))
                        .collect(),
                    awaited: body
                        .iter()
                        .filter(|n| matches!(n.kind(), "await" | "await_expression"))
                        .filter_map(|a| a.named_child(0))
                        .filter(|c| c.child_by_field_name("arguments").is_some())
                        .map(|c| flat(c, src))
                        .collect(),
                    not_awaited: body
                        .iter()
                        .filter(|n| matches!(n.kind(), "call" | "call_expression"))
                        .filter(|c| {
                            c.parent()
                                .is_none_or(|p| !matches!(p.kind(), "await" | "await_expression"))
                        })
                        .map(|c| flat(*c, src))
                        .collect(),
                    params: params_of(node).map_or(vec![], |p| params(p, src)),
                });
            }
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, spec, out);
        }
    }
    walk(tree.root_node(), src, spec, &mut out);
    out
}

fn params(list: Node, src: &[u8]) -> Vec<Param> {
    let text = |n: Option<Node>| {
        n.and_then(|n| n.utf8_text(src).ok())
            .map(|t| t.trim().to_string())
    };
    let mut out = vec![];
    let mut keyword_only = false;
    let mut cur = list.walk();
    for (i, p) in list.named_children(&mut cur).enumerate() {
        let slot = match p.kind() {
            "keyword_separator" => {
                keyword_only = true;
                continue;
            }
            "positional_separator" | "comment" => continue,
            "dictionary_splat_pattern" => Slot::AnyKeyword,
            k if lang::param_kind(k) == lang::ParamKind::Variadic => {
                keyword_only = true;
                Slot::Rest
            }
            _ if keyword_only => Slot::KeywordOnly,
            _ => Slot::Positional,
        };
        let default = text(
            p.child_by_field_name("value")
                .or_else(|| p.child_by_field_name("right")),
        );
        // the name: a plain identifier, or the named part of a typed/defaulted one
        let name = if lang::is_ident(p.kind()) {
            text(Some(p))
        } else {
            text(
                p.child_by_field_name("name")
                    .or_else(|| p.child_by_field_name("left"))
                    .or_else(|| p.child_by_field_name("pattern"))
                    .or_else(|| {
                        let mut c = p.walk();
                        let first = p.named_children(&mut c).find(|c| lang::is_ident(c.kind()));
                        first
                    }),
            )
        };
        let Some(name) = name else {
            continue;
        };
        if i == 0 && matches!(name.as_str(), "self" | "cls" | "this") {
            continue;
        }
        out.push(Param {
            name,
            default,
            slot,
        });
    }
    out
}

fn owning_class(node: Node, src: &[u8]) -> Option<String> {
    let mut n = node.parent();
    while let Some(p) = n {
        if lang::has_signature(p.kind()) {
            return None;
        }
        if lang::is_type_kind(p.kind()) || p.kind() == "class" {
            return node_name(p, src);
        }
        n = p.parent();
    }
    None
}

/// the nodes of `kinds` in a def's own body, nested defs left out
fn own_body<'t>(def: Node<'t>, kinds: &[&str]) -> Vec<Node<'t>> {
    let mut out = vec![];
    let Some(body) = def.child_by_field_name("body") else {
        return out;
    };
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) {
            out.push(n);
        }
        if n.id() != body.id() && lang::has_signature(n.kind()) {
            continue;
        }
        let mut cur = n.walk();
        stack.extend(n.named_children(&mut cur));
    }
    out.sort_by_key(|n| n.start_byte());
    out
}

/// a node's text with its whitespace runs collapsed
fn flat(n: Node, src: &[u8]) -> String {
    n.utf8_text(src)
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// An enum's members and their values as written: `class Color(Enum)`'s
/// `RED = 1`, a ts `enum Color { Red = 1 }`.
pub fn enum_values(spec: &LangSpec, content: &str) -> Vec<(String, String, String, usize)> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    let python_enum = |c: &ClassFacts| {
        c.bases
            .iter()
            .any(|b| b.ends_with("Enum") || b.ends_with("Flag"))
    };
    let enums: Vec<String> = class_facts(spec, content)
        .into_iter()
        .filter(python_enum)
        .map(|c| c.name)
        .collect();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
        let (owner, name, value) = match node.kind() {
            // an `assignment` directly in an enum class body: `expression_statement` > `block` > class
            "assignment" => {
                let class = node
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|b| b.parent())
                    .filter(|c| c.kind() == "class_definition")
                    .and_then(|c| node_name(c, src));
                let Some(class) = class.filter(|c| enums.contains(c)) else {
                    continue;
                };
                (
                    class,
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                )
            }
            "enum_assignment" => {
                let Some(class) = node
                    .parent()
                    .and_then(|b| b.parent())
                    .and_then(|e| node_name(e, src))
                else {
                    continue;
                };
                (
                    class,
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                )
            }
            _ => continue,
        };
        if let (Some(n), Some(v)) = (name, value) {
            out.push((owner, flat(n, src), flat(v, src), node.start_position().row));
        }
    }
    out
}

/// python's `raise X` / `raise X(...)` among a def's own-body nodes
fn raises(body: &[Node], src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = body
        .iter()
        .filter(|n| n.kind() == "raise_statement")
        .filter_map(|n| n.named_child(0))
        .map(|x| {
            if x.kind() == "call" {
                x.child_by_field_name("function").unwrap_or(x)
            } else {
                x
            }
        })
        .filter_map(|t| t.utf8_text(src).ok())
        .filter_map(|t| t.trim().rsplit('.').next().map(str::to_string))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every call to `name` in `content` (as `calls_to` matches it) that sits in
/// the body of a `try`, with the exception types the enclosing handlers
/// name — `*` for a bare `except:`.
pub fn guarded_calls(
    spec: &LangSpec,
    content: &str,
    name: &str,
    receiver: bool,
) -> Vec<(usize, Vec<String>)> {
    let rows: Vec<usize> = calls_to(spec, content, name, receiver)
        .into_iter()
        .map(|c| c.row)
        .collect();
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
        if node.child_by_field_name("arguments").is_none()
            || !rows.contains(&node.start_position().row)
        {
            continue;
        }
        let mut caught = vec![];
        let (mut child, mut up) = (node, node.parent());
        while let Some(p) = up {
            if p.kind() == "try_statement"
                && p.child_by_field_name("body")
                    .is_some_and(|b| b.id() == child.id())
            {
                let mut cur = p.walk();
                for clause in p
                    .named_children(&mut cur)
                    .filter(|c| c.kind() == "except_clause")
                {
                    let mut cc = clause.walk();
                    let first = clause
                        .named_children(&mut cc)
                        .find(|c| c.kind() != "block" && c.kind() != "comment");
                    match first.and_then(|t| t.utf8_text(src).ok()) {
                        None => caught.push("*".to_string()),
                        Some(t) => caught.extend(
                            t.split(" as ")
                                .next()
                                .unwrap_or(t)
                                .trim_matches(|c| c == '(' || c == ')')
                                .split(',')
                                .filter_map(|x| x.trim().rsplit('.').next().map(str::to_string))
                                .filter(|x| !x.is_empty()),
                        ),
                    }
                }
            }
            if lang::has_signature(p.kind()) {
                break;
            }
            (child, up) = (p, p.parent());
        }
        if !caught.is_empty() {
            out.push((node.start_position().row, caught));
        }
    }
    out.sort();
    out.dedup_by_key(|(r, _)| *r);
    out
}

/// Is `node` decorated with one of `names`, however qualified (`abc.abstractmethod`)?
fn decorated(node: Node, src: &[u8], names: &[&str]) -> bool {
    decorators(node, src).iter().any(|d| {
        d.rsplit('.')
            .next()
            .is_some_and(|last| names.contains(&last))
    })
}

/// A class and the names of the classes it derives from, last dotted part
/// only: `class C(abc.ABC, Base)` gives `[ABC, Base]`.
pub struct ClassFacts {
    pub name: String,
    /// 0-based
    pub row: usize,
    pub bases: Vec<String>,
}

pub fn class_facts(spec: &LangSpec, content: &str) -> Vec<ClassFacts> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
        if !matches!(
            node.kind(),
            "class_definition" | "class_declaration" | "class"
        ) {
            continue;
        }
        let Some(name) = node_name(node, src) else {
            continue;
        };
        // python's `superclasses`, js's `class_heritage`
        let mut cur = node.walk();
        let list = node.child_by_field_name("superclasses").or_else(|| {
            let heritage = node
                .named_children(&mut cur)
                .find(|c| c.kind() == "class_heritage");
            heritage
        });
        let bases = list.map_or(vec![], |l| {
            let mut cur = l.walk();
            l.named_children(&mut cur)
                .filter(|b| b.kind() != "keyword_argument")
                .filter_map(|b| b.utf8_text(src).ok())
                .filter_map(|t| t.trim().rsplit('.').next().map(str::to_string))
                .collect()
        });
        out.push(ClassFacts {
            name,
            row: node.start_position().row,
            bases,
        });
    }
    out
}

/// python hangs decorators on a `decorated_definition` wrapper, js/ts on the
/// method itself
fn decorators(node: Node, src: &[u8]) -> Vec<String> {
    let holder = node
        .parent()
        .filter(|p| p.kind() == "decorated_definition")
        .unwrap_or(node);
    let mut cur = holder.walk();
    holder
        .named_children(&mut cur)
        .filter(|c| c.kind() == "decorator")
        .filter_map(|d| d.utf8_text(src).ok())
        .map(|t| {
            let t = t.trim().trim_start_matches('@');
            t.split('(').next().unwrap_or(t).trim().to_string()
        })
        .collect()
}

/// Every `obj.name` in `content` that is not assigned to, as (0-based row,
/// whether it is called). With `self_only`, only `self.name` / `this.name`.
pub fn member_reads(
    spec: &LangSpec,
    content: &str,
    name: &str,
    self_only: bool,
) -> Vec<(usize, bool)> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    let text = |n: Option<Node>| n.and_then(|n| n.utf8_text(src).ok()).map(str::trim);
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let member = node
            .child_by_field_name("attribute")
            .or_else(|| node.child_by_field_name("property"));
        let receiver = text(node.child_by_field_name("object"));
        if member.is_some()
            && text(member) == Some(name)
            && (!self_only || matches!(receiver, Some("self" | "this")))
        {
            let parent = node.parent();
            let assigned = parent.is_some_and(|p| {
                p.child_by_field_name("left")
                    .is_some_and(|l| l.id() == node.id())
            });
            if !assigned {
                let called = parent.is_some_and(|p| {
                    p.child_by_field_name("function")
                        .is_some_and(|f| f.id() == node.id())
                });
                out.push((node.start_position().row, called));
            }
        }
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
    }
    out.sort_unstable();
    out
}

/// One call's arguments, for checking it against a changed signature.
pub struct CallFacts {
    /// 0-based
    pub row: usize,
    /// its source text, whitespace-normalized, to tell a call left as it was
    /// from one the change rewrote
    pub text: String,
    pub positional: usize,
    pub keywords: Vec<String>,
    /// `*xs` / `**kw` / `...xs`: how many it passes cannot be counted
    pub spread: bool,
    /// its result is thrown away or only tested for truth — the two uses
    /// where getting a coroutine or a promise instead of a value goes unnoticed
    pub discarded: bool,
}

/// Every call to `name` in `content`: a bare `name(...)`, or with `receiver`
/// a `self.name(...)` / `this.name(...)` — any other `obj.name()` may be a
/// different `name`.
pub fn calls_to(spec: &LangSpec, content: &str, name: &str, receiver: bool) -> Vec<CallFacts> {
    all_calls(spec, content)
        .into_iter()
        .filter(|(n, r, _)| n == name && *r == receiver)
        .map(|(.., c)| c)
        .collect()
}

/// Every call in `content` a contract check can match to a definition, as
/// (callee name, whether through `self.`/`this.`, the call), in row order.
pub fn all_calls(spec: &LangSpec, content: &str) -> Vec<(String, bool, CallFacts)> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    let text = |n: Node| n.utf8_text(src).map(str::trim).unwrap_or("");
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let mut cur = node.walk();
        stack.extend(node.named_children(&mut cur));
        let (Some(f), Some(args)) = (
            node.child_by_field_name("function"),
            node.child_by_field_name("arguments"),
        ) else {
            continue;
        };
        let member = f
            .child_by_field_name("attribute")
            .or_else(|| f.child_by_field_name("property"));
        let callee = match member {
            Some(m)
                if f.child_by_field_name("object")
                    .is_some_and(|o| matches!(text(o), "self" | "this")) =>
            {
                (text(m), true)
            }
            None if lang::is_ident(f.kind()) => (text(f), false),
            _ => continue,
        };
        let mut cur = args.walk();
        let (mut positional, mut keywords, mut spread) = (0, vec![], false);
        for a in args.named_children(&mut cur) {
            match a.kind() {
                "keyword_argument" => {
                    keywords.extend(a.child_by_field_name("name").map(|n| text(n).to_string()))
                }
                "list_splat" | "dictionary_splat" | "spread_element" => spread = true,
                "comment" => {}
                _ => positional += 1,
            }
        }
        out.push((
            callee.0.to_string(),
            callee.1,
            CallFacts {
                row: node.start_position().row,
                text: text(node).split_whitespace().collect::<Vec<_>>().join(" "),
                positional,
                keywords,
                spread,
                discarded: result_unused_or_tested(node),
            },
        ));
    }
    out.sort_by_key(|(.., c)| c.row);
    out
}

fn result_unused_or_tested(call: Node) -> bool {
    let mut n = call;
    let mut parent = call.parent();
    while let Some(p) = parent.filter(|p| p.kind() == "parenthesized_expression") {
        n = p;
        parent = p.parent();
    }
    let Some(p) = parent else {
        return false;
    };
    let logical = p.kind() == "binary_expression"
        && p.child_by_field_name("operator")
            .is_some_and(|o| matches!(o.kind(), "&&" | "||"));
    p.kind() == "expression_statement"
        || logical
        || matches!(p.kind(), "not_operator" | "boolean_operator")
        || (p.kind() == "unary_expression" && p.child(0).is_some_and(|op| op.kind() == "!"))
        || p.child_by_field_name("condition")
            .is_some_and(|c| c.id() == n.id())
}

/// Every 0-based row where `name` appears as an *identifier* in `content` —
/// not in a string, not in a comment, because those are not references.
/// Used by the incomplete-rename check (P23.2) to find a name that should have
/// stopped existing.
pub fn identifier_rows(spec: &LangSpec, content: &str, name: &str) -> Vec<usize> {
    let mut out = vec![];
    let Some(tree) = lang::parse(spec, content) else {
        return out;
    };
    let src = content.as_bytes();
    fn walk(node: Node, src: &[u8], name: &str, out: &mut Vec<usize>) {
        if lang::is_ident(node.kind()) && node.utf8_text(src).map(str::trim) == Ok(name) {
            out.push(node.start_position().row);
        }
        let mut cur = node.walk();
        for ch in node.named_children(&mut cur) {
            walk(ch, src, name, out);
        }
    }
    walk(tree.root_node(), src, name, &mut out);
    out.dedup();
    out
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

/// The naming paths, tried in order. Three answers are final, a `None`
/// included: a markdown section's heading, a `name` field, and a grammar's
/// own way of naming a construct (an element without an id is anonymous).
fn node_name_inner(node: Node, src: &[u8]) -> Option<String> {
    if let Some(h) = section_heading(node) {
        return heading_name(h, src);
    }
    // own name (function foo, class Foo, local function foo, impl Foo, …).
    // Unquoted: a few grammars name a construct with a string literal rather
    // than an identifier — `{{ define "mychart.labels" }}` — and the quotes
    // are the grammar's, not part of the name.
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(src).ok().map(|t| unquote(t.trim()).to_string());
    }
    if let Some(name) =
        declarator_field_name(node, src).or_else(|| definition_field_name(node, src))
    {
        return Some(name);
    }
    if let Some(own) = grammar_name(node, src) {
        return own;
    }
    config_key_name(node, src)
        .or_else(|| bound_name(node, src))
        .or_else(|| type_field_name(node, src))
        .or_else(|| first_ident_child(node, src))
}

/// A markdown `section`'s heading. No name/declarator/identifier field exists
/// there (a heading is prose, not an identifier), so the section is named
/// from the heading text. Scoped to this exact kind, which no other grammar
/// in this crate produces (verified against each grammar's node-types.json),
/// so it can't shadow any other language's naming path.
///
/// A section's first child is only sometimes a heading: content before the
/// document's first heading is its own headless section (e.g. an HTML comment
/// or a stray paragraph at the top of a file). Naming it after that raw
/// content reads badly, so it stays anonymous rather than borrowing the wrong
/// node's text. ini spells `[user]` as a `section` too — same kind name, a
/// different grammar, told apart by the child that carries the name. It falls
/// through to the config-key path; a *markdown* section with no heading finds
/// nothing there either (that grammar has no `*_name` child and no identifier
/// kind) and stays anonymous.
fn section_heading(node: Node) -> Option<Node> {
    if node.kind() != "section" {
        return None;
    }
    node.named_child(0)
        .filter(|h| matches!(h.kind(), "atx_heading" | "setext_heading"))
}

/// A name nested one or more levels down a `declarator` field — java
/// `field_declaration` -> `variable_declarator`, c/cpp `declaration` ->
/// `pointer_declarator`/`init_declarator`/… -> identifier.
fn declarator_field_name(node: Node, src: &[u8]) -> Option<String> {
    declarator_name(node.child_by_field_name("declarator")?, src)
}

/// python `decorated_definition` -> the class/function it wraps, under field
/// `definition`.
fn definition_field_name(node: Node, src: &[u8]) -> Option<String> {
    node_name(node.child_by_field_name("definition")?, src)
}

/// The `type` field, for a def named after a type rather than an identifier
/// of its own — rust `impl<'s> Worker<'s>`, whose first named child is the
/// lifetime list, and `impl Display for Work`, where the first identifier is
/// the *trait*. Only when there is no `declarator`, so c/cpp's `type` (a
/// return type) and java's (a field type) can never be reached: those shapes
/// are named by `declarator_field_name`.
fn type_field_name(node: Node, src: &[u8]) -> Option<String> {
    if node.child_by_field_name("declarator").is_some() {
        return None;
    }
    type_name(node.child_by_field_name("type")?, src)
}

/// The first identifier-ish child (e.g. rust impl's type_identifier). js/ts
/// `arrow_function` is the one def kind whose own single bare parameter
/// (`x => …`, field `parameter`) is itself a direct identifier child — without
/// this guard it would be picked up here and misname an anonymous callback
/// after its own parameter instead of staying nameless.
fn first_ident_child(node: Node, src: &[u8]) -> Option<String> {
    if node.kind() == "arrow_function" {
        return None;
    }
    let mut cur = node.walk();
    let ch = node
        .named_children(&mut cur)
        .find(|ch| lang::is_ident(ch.kind()))?;
    ch.utf8_text(src).ok().map(|s| s.to_string())
}

/// The name for a node kind that only one grammar produces (each verified
/// against that grammar's node-types.json, so none can shadow another
/// language's naming path). `None` when the kind is not one of these;
/// `Some(None)` when it is and names nothing.
fn grammar_name(node: Node, src: &[u8]) -> Option<Option<String>> {
    let child = |kind: &str| child_where(node, |c| c.kind() == kind);
    let text = |n: Node| n.utf8_text(src).ok().map(str::trim);
    Some(match node.kind() {
        // svelte. `{#snippet row(x)}` declares a reusable named block that
        // `{@render row(1)}` calls — a real definition, not a region, and the
        // one def→use pair a component's markup has.
        "snippet_statement" => {
            let name =
                child("snippet_start").and_then(|s| child_where(s, |c| c.kind() == "snippet_name"));
            name.and_then(text)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        }
        "element" => html_element_id(node, src),
        // css. A rule set is named by its whole selector list — `.btn` and
        // `#nav a:hover` as written, sigils kept, because the sigil is what
        // makes a css symbol unable to collide with a code one. Runs of
        // whitespace collapse to one space (a selector list is punctuation,
        // not an identifier, so `node_name` leaves it out of `tidy_ident`).
        "rule_set" => child("selectors")
            .and_then(text)
            .map(squeeze)
            .filter(|t| !t.is_empty()),
        "keyframes_statement" => child("keyframes_name")
            .and_then(text)
            .filter(|t| !t.is_empty())
            .map(|t| format!("@keyframes {t}")),
        // make. A rule is named by its first target. A *special* target
        // (`.PHONY`, `.SUFFIXES`) names no recipe anyone navigates to, so it
        // stays anonymous — its prerequisites are still read as uses of the
        // real targets it lists, which is exactly what a `.PHONY` line is.
        // `targets` is a node kind here, not a field (unlike `normal:` for
        // prerequisites) — verified against tree-sitter-make-1.1.1
        "rule" => child("targets")
            .and_then(|t| t.named_child(0))
            .and_then(text)
            .filter(|t| !t.is_empty() && !t.starts_with('.'))
            .map(str::to_string),
        // cmake. Every cmake construct is a command whose name is its first
        // argument: `function(my_helper …)`, `set(SOURCES …)`. A command that
        // introduces nothing resolves to no name and stays transparent, which
        // is why `normal_command` can sit in `defs` without every `message()`
        // call becoming a definition.
        "function_def" | "macro_def" => cmake_first_arg(node, src),
        "normal_command" => cmake_command(node, src)
            .filter(|cmd| matches!(cmd.as_str(), "set" | "option"))
            .and_then(|_| cmake_first_arg(node, src)),
        // jinja. `{% block server %}` / `{% macro row(a) %}`: the name lives
        // in the opening statement, and for a macro one level further down
        // inside a `function_call` (jinja spells a parameter list the same
        // way it spells a call). The deep search for the first identifier
        // can't reach into another language's shapes.
        "block_block" | "macro_block" => {
            node.named_child(0).and_then(|st| first_ident_text(st, src))
        }
        _ => return None,
    })
}

/// html. An element is named by its `id`, and only by that: an id is the one
/// handle a stylesheet, a script or a fragment link addresses it by. Without
/// one the element resolves to no name and is transparent, so a page of
/// anonymous `<div>`s contributes no definitions at all.
fn html_element_id(node: Node, src: &[u8]) -> Option<String> {
    let tag = child_where(node, |c| {
        matches!(c.kind(), "start_tag" | "self_closing_tag")
    })?;
    let attr = child_where(tag, |attr| {
        attr.kind() == "attribute"
            && attr
                .named_child(0)
                .filter(|n| n.kind() == "attribute_name")
                .and_then(|n| n.utf8_text(src).ok())
                .is_some_and(|t| t.eq_ignore_ascii_case("id"))
    })?;
    // the first `id` decides: one without a value leaves the element anonymous
    let text = unquote(attr.named_child(1)?.utf8_text(src).ok()?.trim());
    (!text.is_empty()).then(|| format!("#{text}"))
}

// Strip one matched pair of surrounding quotes. Matched, not `trim_matches`:
// a gitconfig subsection is `remote "origin"`, whose quotes are part of the
// name and whose leading character is not one — trimming from both ends
// independently would leave `remote "origin`.
fn unquote(s: &str) -> &str {
    let mut ch = s.chars();
    match (ch.next(), ch.next_back()) {
        (Some(a), Some(b)) if a == b && (a == '"' || a == '\'') => {
            &s[a.len_utf8()..s.len() - a.len_utf8()]
        }
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
        let found = node.named_children(&mut cur).find(|c| {
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
        //
        // A c++ namespace body and class body are spelled `declaration_list`
        // and `field_declaration_list`: scopes, but the `binds` test below
        // matches them on the word "declaration" and then takes the first
        // identifier of the *previous* sibling — which named every templated
        // function after its neighbour's leading token, commonly `nodiscard`.
        // (A list is not always a scope: lua reaches a real binding through an
        // `expression_list`, so this names the scope kinds rather than
        // rejecting every `*_list`.)
        if matches!(
            k,
            "arguments"
                | "statement_block"
                | "class_body"
                | "program"
                | "block"
                | "declaration_list"
                | "field_declaration_list"
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
            if let Some(name) = binding_target(parent, node, src) {
                return Some(name);
            }
        }
        child = parent;
    }
    None
}

/// What a binding node names: its naming field, else the first identifier
/// appearing before `def`'s subtree.
fn binding_target(parent: Node, def: Node, src: &[u8]) -> Option<String> {
    let field = ["name", "left", "variable", "key", "property"]
        .iter()
        .find_map(|f| parent.child_by_field_name(f)?.utf8_text(src).ok())
        .map(|t| t.trim().to_string());
    field.or_else(|| {
        let mut c = parent.walk();
        let found = parent
            .named_children(&mut c)
            .take_while(|ch| ch.byte_range().start < def.byte_range().start)
            .find_map(|ch| first_ident_text(ch, src));
        found
    })
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
    let collapsed = squeeze(stripped);
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
    let mut out = HashSet::new();
    collect_local_binds(tree.root_node(), content.as_bytes(), spec, &mut out);
    out
}

/// Names bound by a local declaration, and nothing else.
///
/// This used to run the general `walk`, which fills seventeen `Collected`
/// fields, to read one of them — measured at a third of `FileSymbols::of`,
/// itself roughly 39% of a run over a large changeset. The rule is
/// self-contained (`is_local`, minus anything under a failed parse), so it
/// gets its own descent.
fn collect_local_binds(node: Node, src: &[u8], spec: &LangSpec, out: &mut HashSet<String>) {
    out.extend(bound_names(node, src, spec).map(|(_, name)| name));
    // a bound value can still hold further declarations
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        collect_local_binds(child, src, spec, out);
    }
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
    walk_file(spec, content).map_or(vec![], |c| c.member_rows)
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
            let row = child.start_position().row + 1;
            out.extend(bound_names(child, src, spec).map(|(_, name)| (name, row)));
            continue;
        }
        collect_top_binds(child, src, spec, out);
    }
}

/// The identifiers a `locals`-kind node binds, with their names — nothing
/// for any other node. A declaration the parser could not make sense of
/// yields nonsense names — a macro-heavy c++ header (VTK's vtkTypeMacro
/// family, say) parses with ERROR nodes and hands back `virtual`/`override`
/// as if they were bound — so a node with an error yields nothing either:
/// nothing harvested from a failed parse is trustworthy. A macro-shaped
/// declaration with no real name is skipped the same way.
fn bound_names<'t>(
    node: Node<'t>,
    src: &'t [u8],
    spec: &LangSpec,
) -> impl Iterator<Item = (Node<'t>, String)> {
    let kind = node.kind();
    let ids = if spec.is_local(kind) && !node.has_error() {
        binding_idents(node, kind)
    } else {
        vec![]
    };
    ids.into_iter().filter_map(move |id| {
        let name = id.utf8_text(src).ok().map(tidy_ident)?;
        (!name.is_empty()).then_some((id, name))
    })
}

/// Declared names with their 1-based rows: `(definitions, imports)`.
pub type SymbolRows = (Vec<(String, usize)>, Vec<(String, usize)>);

/// Each def's `(name, normalized signature/header, normalized whole body,
/// substantial body lines)`. The header (node text before the `body` field)
/// drives signature-change vs body-only-edit wording (#4). The whole body drives
/// exact rename/move matching (#7 / P11.2); the line set drives line-overlap
/// relocation detection (P16). Body = the def's `body` field, else the node text.
pub type Body = (String, String, String, Vec<String>);

/// What one descent of a file's tree collects: definition rows, import rows and
/// every definition's header and body.
#[derive(Default)]
struct DefOut {
    defs: Vec<(String, usize)>,
    imports: Vec<(String, usize)>,
    bodies: Vec<Body>,
}

/// One descent doing the work `collect_rows` and `collect_bodies` used to do in
/// two, over the same tree, for both sides of every file.
///
/// The two had different stopping rules, and this keeps both exactly: rows stop
/// at an import (an import statement's insides are not definitions, and its
/// names are already recorded), while bodies carry on descending, which is what
/// `collect_bodies` did when it ran separately. `in_import` is what tells the
/// row half it is under one; it is not a shortcut for "skip this subtree".
fn collect_defs(node: Node, src: &[u8], spec: &LangSpec, o: &mut DefOut, in_import: bool) {
    let row = node.start_position().row + 1; // 1-based
    let import = !in_import && import_like(node, src, spec);
    if import {
        let names = import_bound_names(node, src, spec)
            .map(|names| names.into_iter().map(|(_, n)| n).collect())
            .unwrap_or_else(|| ident_texts(node, src));
        o.imports.extend(names.into_iter().map(|n| (n, row)));
    }
    let def = is_def_node(node, spec)
        .then(|| node_name(node, src))
        .flatten();
    if let Some(name) = def {
        // `collect_rows` returned before its own def check when the node was an
        // import, so a definition under one was never a row
        if !in_import && !import {
            o.defs.push((name.clone(), row));
        }
        let (header, whole, lines) = body_parts(node, src, spec);
        o.bodies.push((name, header, whole, lines));
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_defs(ch, src, spec, o, in_import || import);
    }
}

/// Definition and import rows, and every definition's header and body, from one
/// descent — what `FileSymbols` needs per side.
pub fn symbol_facts(spec: &LangSpec, content: &str) -> (SymbolRows, Vec<Body>) {
    let mut o = DefOut::default();
    if let Some(tree) = lang::parse(spec, content) {
        collect_defs(tree.root_node(), content.as_bytes(), spec, &mut o, false);
    }
    ((o.defs, o.imports), o.bodies)
}

/// What an import binds, and where that name comes from: the name this file
/// now has, the symbol the *defining* file calls it, and the module it was
/// taken from. `from lib import helper as h` is `("h", "helper", Some("lib"))`.
///
/// `import_bound_names` answers only the first of those, which is all the
/// hunk's own `imports` list needs. An edge needs the other two: a definition
/// is matched by the name its own file gives it, and the module is what says
/// which file that is (see `order::Binding`).
pub struct ImportBinding {
    /// the name this file now has
    pub bound: String,
    /// what the defining file calls it — the only name an edge can match on
    pub origin: String,
    /// the module it was taken from, when the statement names one
    pub module: Option<String>,
}

/// Every symbol-level import binding in a file, for the languages that spell
/// one. `import os` binds a module rather than a symbol and is not one of
/// these: there is no definition in the change it could be matched against.
pub fn import_bindings(spec: &LangSpec, content: &str) -> Vec<ImportBinding> {
    let mut out = vec![];
    if let Some(tree) = lang::parse(spec, content) {
        collect_import_bindings(tree.root_node(), content.as_bytes(), spec, &mut out);
    }
    out
}

fn collect_import_bindings(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<ImportBinding>) {
    if import_origins(node, src, spec, out) {
        return; // an import's insides are not another import
    }
    let mut cur = node.walk();
    for ch in node.named_children(&mut cur) {
        collect_import_bindings(ch, src, spec, out);
    }
}

/// The bindings one import statement introduces, or `false` when this node is
/// not an import this understands. Node kinds and field names verified against
/// tree-sitter-{python-0.23.6,javascript-0.23.1,typescript-0.23.2,rust-0.23}'s
/// node-types.json; the `alias`/`name` field pairing is a shared convention
/// across all four.
fn import_origins(node: Node, src: &[u8], spec: &LangSpec, out: &mut Vec<ImportBinding>) -> bool {
    match (spec.name, node.kind()) {
        ("python" | "xonsh", "import_from_statement") => python_import_origins(node, src, out),
        ("javascript" | "typescript" | "tsx", "import_statement") => {
            js_import_origins(node, src, out)
        }
        ("rust", "use_as_clause") => rust_import_origin(node, src, out),
        _ => false,
    }
}

fn node_text(node: Node, src: &[u8]) -> Option<String> {
    node.utf8_text(src).ok().map(str::to_string)
}

/// the last segment of a dotted name, else the name itself
fn last_segment(node: Node, src: &[u8]) -> Option<String> {
    let mut c = node.walk();
    let parts: Vec<Node> = node.named_children(&mut c).collect();
    parts
        .last()
        .and_then(|n| node_text(*n, src))
        .or_else(|| node_text(node, src))
}

// `from a.b import c, d as e` binds c from a.b and e (really d) from a.b
fn python_import_origins(node: Node, src: &[u8], out: &mut Vec<ImportBinding>) -> bool {
    // `from . import helper` names the package, not a module: there is
    // no file name in it to match a definer against, so it constrains
    // nothing and the name match stands on its own
    let module = node
        .child_by_field_name("module_name")
        // a bare `from . import x` is an `import_prefix` and nothing
        // else; a `from .pkg import x` carries the dotted name beside
        // it, which is the part that can name a file
        .filter(|m| m.kind() != "relative_import" || m.named_child_count() > 1)
        .and_then(|m| last_segment(m, src));
    let mut cur = node.walk();
    for n in node.children_by_field_name("name", &mut cur) {
        let (bound, origin) = match n.kind() {
            "aliased_import" => (
                n.child_by_field_name("alias")
                    .and_then(|a| node_text(a, src)),
                n.child_by_field_name("name")
                    .and_then(|a| last_segment(a, src)),
            ),
            _ => (last_segment(n, src), last_segment(n, src)),
        };
        if let (Some(bound), Some(origin)) = (bound, origin) {
            out.push(ImportBinding {
                bound,
                origin,
                module: module.clone(),
            });
        }
    }
    true
}

// `import { a, b as c } from './x'`
fn js_import_origins(node: Node, src: &[u8], out: &mut Vec<ImportBinding>) -> bool {
    let module = node
        .child_by_field_name("source")
        .and_then(|s| node_text(s, src))
        .map(|t| unquote(t.trim()).to_string());
    let mut found = false;
    for n in import_specifiers(node) {
        let origin = n
            .child_by_field_name("name")
            .and_then(|a| node_text(a, src));
        let bound = n
            .child_by_field_name("alias")
            .and_then(|a| node_text(a, src))
            .or_else(|| origin.clone());
        if let (Some(bound), Some(origin)) = (bound, origin) {
            out.push(ImportBinding {
                bound,
                origin,
                module: module.clone(),
            });
            found = true;
        }
    }
    found
}

// `use a::b as c` — the module is the path without its last segment
fn rust_import_origin(node: Node, src: &[u8], out: &mut Vec<ImportBinding>) -> bool {
    let path = node.child_by_field_name("path");
    let origin = path.and_then(|p| last_segment(p, src));
    let bound = node
        .child_by_field_name("alias")
        .and_then(|a| node_text(a, src));
    let module = path.and_then(|p| node_text(p, src)).and_then(|p| {
        p.rsplit_once("::")
            .map(|(head, _)| head.trim().to_string())
            .filter(|m| !m.is_empty())
    });
    if let (Some(bound), Some(origin)) = (bound, origin) {
        out.push(ImportBinding {
            bound,
            origin,
            module,
        });
    }
    true
}

/// every `import_specifier` under a js/ts import, however it is nested
fn import_specifiers(node: Node) -> Vec<Node> {
    let mut out = vec![];
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "import_specifier" {
            out.push(n);
            continue;
        }
        let mut cur = n.walk();
        stack.extend(n.named_children(&mut cur));
    }
    out
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
            out.insert(squeeze(t));
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
    // an include or a go import names a *path*, not an identifier, so the
    // identifier fallback finds nothing and the hunk binds no name at all —
    // `imports = "boost/**"` could never match. Bind the path text instead:
    // the include as written, a go package by the name code refers to it by.
    match (spec.name, node.kind()) {
        ("c" | "cpp", "preproc_include") => {
            let path = node.child_by_field_name("path")?.utf8_text(src).ok()?;
            let path = path.trim().trim_matches(|c| matches!(c, '<' | '>' | '"'));
            Some(vec![(row, path.to_string())])
        }
        // bash: the script `source ./lib/common.sh` pulls in
        ("bash", "command") => {
            let arg = node.child_by_field_name("argument")?;
            let text = unquote(arg.utf8_text(src).ok()?.trim());
            (!text.is_empty()).then(|| vec![(row, text.to_string())])
        }
        // nix: the path `import ./overlays.nix` pulls in
        ("nix", "apply_expression") => {
            let arg = node.child_by_field_name("argument")?;
            let text = arg.utf8_text(src).ok()?.trim();
            (!text.is_empty()).then(|| vec![(row, text.to_string())])
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
            (!out.is_empty()).then_some(out)
        }
        // cmake: the module, package or subdirectory the command names
        ("cmake", _) => Some(vec![(row, cmake_first_arg(node, src)?)]),
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
            Some(vec![(row, lit)])
        }
        ("go", "import_spec") => Some(go_import_name(node, src).into_iter().collect()),
        ("go", "import_declaration") => Some(go_import_names(node, src)),
        ("python" | "xonsh", "import_statement" | "import_from_statement") => {
            Some(python_import_names(node, src))
        }
        _ => None,
    }
}

/// every package a go `import (...)` block or single import names
fn go_import_names(node: Node, src: &[u8]) -> Vec<(usize, String)> {
    let mut cur = node.walk();
    let mut out = vec![];
    for spec_node in node.named_children(&mut cur) {
        match spec_node.kind() {
            "import_spec" => out.extend(go_import_name(spec_node, src)),
            "import_spec_list" => {
                let mut c2 = spec_node.walk();
                let specs = spec_node
                    .named_children(&mut c2)
                    .filter(|n| n.kind() == "import_spec");
                out.extend(specs.filter_map(|sp| go_import_name(sp, src)));
            }
            _ => {}
        }
    }
    out
}

/// python: `import a.b` binds `a`; `from a.b import c, d as e` binds c, e
fn python_import_names(node: Node, src: &[u8]) -> Vec<(usize, String)> {
    let row = node.start_position().row;
    let text = |n: Node| n.utf8_text(src).ok().map(|t| (row, t.to_string()));
    let from = node.kind() == "import_from_statement";
    let mut cur = node.walk();
    node.children_by_field_name("name", &mut cur)
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
        .collect()
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
    walk_file(spec, content).map_or_else(HashSet::new, |c| c.field_inits)
}

/// `Class.member` when the walker knows the class, else the bare name — which
/// is the old behaviour, and right for a member with no enclosing type.
pub fn owned_member(stack: &[String], name: &str) -> String {
    match stack.last() {
        Some(owner) => format!("{owner}.{name}"),
        None => name.to_string(),
    }
}

/// One string with runs of whitespace collapsed to single spaces and the ends
/// trimmed — the normalisation every body, header and line comparison in this
/// module runs before matching.
///
/// `split_whitespace().collect::<Vec<_>>().join(" ")` says the same thing, and
/// allocates a `Vec<&str>` to do it. `collect_bodies` runs this once per line
/// of every definition in every file on both sides, which measured as the
/// single largest cost in `FileSymbols::of`.
pub(crate) fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
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

#[cfg(test)]
mod tests {
    use super::{squeeze, symbol_facts};

    /// `symbol_facts` is one descent doing what two used to. The halves have
    /// different stopping rules — rows stop at an import, bodies do not — so
    /// the risk of merging them is that one half quietly stops contributing.
    #[test]
    fn one_descent_still_returns_both_halves() {
        let spec = crate::lang::for_path("a.py").expect("python");
        let src = "import os\nfrom sys import argv\n\ndef parse(p):\n    return os.path.join(p)\n\ndef main():\n    return parse(argv[1])\n";
        let ((defs, imports), bodies) = symbol_facts(spec, src);

        let names: Vec<&str> = defs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"parse") && names.contains(&"main"),
            "{names:?}"
        );
        assert!(
            imports.iter().any(|(n, _)| n == "os"),
            "imports still collected: {imports:?}"
        );
        // and the bodies half of the same descent
        let bodied: Vec<&str> = bodies.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(
            bodied.contains(&"parse") && bodied.contains(&"main"),
            "{bodied:?}"
        );
        let parse = bodies.iter().find(|(n, _, _, _)| n == "parse").unwrap();
        assert!(parse.1.contains("def parse"), "header: {:?}", parse.1);
        assert!(parse.2.contains("os.path.join"), "body: {:?}", parse.2);
    }

    /// An import statement's insides are not definitions. `collect_rows` used
    /// to return at an import so nothing under one became a row; the merged
    /// descent has to carry on for the bodies half without losing that.
    #[test]
    fn nothing_under_an_import_becomes_a_definition_row() {
        let spec = crate::lang::for_path("a.py").expect("python");
        let ((defs, imports), _) = symbol_facts(spec, "from a import (b, c)\n");
        assert!(
            defs.is_empty(),
            "an import declares no definitions: {defs:?}"
        );
        assert!(!imports.is_empty(), "but it does declare imports");
    }

    /// `squeeze` replaced `split_whitespace().collect::<Vec<_>>().join(" ")` at
    /// fourteen sites; it has to mean exactly that, including at the edges.
    #[test]
    fn squeeze_matches_the_idiom_it_replaced() {
        let idiom = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        for case in [
            "",
            "   ",
            "one",
            "  leading and trailing  ",
            "runs   of\tmixed \n whitespace",
            "\n\ttabs\tand\nnewlines\n",
            "def f(a,   b):  return   a",
            "héllo   wörld",
        ] {
            assert_eq!(squeeze(case), idiom(case), "{case:?}");
        }
    }
}
