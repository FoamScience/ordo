//! ordo — comprehension-optimized ordering of code-change hunks.
//! Public entry: [`run`] takes an [`Input`] and returns the v1 [`Output`].
mod advisories;
pub mod catalog;
mod extract;
mod lang;
pub mod model;
mod order;
mod patch;
pub mod refine;
pub mod rules;

use extract::{analyze, compute_hunks, HunkSem, RawHunk};
use lang::LangSpec;
use model::*;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::ops::RangeInclusive;
use tree_sitter::Node;

pub use lang::is_generated_path;
pub use lang::is_test_path;

/// Every registered language as (name, member node kinds) — the language
/// registry as the generated docs read it, so `docs/languages.md` and the
/// detail-layer table cannot name a language the engine does not support.
pub fn languages() -> Vec<(&'static str, &'static [&'static str])> {
    lang::all().iter().map(|s| (s.name, s.members)).collect()
}

/// The language name `src/lang.rs` knows a path by (`python`, `cpp`, `yaml`, …)
/// — the same string a rule's `lang` condition is written against. `None` when
/// the path has no grammar.
pub fn lang_name_for_path(path: &str) -> Option<&'static str> {
    lang::for_path(path).map(|s| s.name)
}
pub use patch::split_patch;

pub const SCHEMA_VERSION: u32 = 2;

/// Which catalog rules this run reports: none when the caller turned the
/// catalog off, otherwise everything their `disable` globs do not name.
fn catalog_for<'a>(options: &Options, problems: &mut Vec<String>) -> Cow<'a, [Rule]> {
    if !options.catalog {
        return Cow::Borrowed(&[]);
    }
    if options.disable.is_empty() {
        // the common path: borrow the compiled catalog rather than deep-copying
        // every rule's query and message on every run
        return Cow::Borrowed(catalog::rules());
    }
    let mut set = globset::GlobSetBuilder::new();
    for d in &options.disable {
        match globset::Glob::new(d) {
            Ok(g) => {
                set.add(g);
            }
            // a `disable` nobody can parse silences nothing, and saying so is
            // the difference between "that rule is off" and "you typed it wrong"
            Err(e) => problems.push(format!("disable `{d}` is not a glob: {e}")),
        }
    }
    let set = match set.build() {
        Ok(s) => s,
        Err(e) => {
            problems.push(format!("disable: {e}"));
            return Cow::Borrowed(catalog::rules());
        }
    };
    Cow::Owned(
        catalog::rules()
            .iter()
            .filter(|r| !set.is_match(&r.name))
            .cloned()
            .collect(),
    )
}

pub fn run(input: Input) -> Output {
    // A caller who sends a diff gets the same review as one who sends
    // `old`/`new`, whenever the diff carries enough to rebuild them.
    let mut input = input;
    let mut input_problems = fill_sides(&mut input);
    // Templates are rewritten before anything else looks at them: every later
    // parse (semantics, symbol rows, bodies, advisories) then sees text the
    // underlying grammar can read, at unchanged offsets. What the jinja
    // statements *said* is harvested first, since blanking is what makes the
    // rest work — see `extract::mask_template`.
    let (input, templates) = mask_templates(input);
    let mut hunks = build_hunks(&input, &templates);

    let paths: Vec<String> = input.changes.iter().map(|c| c.path.clone()).collect();
    // per-file old/new symbol data (rows for positions, sets for membership,
    // bodies for rename/move matching) — drives #3/#5/#7 and P12.1 moves.
    let symbols: Vec<FileSymbols> = input.changes.iter().map(FileSymbols::of).collect();

    let changed = detect_changes(&symbols, &paths);

    classify_imports(&mut hunks, &input.changes, &symbols, &changed);

    uninit_members(&mut hunks, &input.changes);
    let (rule_hits, mut problems) = apply_rules(&mut hunks, &input);
    problems.append(&mut input_problems);
    problems.sort();
    problems.dedup();

    let facts = order::FileFacts {
        symbols: &symbols,
        changed: &changed,
    };
    let ordered = order::order_all(&hunks, &paths, &facts, &input.options);
    let placed = placements(&ordered, &hunks);
    let gids: Vec<String> = (0..ordered.groups.len())
        .map(|gi| format!("g{gi}"))
        .collect();
    // global reading order
    let order: Vec<OrderItem> = ordered
        .perm
        .iter()
        .map(|&i| {
            let (fi, li) = ordered.coord[i];
            OrderItem {
                path: input.changes[fi].path.clone(),
                hunk: placed[fi][li].id.clone(),
            }
        })
        .collect();
    // the drops leave `hunks` here: the output owns them, and nothing after
    // this reads them from the pass tables
    let mut dropped: Vec<Vec<DroppedHunk>> = hunks
        .iter_mut()
        .map(|f| std::mem::take(&mut f.dropped))
        .collect();
    let passes = Passes {
        hunks: &hunks,
        placed: &placed,
        ordered: &ordered,
        gids: &gids,
        rule_hits: &rule_hits,
        symbols: &symbols,
    };
    let mut files: Vec<FileOut> = input
        .changes
        .iter()
        .enumerate()
        .map(|(fi, change)| FileOut {
            path: change.path.clone(),
            hunks: (0..placed[fi].len())
                .map(|li| passes.hunk_out(fi, li))
                .collect(),
            degraded: hunks[fi].degraded,
            unsupported: lang::for_path(&change.path).is_none(),
            dropped: std::mem::take(&mut dropped[fi]),
        })
        .collect();

    let hid = |gi: usize| {
        let (fi, li) = ordered.coord[gi];
        placed[fi][li].id.clone()
    };
    let groups_out: Vec<Group> = ordered
        .groups
        .iter()
        .enumerate()
        .map(|(gi, info)| Group {
            id: gids[gi].clone(),
            reason: info.reason.clone(),
            members: info.members.iter().map(|&i| hid(i)).collect(),
        })
        .collect();
    let edges_out: Vec<Edge> = ordered
        .edges
        .iter()
        .map(|(from, to, why)| Edge {
            from: hid(*from),
            to: hid(*to),
            why: why.clone(),
        })
        .collect();
    let clusters: Vec<Vec<String>> = ordered
        .clusters
        .iter()
        .map(|c| c.iter().map(|&i| hid(i)).collect())
        .collect();

    let ledger = build_ledger(&files, &order, &facts);
    let notes = changeset_notes(&files, &ledger);
    arity_check(&mut files, &ledger, &input.changes);
    incomplete_rename(&mut files, &ledger, &input.changes, &symbols);
    // every pass is done with the trees; the client calls `run` again on each
    // reload, and holding this changeset's trees until then buys nothing
    lang::forget_trees();
    Output {
        schema: SCHEMA_VERSION,
        order,
        files,
        groups: groups_out,
        edges: edges_out,
        clusters,
        problems,
        notes,
        ledger,
    }
}

/// The built-in construct catalog and the caller's rules, evaluated against
/// every hunk: after the semantics they match on, before the ordering they can
/// influence. A rule's `noise` and `priority` reach the hunk itself; its
/// findings come back per file and hunk, to ride along to the output, with
/// the problems every rule set reported — once per rule, not once per file.
///
/// The catalog rides the same engine: it is the same mechanism with a
/// different `FindingSource`, so both sets share one parse per file. The
/// catalog is on unless the caller says otherwise, and a `disable` glob
/// silences a catalog entry the same way it silences one of their own: a
/// catalog rule is a rule, and the name is the name.
fn apply_rules(hunks: &mut [PerFileHunks], input: &Input) -> (Vec<Vec<Vec<Finding>>>, Vec<String>) {
    let mut catalog_problems = vec![];
    let catalog = catalog_for(&input.options, &mut catalog_problems);
    let mut engine = rules::Rules::with_catalog(&catalog, &input.options.rules);
    let mut rule_hits: Vec<Vec<Vec<Finding>>> = vec![];
    for (fi, change) in input.changes.iter().enumerate() {
        let query_rows = match (lang::for_path(&change.path), change.new.as_deref()) {
            (Some(spec), Some(new)) => engine.query_rows(spec, new),
            _ => HashMap::new(),
        };
        let file_lines = (
            change.old.as_deref().map(|o| o.lines().count()),
            change.new.as_deref().map_or(0, |n| n.lines().count()),
        );
        let per_file: Vec<Vec<Finding>> = (0..hunks[fi].raw.len())
            .map(|li| {
                engine.hits(
                    &hunk_facts(&hunks[fi], li, &change.path, file_lines),
                    &query_rows,
                )
            })
            .collect();
        rule_hits.push(cap_repeats(per_file));
    }
    engine.finish();
    for (fi, per_file) in rule_hits.iter().enumerate() {
        for (li, hits) in per_file.iter().enumerate() {
            if rules::any_noise(hits, &input.options.rules) {
                hunks[fi].sem[li].noise = true;
            }
            hunks[fi].sem[li].priority = rules::priority(hits, &input.options.rules);
        }
    }
    let mut problems = engine.problems.clone();
    // a catalog that failed to parse is our bug, not a fault in the caller's
    // rules — but they still deserve to know this run is missing every
    // construct advisory
    problems.extend(catalog::problem().map(str::to_string));
    // and a `disable` glob nobody can parse silences nothing
    problems.extend(catalog_problems);
    problems.sort();
    problems.dedup();
    (rule_hits, problems)
}

/// How many times one rule speaks about one file before it starts counting
/// instead. A construct a file uses everywhere — OpenFOAM wraps every `new`
/// in a `tmp<>` — produced twenty-five identical notes in one review, of
/// which a labeled sample judged one worth reading.
const RULE_REPEATS: usize = 3;

/// The same rule, past `RULE_REPEATS` hunks of one file, stops repeating and
/// says how many more there are. Nothing is dropped silently: the last
/// finding that speaks carries the count of the ones that did not.
fn cap_repeats(mut per_file: Vec<Vec<Finding>>) -> Vec<Vec<Finding>> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for hits in per_file.iter_mut() {
        hits.retain(|f| {
            let n = seen.entry(f.name.clone()).or_default();
            *n += 1;
            *n <= RULE_REPEATS
        });
    }
    for (name, total) in seen {
        let more = total.saturating_sub(RULE_REPEATS);
        if more == 0 {
            continue;
        }
        // the last one that still speaks carries the count of the rest
        let last = per_file
            .iter_mut()
            .rev()
            .flat_map(|hits| hits.iter_mut().rev())
            .find(|f| f.name == name);
        if let Some(f) = last {
            f.message
                .push_str(&format!("\n(and {more} more in this file)"));
        }
    }
    per_file
}

/// What a rule can ask about one hunk.
fn hunk_facts<'a>(
    f: &'a PerFileHunks,
    li: usize,
    path: &'a str,
    file_lines: (Option<usize>, usize),
) -> rules::HunkFacts<'a> {
    let sem = &f.sem[li];
    let [r0, r1] = f.raw[li].new_range;
    rules::HunkFacts {
        path,
        rows: (r0, r1),
        category: sem.category,
        enclosing: sem.enclosing.as_deref(),
        enclosing_kind: sem.enclosing_kind,
        defines: &sem.defines,
        uses: &sem.uses,
        imports: &sem.imports,
        noise: sem.noise,
        comment: f.comment[li],
        def_lines: sem.def_lines,
        def_params: sem.def_params,
        nesting: sem.nesting,
        file_lines,
        recursive: sem.recursive,
        container_members: &sem.container_members,
        uninit_members: &sem.uninit_members,
    }
}

/// What the ordering decided about each hunk, at the hunk: its id, its global
/// index, and its rank within its file across the global reading order.
fn placements(ordered: &order::OrderedAll, hunks: &[PerFileHunks]) -> Vec<Vec<HunkPlace>> {
    let mut placed: Vec<Vec<HunkPlace>> = hunks
        .iter()
        .map(|f| vec![HunkPlace::default(); f.raw.len()])
        .collect();
    for (i, &(fi, li)) in ordered.coord.iter().enumerate() {
        placed[fi][li].id = format!("h{i}");
        placed[fi][li].global = i;
    }
    let mut file_counter = vec![0usize; hunks.len()];
    for &i in &ordered.perm {
        let (fi, li) = ordered.coord[i];
        placed[fi][li].in_file = file_counter[fi];
        file_counter[fi] += 1;
    }
    placed
}

/// What `hunk_out` reads: the per-file tables and the global order.
struct Passes<'a> {
    hunks: &'a [PerFileHunks],
    placed: &'a [Vec<HunkPlace>],
    ordered: &'a order::OrderedAll,
    gids: &'a [String],
    rule_hits: &'a [Vec<Vec<Finding>>],
    symbols: &'a [FileSymbols],
}

impl Passes<'_> {
    /// One hunk of the output, from everything the passes learned about it.
    fn hunk_out(&self, fi: usize, li: usize) -> HunkOut {
        let (f, place) = (&self.hunks[fi], &self.placed[fi][li]);
        let (sem, gi) = (&f.sem[li], place.global);
        HunkOut {
            id: place.id.clone(),
            old_range: f.raw[li].old_range,
            new_range: f.raw[li].new_range,
            category: sem.category,
            enclosing: sem.enclosing.clone(),
            enclosing_kind: sem.enclosing_kind,
            defines: sem.defines.clone(),
            uses: sem.uses.clone(),
            group: self.gids[self.ordered.group_idx[gi]].clone(),
            order_index: place.in_file,
            rationale: self.ordered.rationale[gi].clone(),
            noise: sem.noise,
            comment: f.comment[li],
            details: sem.details.clone(),
            notes: sem.notes.clone(),
            symbols: sem.symbols.clone(),
            // one list: the construct catalog, the caller's rules, and anything a
            // client adds later, all say the same kind of thing
            findings: sem
                .advisories
                .iter()
                .cloned()
                .chain(self.rule_hits[fi].get(li).into_iter().flatten().cloned())
                .collect(),
            uses_at: order::introduced_bindings(sem, &self.symbols[fi].old_locals)
                .filter(|b| !b.uses.is_empty())
                .map(|b| UseSite {
                    name: b.name.clone(),
                    rows: b.uses.clone(),
                })
                .collect(),
        }
    }
}

/// Fill `old`/`new` from a `diff` wherever they can be recovered, so every
/// stage that reads file content sees the same two sides.
///
/// The reconstruction used to live inside `build_change` and stay there: hunks
/// were computed from the rebuilt text, but the symbol stage still saw
/// `old: None` and read the file as having had no definitions at all — so on
/// the whole patch path every pre-existing definition was reported as a
/// signature change, and no hunk ever reported a body-only edit.
fn fill_sides(input: &mut Input) -> Vec<String> {
    let full_context = input.options.full_context;
    let mut problems = vec![];
    for c in &mut input.changes {
        if c.new.is_some() {
            continue;
        }
        let Some(diff) = c.diff.as_deref() else {
            continue;
        };
        // L1: the caller gave the old side, so the new one is old + diff
        if let Some(old) = c.old.as_deref() {
            c.new = patch::apply(old, diff);
            continue;
        }
        // L2: an added file, or a caller-asserted full-context patch
        let pf = patch::parse_file_diff(diff, full_context);
        let whole_file = pf.hunks.len() == 1 && pf.hunks[0].old_range[0] == 1;
        match (pf.old, pf.new) {
            (Some(old), Some(new)) => (c.old, c.new) = (Some(old), Some(new)),
            // The caller asserted a complete patch and sent a context-limited
            // one. Saying so beats a confident answer read off three lines of
            // context per hunk: the assertion is the only reason the engine
            // would have trusted it.
            _ if full_context && !whole_file => problems.push(format!(
                "{}: full_context was asserted but the patch carries {} hunks \
                 rather than the whole file — positional order only",
                c.path,
                pf.hunks.len()
            )),
            _ => {}
        }
    }
    problems
}

/// Hunks and semantics for every changed file, with the drops the caller asked
/// for already taken out.
fn build_hunks(input: &Input, templates: &[TemplateFacts]) -> Vec<PerFileHunks> {
    input
        .changes
        .iter()
        .zip(templates)
        .map(|(change, facts)| file_hunks(change, facts, &input.options))
        .collect()
}

/// One file's hunks: built, told what its template said, and sieved.
fn file_hunks(change: &Change, facts: &TemplateFacts, options: &Options) -> PerFileHunks {
    let (raw, mut sem, degraded, com, sw) = build_change(change, options.full_context);
    apply_template_facts(facts, &change.path, &raw, &mut sem);
    if degraded {
        eprintln!(
            "ordo: {}: diff lacks full context — positional order only (use old/new, or pass a full-context patch with `full_context`/`--full-context`)",
            change.path
        );
    }
    let mut f = PerFileHunks {
        raw: vec![],
        sem: vec![],
        degraded,
        comment: vec![],
        switched: vec![],
        dropped: vec![],
    };
    for (((r, mut s), c), w) in raw.into_iter().zip(sem).zip(com).zip(sw) {
        // `only_comments` drops, because there the caller asked for a subset;
        // the drop is recorded with its range so `hunks + dropped` accounts
        // for every hunk the diff produced
        if options.only_comments && !c {
            f.dropped.push(DroppedHunk {
                reason: DropReason::NonComment,
                old_range: r.old_range,
                new_range: r.new_range,
            });
            continue;
        }
        // A pure-import hunk is *noise*, not nothing: it follows from the real
        // change rather than being it, so it never leads the reading order and
        // never seeds a def→use edge — but it stays visible, dimmed, where the
        // diff put it. Dropping it outright made ordo asymmetric in a way
        // reviewers noticed: a removed import was reported ("removes import
        // loguru", from the deletion path) while an added one vanished, so a
        // moved import read as a deletion with no counterpart.
        if s.category == Category::Import {
            s.noise = true;
        }
        f.raw.push(r);
        f.sem.push(s);
        f.comment.push(c);
        f.switched.push(w);
    }
    f
}

/// Three corrections to import classification that need the old side, applied
/// after `analyze` has had its say: a pure deletion classified from the side it
/// actually has, a reordering told apart from an arrival, and the blank line a
/// moved import leaves behind marked as the formatting it is.
fn classify_imports(
    hunks: &mut [PerFileHunks],
    changes: &[Change],
    symbols: &[FileSymbols],
    changed: &[FileChanges],
) {
    for fi in 0..changes.len() {
        name_deleted_imports(&mut hunks[fi], &changes[fi], &symbols[fi]);
        mark_moved_imports(&mut hunks[fi], &changes[fi]);
        mark_import_residue(&mut hunks[fi], &changes[fi], &symbols[fi], &changed[fi]);
        keep_removed_definitions(&mut hunks[fi], &changed[fi]);
    }
}

/// A hunk is classified from its new side, so one that deletes a definition
/// and leaves an import line behind read as an import hunk and was dimmed —
/// a removed public export hidden as noise. The old side knows better: a
/// definition removed inside the hunk's rows makes it a real change.
fn keep_removed_definitions(f: &mut PerFileHunks, changed: &FileChanges) {
    for sem in &mut f.sem {
        let [o0, o1] = sem.old_range;
        if sem.category != Category::Import || o0 == 0 || o0 > o1 {
            continue;
        }
        let removes_def = changed
            .removals
            .iter()
            .any(|r| r.kind != RemovalKind::Import && (o0..=o1).contains(&r.row));
        if removes_def {
            sem.category = Category::Other;
            sem.noise = false;
        }
    }
}

// A hunk that only deletes has no new side to classify from, so a removed
// import used to read as a plain `other` hunk while an added one was an
// import. Classify a pure deletion from the side it actually has.
fn name_deleted_imports(f: &mut PerFileHunks, change: &Change, symbols: &FileSymbols) {
    let (Some(spec), Some(old_src)) = (lang::for_path(&change.path), change.old.as_deref()) else {
        return;
    };
    let rows = extract::import_row_set(spec, old_src);
    if rows.is_empty() {
        return;
    }
    let old_lines: Vec<&str> = old_src.lines().collect();
    let PerFileHunks { raw, sem: sems, .. } = f;
    for (li, sem) in sems.iter_mut().enumerate() {
        let [o0, o1] = sem.old_range;
        let [n0, n1] = raw[li].new_range;
        let deletes_only = n0 > n1;
        if !deletes_only || !(1..=o1).contains(&o0) || !(o0..=o1).all(|r| rows.contains(&r)) {
            continue;
        }
        sem.category = Category::Import;
        sem.noise = true;
        let deleted = &old_lines[o0 - 1..o1.min(old_lines.len())];
        sem.imports = gone_imports(symbols, deleted, o0..=o1);
    }
}

/// What left: an import recorded on a deleted row, or — for one member dropped
/// from a multi-line `import { a, b }`, which sits rows below the statement
/// its removal is recorded against — a deleted line that is just that name. A
/// name the new side still imports is a moved import, and the residue it
/// leaves behind stays "formatting only".
fn gone_imports(
    symbols: &FileSymbols,
    deleted: &[&str],
    rows: RangeInclusive<usize>,
) -> Vec<String> {
    let member_line = |nm: &str| {
        deleted.iter().any(|l| {
            let l = l.trim().trim_end_matches(',').trim();
            l == nm || l.ends_with(&format!(" as {nm}"))
        })
    };
    let mut gone: Vec<String> = symbols
        .old_rows
        .1
        .iter()
        .filter(|(nm, row)| {
            !symbols.new_imports.contains(nm) && (rows.contains(row) || member_line(nm))
        })
        .map(|(nm, _)| nm.clone())
        .collect();
    gone.sort();
    gone.dedup();
    gone
}

// A pure-import hunk whose statements all existed in the old file is a
// reordering, not an arrival: "moves import pg" rather than "changes".
fn mark_moved_imports(f: &mut PerFileHunks, change: &Change) {
    let (Some(spec), Some(old_src), Some(new_src)) = (
        lang::for_path(&change.path),
        change.old.as_deref(),
        change.new.as_deref(),
    ) else {
        return;
    };
    let before = extract::import_statements(spec, old_src);
    if before.is_empty() {
        return;
    }
    let new_lines: Vec<&str> = new_src.lines().collect();
    let PerFileHunks { raw, sem: sems, .. } = f;
    for (li, sem) in sems.iter_mut().enumerate() {
        let [r0, r1] = raw[li].new_range;
        if sem.category != Category::Import || r0 == 0 || r0 > r1 {
            continue;
        }
        // every non-blank line of the hunk has to be an import the old file
        // already had; one new line among them makes this an arrival
        let mut lines = (r0..=r1)
            .filter_map(|r| new_lines.get(r - 1))
            .map(|l| extract::squeeze(l))
            .filter(|l| !l.is_empty())
            .peekable();
        sem.import_moved = lines.peek().is_some() && lines.all(|l| before.contains(&l));
    }
}

// An import line that moved, leaving a blank line behind, reads as a bare
// change: the new side carries no rows to classify by, and the old side is
// where the meaning was. ordo already treats pure-import hunks as
// bookkeeping, so the residue of reordering them is formatting. A genuinely
// deleted import is excluded — that one is named.
fn mark_import_residue(
    f: &mut PerFileHunks,
    change: &Change,
    symbols: &FileSymbols,
    changed: &FileChanges,
) {
    let Some(new) = change.new.as_deref() else {
        return;
    };
    let new_lines: Vec<&str> = new.lines().collect();
    let import_rows: HashSet<usize> = symbols.old_rows.1.iter().map(|(_, r)| *r).collect();
    let PerFileHunks { raw, sem: sems, .. } = f;
    for (li, sem) in sems.iter_mut().enumerate() {
        let [o0, o1] = sem.old_range;
        if sem.noise || !(1..=o1).contains(&o0) {
            continue;
        }
        // a *deleted* import is a real removal and is named as such; this is
        // only the residue of one that moved, where nothing was removed
        let named = changed.removals.iter().any(|r| (o0..=o1).contains(&r.row));
        let [n0, n1] = raw[li].new_range;
        let new_blank =
            (n0..=n1).all(|r| new_lines.get(r - 1).is_some_and(|l| l.trim().is_empty()));
        if new_blank && !named && (o0..=o1).all(|r| import_rows.contains(&r)) {
            sem.noise = true;
        }
    }
}

/// Drop the "uninitialized member" note for anything this change does
/// initialize, anywhere in it — the declaration and the constructor are
/// routinely in different files.
fn uninit_members(hunks: &mut [PerFileHunks], changes: &[Change]) {
    // A data member added in this change is initialized in-class or in a
    // constructor's initializer list — and if that constructor changed, its
    // file is in the diff. So "no initializer anywhere in the change" is
    // decidable from the change alone, header and `.cpp` together. A member
    // the old side already had is not this change's to answer for.
    //
    // Keyed by `Class.member`, so one class's initializer cannot answer for
    // another's same-named member while the header/`.cpp` pair still meets —
    // see `extract::owned_member`.
    let inits: HashSet<String> = changes
        .iter()
        .filter_map(|c| Some((member_lang(c)?, c.new.as_deref()?)))
        .flat_map(|(spec, new)| extract::field_initializers(spec, new))
        .collect();
    for (fi, change) in changes.iter().enumerate() {
        let Some(spec) = member_lang(change) else {
            continue;
        };
        let old_names = old_member_keys(spec, change);
        for sem in &mut hunks[fi].sem {
            sem.uninit_members
                .retain(|n| !inits.contains(n) && !old_names.contains(n));
            for n in &sem.uninit_members {
                // the key is qualified so header and `.cpp` match; the note
                // sits on the hunk that declares the member, where the class
                // is already on screen, so it reads the bare name
                let bare = n.rsplit('.').next().unwrap_or(n);
                sem.notes.push(format!("uninitialized member {bare}"));
            }
        }
    }
}

/// The languages whose data members the walker reports uninitialized.
fn member_lang(change: &Change) -> Option<&'static LangSpec> {
    lang::for_path(&change.path).filter(|s| matches!(s.name, "cpp" | "java"))
}

/// `Class.member` for every member the old side already had.
fn old_member_keys(spec: &LangSpec, change: &Change) -> HashSet<String> {
    let Some(old) = change.old.as_deref() else {
        return HashSet::new();
    };
    extract::member_rows(spec, old)
        .into_iter()
        .map(|(_, n, _, ctr)| match ctr {
            Some(c) => format!("{c}.{n}"),
            None => n,
        })
        .collect()
}

/// Rename, move, extraction, body-only edits and removals, per file.
///
/// Everything here is decided from the two sides' symbol tables alone — one
/// pass over `symbols`, producing one `FileChanges` per file. It was 156 lines
/// inline in `run`, between the pass that built the symbol tables and the one
/// that classified imports, sharing five mutable vectors with both.
fn detect_changes(symbols: &[FileSymbols], paths: &[String]) -> Vec<FileChanges> {
    let n = symbols.len();
    let mut changed: Vec<FileChanges> = (0..n).map(|_| FileChanges::default()).collect();
    let appeared = appeared_defs(symbols);
    for fi in 0..n {
        let fs = &symbols[fi];
        let (od, nd) = (&fs.old_defs, &fs.new_defs);
        let mut removed_d: Vec<String> = od.difference(nd).cloned().collect();
        let mut added_d: Vec<String> = nd.difference(od).cloned().collect();
        removed_d.sort();
        added_d.sort();
        let ren = renames(fs, &removed_d, &added_d);
        let renamed_old: HashSet<String> = ren.values().cloned().collect();
        let mut moved_to: HashMap<String, String> = HashMap::new();
        for (name, tgt) in moves_out(fs, fi, &removed_d, &renamed_old, &appeared) {
            changed[tgt]
                .moved_in
                .insert(name.clone(), paths[fi].clone());
            moved_to.insert(name, paths[tgt].clone());
        }
        changed[fi].relocated = relocations(fs, &added_d, &ren);
        changed[fi].body_only = body_only(fs);
        changed[fi].removals = removals(fs, &paths[fi], &renamed_old, &moved_to);
        changed[fi].rename = ren;
    }
    changed
}

fn body_of<'a>(list: &'a [extract::Body], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|(nm, _, _, _)| nm == name)
        .map(|(_, _, b, _)| b.as_str())
}

fn header_of<'a>(list: &'a [extract::Body], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|(nm, _, _, _)| nm == name)
        .map(|(_, h, _, _)| h.as_str())
}

/// P12.1: freshly-appeared new defs by (name, body) → file, for moves
fn appeared_defs(symbols: &[FileSymbols]) -> HashMap<(String, String), usize> {
    let mut appeared: HashMap<(String, String), Option<usize>> = HashMap::new();
    for (fi, fs) in symbols.iter().enumerate() {
        for (name, _, body, _) in &fs.new_body {
            if body.len() >= 8 && !fs.old_defs.contains(name) {
                // Two files receiving the same (name, body) name no
                // destination: a yaml key with a common value, a boilerplate
                // struct. Claiming the first is how a move ended up pointing
                // at a file the code never came from.
                appeared
                    .entry((name.clone(), body.clone()))
                    .and_modify(|e| {
                        if *e != Some(fi) {
                            *e = None;
                        }
                    })
                    .or_insert(Some(fi));
            }
        }
    }
    appeared
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect()
}

/// #7 rename (same file): new name → old name, by body match, then a
/// lone-pair fallback
fn renames(fs: &FileSymbols, removed: &[String], added: &[String]) -> HashMap<String, String> {
    // Every pair that could be a rename at all: the body reappears verbatim,
    // or the two are recognisably the same code. `similar` is the guard —
    // deleting `foo` and adding an unrelated `bar` shares no body lines, so
    // the pair never comes up for consideration.
    // one index per side, so a file of N definitions is walked twice rather
    // than once per candidate pair
    let (old_body, new_body) = (index_bodies(&fs.old_body), index_bodies(&fs.new_body));
    let mut pairs: Vec<(bool, bool, usize, &String, &String)> = vec![];
    for r in removed {
        let rb = old_body
            .get(r.as_str())
            .map(|(whole, _)| *whole)
            .filter(|b| b.len() >= 8);
        for a in added {
            let exact = rb.is_some() && new_body.get(a.as_str()).map(|(w, _)| *w) == rb;
            if !exact && !alike(&old_body, r, &new_body, a) {
                continue;
            }
            // One name inside the other is a rename on sight — `is_hidden` →
            // `is_hidden_entry`, `get_stream` → `_get_stream` — and outranks
            // any body evidence. A shared tail is not: `get_value` and
            // `set_value` are two functions, not one renamed.
            let (short, long) = if r.len() <= a.len() { (r, a) } else { (a, r) };
            pairs.push((
                long.contains(short.as_str()),
                exact,
                name_overlap(r, a),
                r,
                a,
            ));
        }
    }
    // Best pair first, across the whole file rather than per removed name.
    // An unmistakable name wins outright: two functions can share a body to
    // the byte — two stream openers differing only in their name, a helper
    // and the function that now wraps it — and then the body says nothing
    // about which became which. Failing that an exact body outranks a merely
    // similar one, and the closest spelling breaks what is left. Ordering by
    // name last keeps the result stable.
    pairs.sort_by_key(|(obvious, exact, overlap, r, a)| {
        (std::cmp::Reverse((*obvious, *exact, *overlap)), *r, *a)
    });
    let mut ren = HashMap::new();
    let (mut taken, mut claimed): (HashSet<&String>, HashSet<&String>) = Default::default();
    for (_, _, _, r, a) in pairs {
        if taken.contains(r) || claimed.contains(a) {
            continue;
        }
        taken.insert(r);
        claimed.insert(a);
        ren.insert(a.clone(), r.clone());
    }
    ren
}

/// How much of two names is shared at either end: `get_stream` and
/// `_get_stream` share ten characters, `get_stream` and `read_text` none.
fn name_overlap(a: &str, b: &str) -> usize {
    let prefix = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let suffix = a
        .bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (prefix + suffix).min(a.len().min(b.len()))
}

/// P12.1 moves: a removed (non-renamed) def whose body reappears same-name in
/// another file → that file's index
fn moves_out(
    fs: &FileSymbols,
    fi: usize,
    removed: &[String],
    renamed_old: &HashSet<String>,
    appeared: &HashMap<(String, String), usize>,
) -> HashMap<String, usize> {
    let mut moved_out = HashMap::new();
    for r in removed.iter().filter(|r| !renamed_old.contains(*r)) {
        let rb = match body_of(&fs.old_body, r) {
            Some(b) if b.len() >= 8 => b,
            _ => continue,
        };
        if let Some(&tgt) = appeared.get(&(r.clone(), rb.to_string())) {
            if tgt != fi {
                moved_out.insert(r.clone(), tgt);
            }
        }
    }
    moved_out
}

/// P16 relocation: an added def whose body-lines overlap a still-present old
/// def → it was extracted/relocated out of that def (body may differ)
fn relocations(
    fs: &FileSymbols,
    added: &[String],
    ren: &HashMap<String, String>,
) -> HashMap<String, String> {
    added
        .iter()
        .filter(|a| !ren.contains_key(*a))
        .filter_map(|a| Some((a.clone(), relocation_source(fs, a)?)))
        .collect()
}

/// The still-present old def sharing the most body lines with the added `a`:
/// at least three of them, and at least half of `a`.
fn relocation_source(fs: &FileSymbols, a: &str) -> Option<String> {
    let al = match fs.new_body.iter().find(|(nm, _, _, _)| nm == a) {
        Some((_, _, _, l)) if l.len() >= 3 => l,
        _ => return None,
    };
    let aset: HashSet<&String> = al.iter().collect();
    // What the source still has. An extraction takes code OUT of it, so a
    // shared line that is still there was not extracted: a new function
    // modelled on an existing one shares its shape without touching it, and
    // used to be reported as extracted from the function it resembles.
    let kept = |x: &str| -> HashSet<&String> {
        fs.new_body
            .iter()
            .find(|(nm, _, _, _)| nm == x)
            .map(|(_, _, _, l)| l.iter().collect())
            .unwrap_or_default()
    };
    fs.old_body
        .iter()
        .filter(|(x, _, _, _)| x != a && fs.new_defs.contains(x))
        .map(|(x, _, _, xl)| {
            let still = kept(x);
            (
                x,
                xl.iter()
                    .filter(|l| aset.contains(*l) && !still.contains(*l))
                    .count(),
            )
        })
        .filter(|(_, shared)| *shared >= 3 && shared * 2 >= al.len())
        // the first of equally good candidates wins
        .fold(None, |best: Option<(&String, usize)>, (x, shared)| {
            if best.is_none_or(|(_, s)| shared > s) {
                Some((x, shared))
            } else {
                best
            }
        })
        .map(|(x, _)| x.clone())
}

/// #4: an existing def whose signature is unchanged → body-only edit, not a
/// signature change. Positive-only: unknown headers keep "changes signature
/// of".
fn body_only(fs: &FileSymbols) -> HashSet<String> {
    fs.old_defs
        .intersection(&fs.new_defs)
        .filter(|name| {
            let (o, m) = (header_of(&fs.old_body, name), header_of(&fs.new_body, name));
            o.is_some() && o == m
        })
        .cloned()
        .collect()
}

/// #7 delete / #5 import remove / P12.1 move-out (else "removes"), plus a
/// removed file-scope binding: not a definition, but naming it beats the
/// "removes N lines" fallback a module constant would get otherwise
fn removals(
    fs: &FileSymbols,
    path: &str,
    renamed_old: &HashSet<String>,
    moved_to: &HashMap<String, String>,
) -> Vec<Removal> {
    let (odr, oir) = &fs.old_rows;
    let (od, nd, ni) = (&fs.old_defs, &fs.new_defs, &fs.new_imports);
    let prose = lang::for_path(path).is_some_and(|s| s.prose);
    let removal = |(name, row): &(String, usize), kind: RemovalKind| Removal {
        row: *row,
        name: name.clone(),
        kind,
    };
    let defs = odr
        .iter()
        .filter(|(name, _)| !nd.contains(name) && !renamed_old.contains(name))
        .map(|d| {
            let kind = match moved_to.get(&d.0) {
                Some(tgt) => RemovalKind::MovedTo(tgt.clone()),
                None if prose => RemovalKind::Section,
                None => RemovalKind::Def,
            };
            removal(d, kind)
        });
    let imports = oir
        .iter()
        .filter(|(name, _)| !ni.contains(name))
        .map(|i| removal(i, RemovalKind::Import));
    let binds = fs
        .old_binds
        .iter()
        .filter(|(name, _)| {
            !fs.new_binds.contains(name) && !nd.contains(name) && !od.contains(name)
        })
        .map(|b| removal(b, RemovalKind::Def));
    defs.chain(imports).chain(binds).collect()
}

/// P23.2: a definition whose signature changed, against the calls to it in
/// this same change. The most common way an edit goes wrong is that the
/// function moved and one caller did not follow.
///
/// Deliberately narrow — a false "wrong number of arguments" is worse than a
/// missed one, so this only speaks when it can be exact. See `signatures` and
/// `call_sites` for what is skipped (variadics, methods, keyword arguments,
/// qualified callees). Only callers *in the change* are considered, which is
/// the honest scope: those are the ones the author touched.
fn arity_check(files: &mut [FileOut], ledger: &[LedgerEntry], changes: &[Change]) {
    let changed: Vec<&LedgerEntry> = ledger
        .iter()
        .filter(|e| e.change == SymbolChange::Signature)
        .collect();
    if changed.is_empty() {
        return;
    }
    let CallTable { sigs, calls } = signature_table(changes);
    for e in changed {
        let Some(Some(sig)) = sigs.get(&e.name) else {
            continue; // its arity could not be stated exactly, or is ambiguous
        };
        let Some(note) = arity_note(&e.name, sig, &calls, changes) else {
            continue;
        };
        // the note belongs on the hunk that changed the signature — that is
        // where a reviewer is standing when the question arises
        for f in files.iter_mut() {
            if let Some(h) = f.hunks.iter_mut().find(|h| h.id == e.at) {
                h.notes.push(note);
                break;
            }
        }
    }
}

/// the new arity of everything in the change, and every call to anything
struct CallTable {
    /// name → its arity, `None` once two files disagree on it
    sigs: HashMap<String, Option<extract::SigInfo>>,
    /// (file index, call) for every call in the change
    calls: Vec<(usize, extract::CallSite)>,
}

fn signature_table(changes: &[Change]) -> CallTable {
    let mut sigs: HashMap<String, Option<extract::SigInfo>> = HashMap::new();
    let mut calls: Vec<(usize, extract::CallSite)> = vec![];
    for (fi, c) in changes.iter().enumerate() {
        let (Some(spec), Some(new)) = (lang::for_path(&c.path), c.new.as_deref()) else {
            continue;
        };
        for s in extract::signatures(spec, new) {
            record_signature(&mut sigs, s);
        }
        calls.extend(
            extract::call_sites(spec, new)
                .into_iter()
                .map(|cs| (fi, cs)),
        );
    }
    CallTable { sigs, calls }
}

/// A bare name is the only key the call sites can be matched on, so two files
/// defining the same name are indistinguishable here. The last one used to win
/// and its arity was then checked against the other's callers; an ambiguous
/// name is dropped instead, which is what "only speaks when it can be exact"
/// has to mean.
fn record_signature(sigs: &mut HashMap<String, Option<extract::SigInfo>>, s: extract::SigInfo) {
    use std::collections::hash_map::Entry;
    match sigs.entry(s.name.clone()) {
        Entry::Occupied(mut e) => {
            let differs = e
                .get()
                .as_ref()
                .is_some_and(|p| (p.required, p.total) != (s.required, s.total));
            if differs {
                e.insert(None);
            }
        }
        Entry::Vacant(e) => {
            e.insert(Some(s));
        }
    }
}

/// The note for a signature whose callers in the change do not all fit it,
/// when any does not.
fn arity_note(
    name: &str,
    sig: &extract::SigInfo,
    calls: &[(usize, extract::CallSite)],
    changes: &[Change],
) -> Option<String> {
    let bad: Vec<&(usize, extract::CallSite)> = calls
        .iter()
        .filter(|(_, c)| c.name == name && (c.argc < sig.required || c.argc > sig.total))
        .collect();
    if bad.is_empty() {
        return None;
    }
    let total_calls = calls.iter().filter(|(_, c)| c.name == name).count();
    let where_ = bad
        .iter()
        .take(3)
        .map(|(fi, c)| format!("{}:L{}", changes[*fi].path, c.row + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let more = if bad.len() > 3 {
        format!(" and {} more", bad.len() - 3)
    } else {
        String::new()
    };
    let expected = if sig.required == sig.total {
        format!("{}", sig.required)
    } else {
        format!("{}–{}", sig.required, sig.total)
    };
    Some(format!(
        "{} of {total_calls} call sites in this change do not pass {expected} arguments to {name} ({where_}{more})",
        bad.len()
    ))
}

/// P23.2: a rename that did not finish. Rename detection already says
/// `renames parse_cfg → load_cfg`; the question it leaves open is whether the
/// old name still appears anywhere. Searched across the *whole new content* of
/// every changed file, not just its hunks — a reference on a line nobody
/// touched is exactly the one that gets missed.
///
/// Silent when the old name is still defined somewhere in the change: then it
/// is a name that legitimately still exists, not an orphaned reference. Only
/// identifiers count, so the name surviving in a string or a comment says
/// nothing. **Ceiling:** files *in the change* only — a caller in a file the
/// author never opened is invisible to the pure engine, and finding it needs
/// the repo access the `ordo` reviewer has.
fn incomplete_rename(
    files: &mut [FileOut],
    ledger: &[LedgerEntry],
    changes: &[Change],
    symbols: &[FileSymbols],
) {
    for e in ledger.iter().filter(|e| e.change == SymbolChange::Renamed) {
        let Some(old) = e.from.as_deref() else {
            continue;
        };
        if symbols.iter().any(|s| s.new_defs.contains(old)) {
            continue; // the old name still defines something; not an orphan
        }
        let left = uses_of(changes, old);
        if left.is_empty() {
            continue;
        }
        let more = if left.len() > 3 {
            format!(" and {} more", left.len() - 3)
        } else {
            String::new()
        };
        let note = format!(
            "{old} still used at {}{more} after the rename to {}",
            left.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
            e.name
        );
        if let Some(h) = files
            .iter_mut()
            .flat_map(|f| &mut f.hunks)
            .find(|h| h.id == e.at)
        {
            h.notes.push(note);
        }
    }
}

/// `path:Lrow` for every identifier `name` on the new side of the change.
fn uses_of(changes: &[Change], name: &str) -> Vec<String> {
    changes
        .iter()
        .filter_map(|c| Some((c, lang::for_path(&c.path)?, c.new.as_deref()?)))
        .flat_map(|(c, spec, new)| {
            extract::identifier_rows(spec, new, name)
                .into_iter()
                .map(move |row| format!("{}:L{}", c.path, row + 1))
        })
        .collect()
}

/// P23.1: what happened to each *symbol*, rather than to each hunk. Every field
/// is already computed — this is a second projection of `symbols`, the status
/// maps in `FileFacts` and the `uses` on every hunk, not new analysis.
///
/// Entries follow the reading order of the hunk that defines them, so the
/// ledger and the hunk list tell the same story in the same sequence.
fn build_ledger(
    files: &[FileOut],
    order: &[OrderItem],
    facts: &order::FileFacts,
) -> Vec<LedgerEntry> {
    // fan-in: which hunks use a given name, anywhere in the change
    let mut users: HashMap<&str, Vec<&str>> = HashMap::new();
    for f in files {
        for h in &f.hunks {
            for u in &h.uses {
                users.entry(u.as_str()).or_default().push(h.id.as_str());
            }
        }
    }
    let mut ledger = Ledger {
        // global reading position of every hunk, so the ledger can be sorted
        // the way the review is
        pos: order
            .iter()
            .enumerate()
            .map(|(i, o)| (o.hunk.as_str(), i))
            .collect(),
        users,
        seen: HashSet::new(),
        out: vec![],
    };
    for (fi, f) in files.iter().enumerate() {
        ledger.symbol_entries(f, &facts.changed[fi], &facts.symbols[fi]);
        ledger.body_edit_entries(f, &facts.changed[fi]);
        ledger.removal_entries(f, &facts.changed[fi]);
    }
    ledger.out.sort_by_key(|(p, _)| *p);
    ledger.out.into_iter().map(|(_, e)| e).collect()
}

/// The ledger as it is built: one entry per symbol, each with its reading
/// position, for `build_ledger` to sort by.
struct Ledger<'a> {
    pos: HashMap<&'a str, usize>,
    users: HashMap<&'a str, Vec<&'a str>>,
    /// (path, name, scope) already entered — one line per symbol, not per
    /// hunk that touches it
    seen: HashSet<(String, String, Option<String>)>,
    out: Vec<(usize, LedgerEntry)>,
}

impl Ledger<'_> {
    /// the hunks using `name`; a symbol never counts as using itself, so the
    /// hunk it sits in is left out when given
    fn used_by(&self, name: &str, except: Option<&str>) -> Vec<String> {
        self.users
            .get(name)
            .map(|v| {
                v.iter()
                    .filter(|id| except.is_none_or(|e| **id != e))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn push(&mut self, at: &str, entry: LedgerEntry) {
        let pos = self.pos.get(at).copied().unwrap_or(usize::MAX);
        self.out.push((pos, entry));
    }

    fn symbol_entries(&mut self, f: &FileOut, c: &FileChanges, symbols: &FileSymbols) {
        for h in &f.hunks {
            for sym in &h.symbols {
                let key = (f.path.clone(), sym.name.clone(), sym.scope.clone());
                if !self.seen.insert(key) {
                    continue;
                }
                let n = sym.name.as_str();
                let (change, from) = symbol_change(c, symbols, n);
                let entry = LedgerEntry {
                    name: sym.name.clone(),
                    kind: Some(sym.kind.clone()),
                    scope: sym.scope.clone(),
                    path: f.path.clone(),
                    at: h.id.clone(),
                    change,
                    from,
                    used_by: self.used_by(n, Some(&h.id)),
                };
                self.push(&h.id, entry);
            }
        }
    }

    // A body-only edit introduces no symbol — the def's declaration line is
    // not in the hunk — so it is found through the container instead: a
    // hunk whose enclosing is a plain definition (`enclosing_kind` is None)
    // and which declares nothing of its own edited that definition's body.
    fn body_edit_entries(&mut self, f: &FileOut, c: &FileChanges) {
        for h in &f.hunks {
            if !h.symbols.is_empty() || h.enclosing_kind.is_some() {
                continue;
            }
            let Some(name) = h.enclosing.as_deref() else {
                continue;
            };
            if !self.seen.insert((f.path.clone(), name.to_string(), None)) {
                continue;
            }
            let bare = order::bare_name(name);
            let change = if c.body_only.contains(bare) {
                SymbolChange::Body
            } else {
                SymbolChange::Signature
            };
            let entry = LedgerEntry {
                name: name.to_string(),
                kind: None,
                scope: None,
                path: f.path.clone(),
                at: h.id.clone(),
                change,
                from: None,
                used_by: self.used_by(bare, Some(&h.id)),
            };
            self.push(&h.id, entry);
        }
    }

    // A removed symbol has no defining node left to read a kind off, so the
    // entry is built from `c.removals` — the same set the rationale layer
    // names, which already excludes a symbol that left because it was renamed
    // or moved to another file. An import is not a symbol the ledger tracks,
    // and a move is reported as `Moved` from the arriving side.
    fn removal_entries(&mut self, f: &FileOut, c: &FileChanges) {
        for r in &c.removals {
            if !matches!(r.kind, RemovalKind::Def | RemovalKind::Section) {
                continue;
            }
            // Only a symbol some hunk actually deletes is reported gone. The
            // set difference alone is not enough: when the caller sends
            // `old` + `diff` rather than `old` + `new` there is no new-side
            // symbol set to compare against, and every untouched definition
            // in the file would read as removed. The deleting hunk is also
            // where the entry belongs in the reading order.
            let Some(at) = f
                .hunks
                .iter()
                .find(|h| h.old_range[0] <= r.row && r.row <= h.old_range[1])
            else {
                continue;
            };
            if !self.seen.insert((f.path.clone(), r.name.clone(), None)) {
                continue;
            }
            let entry = LedgerEntry {
                name: r.name.clone(),
                kind: None,
                scope: None,
                path: f.path.clone(),
                at: at.id.clone(),
                change: SymbolChange::Removed,
                from: None,
                used_by: self.used_by(&r.name, None),
            };
            self.push(&at.id, entry);
        }
    }
}

/// What one file's two sides declare, as the later passes ask about it.
///
/// These were ten `Vec`s pushed in lockstep inside `run`, correct only so long
/// as every iteration pushed to every one of them. Bundled, the compiler holds
/// that invariant instead, and a pass that wants a file's symbols names one
/// thing rather than indexing ten.
/// Where one hunk landed once the ordering ran: its public id, its position in
/// the global reading order, and its rank within its own file. Three
/// `Vec<Vec<_>>` indexed by the same `(file, hunk)` pair, so one value.
#[derive(Clone, Default)]
struct HunkPlace {
    id: String,
    global: usize,
    in_file: usize,
}

/// What happened to the symbol `n`, and where it came from when it did.
fn symbol_change(
    c: &FileChanges,
    symbols: &FileSymbols,
    n: &str,
) -> (SymbolChange, Option<String>) {
    if let Some(src) = c.relocated.get(n) {
        return (SymbolChange::Extracted, Some(src.clone()));
    }
    if let Some(src) = c.moved_in.get(n) {
        return (SymbolChange::Moved, Some(src.clone()));
    }
    if let Some(old) = c.rename.get(n) {
        return (SymbolChange::Renamed, Some(old.clone()));
    }
    let change = if !symbols.old_defs.contains(n) {
        SymbolChange::Added
    } else if c.body_only.contains(n) {
        SymbolChange::Body
    } else {
        SymbolChange::Signature
    };
    (change, None)
}

/// One file's hunks and everything the engine knows per hunk.
///
/// `raw`, `sem`, `comment` and `switched` are strictly parallel — index `li` of
/// each describes the same hunk. They were four separate `Vec<Vec<_>>` in
/// `run`, kept in step only because every loop that pushed to one pushed to all
/// four; here the invariant is that they are built together in one place.
pub(crate) struct PerFileHunks {
    pub(crate) raw: Vec<RawHunk>,
    pub(crate) sem: Vec<HunkSem>,
    /// the file carried a diff but full content could not be recovered, so its
    /// ordering is positional only
    pub(crate) degraded: bool,
    /// every changed line of this hunk is a comment
    pub(crate) comment: Vec<bool>,
    /// how this hunk moved code across the comment boundary, if it did
    pub(crate) switched: Vec<Option<SideShift>>,
    /// hunks this file had that never reached the reading order
    pub(crate) dropped: Vec<DroppedHunk>,
}

/// What this change did to one file's definitions — the per-file half of the
/// answer the narration and the ledger both read. Five `Vec`s, all indexed by
/// the same `fi` and all filled in the same pass, so they are one value.
#[derive(Default)]
pub(crate) struct FileChanges {
    /// new name → the name it had before (#7)
    pub(crate) rename: HashMap<String, String>,
    /// new name → the path it arrived from (P12.1)
    pub(crate) moved_in: HashMap<String, String>,
    /// new name → the def it was extracted out of (P16)
    pub(crate) relocated: HashMap<String, String>,
    /// defs whose signature is unchanged, so a hunk in them is a body edit (#4)
    pub(crate) body_only: HashSet<String>,
    /// everything the file no longer has, as facts (see `Removal`)
    pub(crate) removals: Vec<Removal>,
}

pub(crate) struct FileSymbols {
    /// names the old side defined / imported / bound locally
    pub(crate) old_defs: HashSet<String>,
    pub(crate) old_imports: HashSet<String>,
    pub(crate) old_locals: HashSet<String>,
    /// the same for the new side
    new_defs: HashSet<String>,
    pub(crate) new_imports: HashSet<String>,
    /// name this file binds → (the symbol its own file calls it, the module it
    /// came from). An edge can only be matched on the origin, and only the
    /// module says which file is allowed to answer for it — see
    /// `order::Binding`
    pub(crate) imported_from: HashMap<String, (String, Option<String>)>,
    /// old-side (name, row) for defs and imports — positions, not just names
    pub(crate) old_rows: extract::SymbolRows,
    /// header and body text per definition, for rename and move matching
    pub(crate) old_body: Vec<extract::Body>,
    pub(crate) new_body: Vec<extract::Body>,
    /// file-scope bindings: a removed one is named rather than counted, and a
    /// touched one is what a hunk on a module constant did
    pub(crate) old_binds: Vec<(String, usize)>,
    pub(crate) new_binds: HashSet<String>,
    pub(crate) new_bind_rows: Vec<(String, usize)>,
}

impl FileSymbols {
    fn of(c: &Change) -> FileSymbols {
        let spec = lang::for_path(&c.path);
        let old = c.old.as_deref().zip(spec);
        let new = c.new.as_deref().zip(spec);
        // one descent per side: rows and bodies come back together, where they
        // used to be two walks of the same tree (see `extract::symbol_facts`)
        let (old_rows, old_body) = old.map_or(((vec![], vec![]), vec![]), |(o, sp)| {
            extract::symbol_facts(sp, o)
        });
        let (new_rows, new_body) = new.map_or(((vec![], vec![]), vec![]), |(n, sp)| {
            extract::symbol_facts(sp, n)
        });
        let set = |v: &[(String, usize)]| v.iter().map(|(n, _)| n.clone()).collect();
        let new_bind_rows = new.map_or(vec![], |(n, sp)| extract::top_level_bindings(sp, n));
        let (new_defs, new_imports): (HashSet<String>, HashSet<String>) =
            (set(&new_rows.0), set(&new_rows.1));
        FileSymbols {
            imported_from: new.map_or(HashMap::new(), |(n, sp)| {
                let mut out: HashMap<String, (String, Option<String>)> = HashMap::new();
                for b in extract::import_bindings(sp, n) {
                    let (bound, origin, module) = (b.bound, b.origin, b.module);
                    // the origin is a key too: a use registered under it (see
                    // `order_all`'s `guse`) has to find the same module without
                    // searching, and a name bound directly outranks one that is
                    // only some other alias's origin
                    out.entry(origin.clone())
                        .or_insert_with(|| (origin.clone(), module.clone()));
                    out.insert(bound, (origin, module));
                }
                out
            }),
            old_defs: old_rows.0.iter().map(|(nm, _)| nm.clone()).collect(),
            old_imports: old_rows.1.iter().map(|(nm, _)| nm.clone()).collect(),
            old_locals: old.map_or(HashSet::new(), |(o, sp)| extract::local_names(sp, o)),
            new_defs,
            new_imports,
            old_body,
            new_body,
            old_binds: old.map_or(vec![], |(o, sp)| extract::top_level_bindings(sp, o)),
            new_binds: new_bind_rows.iter().map(|(nm, _)| nm.clone()).collect(),
            new_bind_rows,
            old_rows,
        }
    }
}

/// name → (whole body, its lines), so a pass that asks about many pairs walks
/// the definition list once instead of once per question.
type BodyIndex<'a> = HashMap<&'a str, (&'a str, &'a [String])>;

fn index_bodies(list: &[extract::Body]) -> BodyIndex<'_> {
    list.iter()
        .map(|(nm, _, whole, lines)| (nm.as_str(), (whole.as_str(), lines.as_slice())))
        .collect()
}

/// Two definitions on opposite sides of a change are the same code under a
/// new name when their body lines overlap: every line the shorter of the two
/// has, up to a third of them, has to appear in the other. A one-line body
/// matching one line passes — that is all the evidence a one-liner can offer
/// — while two unrelated definitions share nothing and are rejected.
fn alike(old: &BodyIndex, from: &str, new: &BodyIndex, to: &str) -> bool {
    let (Some((_, o)), Some((_, n))) = (old.get(from), new.get(to)) else {
        return false;
    };
    if o.is_empty() || n.is_empty() {
        return false;
    }
    let os: HashSet<&String> = o.iter().collect();
    let shared = n.iter().filter(|l| os.contains(*l)).count();
    shared > 0 && shared * 3 >= o.len().min(n.len())
}

/// A path with this many hunks is churning rather than being edited.
const HIGH_CHURN: usize = 10;

/// P13.2: what the *changeset* looks like, as facts a reviewer can act on —
/// never judgments. Both signals are decidable from the finished output alone.
///
/// "code" here means any supported language that is neither prose nor a config
/// format. That deliberately includes css and html, so a stylesheet-only change
/// also reports an untouched test suite; tightening it would need a notion of
/// "language people write tests for" that the registry does not have and that
/// nothing has yet asked for.
fn changeset_notes(files: &[FileOut], ledger: &[LedgerEntry]) -> Vec<String> {
    let mut notes = vec![];
    let is_code =
        |p: &str| lang::for_path(p).is_some_and(|s| !s.prose && !s.data && s.template.is_none());
    // a hunk-less file is one the caller sent with nothing in it; it says
    // nothing about whether code changed
    let touched: Vec<&FileOut> = files.iter().filter(|f| !f.hunks.is_empty()).collect();
    let any_code = touched.iter().any(|f| is_code(&f.path));
    let any_test = touched.iter().any(|f| lang::is_test_path(&f.path));
    if any_code && !any_test {
        notes.push("code changed but no test touched".to_string());
    }
    for f in &touched {
        if f.hunks.len() >= HIGH_CHURN {
            notes.push(format!("{}: {} hunks (high churn)", f.path, f.hunks.len()));
        }
    }
    notes.extend(untargeted_test_notes(&touched, ledger));
    notes
}

/// Sharper than "no test touched": a test file *was* touched, but what the
/// change wrote there references none of the definitions the change altered.
/// The failure it catches is a test that exercises something adjacent to the
/// thing that moved.
fn untargeted_test_notes(touched: &[&FileOut], ledger: &[LedgerEntry]) -> Vec<String> {
    let changed: HashSet<&str> = ledger
        .iter()
        .filter(|e| !lang::is_test_path(&e.path))
        .map(|e| e.name.as_str())
        .collect();
    if changed.is_empty() {
        return vec![];
    }
    touched
        .iter()
        .filter(|f| lang::is_test_path(&f.path))
        // what the change wrote in this test file, not what the file already
        // contained — untouched tests are existing coverage
        .filter(|f| {
            !f.hunks
                .iter()
                .flat_map(|h| &h.uses)
                .any(|u| changed.contains(u.as_str()))
        })
        .map(|f| {
            format!(
                "{} touched, but none of its uses reference the {} changed def{}",
                f.path,
                changed.len(),
                if changed.len() == 1 { "" } else { "s" }
            )
        })
        .collect()
}

/// Render the findings as SARIF 2.1.0, so ordo's advisories land in whatever
/// already reads analyzer output — GitHub code scanning, an IDE, a dashboard.
///
/// Deliberately the *findings* only. SARIF describes results at locations; it
/// has no vocabulary for a reading order, a def→use edge or a group, and
/// inventing one in a `properties` bag would produce a file nothing consumes.
/// A caller that wants the ordering reads `Output` (or `pack`).
///
/// A finding is located at its hunk, since that is the resolution ordo works
/// at: the region spans the whole hunk rather than claiming a line the engine
/// never identified.
pub fn sarif(out: &Output) -> String {
    let (rules, results) = sarif_findings(out);
    let notifications = sarif_notifications(out);
    let doc = serde_json::json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": { "driver": {
                "name": "ordo",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": env!("CARGO_PKG_REPOSITORY"),
                "rules": rules,
            }},
            "invocations": [{
                "executionSuccessful": true,
                "toolExecutionNotifications": notifications,
            }],
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
}

/// Each rule once, in first-seen order, and every finding as a result.
fn sarif_findings(out: &Output) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut results = vec![];
    let mut rules = vec![];
    let mut seen: HashSet<&str> = HashSet::new();
    let findings = out.files.iter().flat_map(|file| {
        file.hunks
            .iter()
            .flat_map(move |hunk| hunk.findings.iter().map(move |f| (file, hunk, f)))
    });
    for (file, hunk, f) in findings {
        if seen.insert(&f.name) {
            rules.push(sarif_rule(f));
        }
        results.push(sarif_result(file, hunk, f));
    }
    (rules, results)
}

/// What the engine could not do is part of the report: an empty `results` on
/// a patch that could not be analysed must not read as a clean review.
fn sarif_notifications(out: &Output) -> Vec<serde_json::Value> {
    let problems = out
        .problems
        .iter()
        .map(|p| serde_json::json!({ "level": "warning", "message": { "text": p } }));
    let files = out
        .files
        .iter()
        .filter(|f| f.degraded || f.unsupported)
        .map(|f| {
            let why = if f.unsupported {
                "no grammar for this file type — no structural analysis"
            } else {
                "context-limited diff — positional order only, no semantics"
            };
            serde_json::json!({
                "level": "warning",
                "message": { "text": format!("{}: {why}", f.path) },
                "locations": [{ "physicalLocation": {
                    "artifactLocation": { "uri": f.path },
                }}],
            })
        });
    problems.chain(files).collect()
}

fn sarif_rule(f: &Finding) -> serde_json::Value {
    serde_json::json!({
        "id": f.name,
        // one sentence, as SARIF asks: a catalog message is a numbered remedy
        // list, and a consumer puts this in a rule index beside forty others
        "shortDescription": { "text": first_sentence(&f.message) },
        "fullDescription": { "text": f.message },
        "properties": { "source": f.source.as_str() },
    })
}

fn sarif_result(file: &FileOut, hunk: &HunkOut, f: &Finding) -> serde_json::Value {
    let level = match f.level {
        Level::Note => "note",
        Level::Warn => "warning",
        Level::Verdict => "error",
    };
    serde_json::json!({
        "ruleId": f.name,
        "level": level,
        "message": { "text": f.message },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": file.path },
                "region": {
                    "startLine": hunk.new_range[0],
                    "endLine": hunk.new_range[1],
                },
            },
        }],
        // what the finding is *about*, not where it sits today: a consumer
        // (GitHub code scanning among them) matches an alert across commits
        // on this, and a line number would re-raise everything on the next
        // edit above it
        "partialFingerprints": {
            "ordo/v1": fingerprint(&[
                &f.name,
                &file.path,
                hunk.enclosing.as_deref().unwrap_or(""),
            ]),
        },
        "properties": {
            "source": f.source.as_str(),
            "rationale": hunk.rationale,
        },
    })
}

/// The first sentence of a message, for a field a consumer shows in a list.
fn first_sentence(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.find(". ") {
        Some(i) => line[..=i].trim_end().to_string(),
        None => line.to_string(),
    }
}

/// A stable identity for a finding, as hex. FNV-1a rather than `DefaultHasher`,
/// whose output is explicitly not stable across Rust releases — a fingerprint
/// that changes with the compiler would re-raise every alert on a toolchain
/// bump.
fn fingerprint(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:016x}")
}

/// P12.4: render a compact, deterministic review pack from the engine output —
/// reading order + rationale + independent parts + def→use edges — as LLM-ready
/// context an AI reviewer would otherwise re-derive per run.
pub fn pack(out: &Output) -> String {
    let by_id: ById = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    let total: usize = out.files.iter().map(|f| f.hunks.len()).sum();

    let mut s = String::new();
    let _ = writeln!(
        s,
        "# ordo review pack — {} file(s), {total} hunk(s), {} part(s)",
        out.files.len(),
        out.clusters.len()
    );
    // changeset-level signals lead: they are about the change as a whole, so
    // they frame the reading order rather than sitting after it
    if !out.notes.is_empty() {
        let _ = writeln!(s, "\n## notes");
        for n in &out.notes {
            let _ = writeln!(s, "- {n}");
        }
    }
    pack_ledger(&mut s, out, &by_id);
    pack_order(&mut s, out, &by_id);
    pack_parts(&mut s, out, &by_id);
    pack_edges(&mut s, out, &by_id);
    pack_findings(&mut s, out);
    s
}

fn pack_parts(s: &mut String, out: &Output, by_id: &ById) {
    if out.clusters.len() <= 1 {
        return;
    }
    let _ = writeln!(
        s,
        "\n## independent parts ({}) — candidate PR split",
        out.clusters.len()
    );
    for (i, c) in out.clusters.iter().enumerate() {
        let locs: Vec<String> = c.iter().map(|id| loc(by_id, id)).collect();
        let _ = writeln!(s, "part {}: {}", i + 1, locs.join(", "));
    }
}

fn pack_edges(s: &mut String, out: &Output, by_id: &ById) {
    if out.edges.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## dependencies");
    for e in &out.edges {
        let _ = writeln!(
            s,
            "{} → {}   {}",
            loc(by_id, &e.from),
            loc(by_id, &e.to),
            e.why
        );
    }
}

/// hunk id → (path, hunk)
type ById<'a> = HashMap<&'a str, (&'a str, &'a HunkOut)>;

/// `path:Lrow` for a hunk id, or the id itself when it is unknown
fn loc(by_id: &ById, id: &str) -> String {
    by_id
        .get(id)
        .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
        .unwrap_or_else(|| id.to_string())
}

// the ledger is what the change *did*, one line per symbol; it is read
// before any hunk, so it sits between the changeset notes and the order
fn pack_ledger(s: &mut String, out: &Output, by_id: &ById) {
    if out.ledger.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## ledger — {} symbol(s)", out.ledger.len());
    for e in &out.ledger {
        let change = format!("{:?}", e.change).to_lowercase();
        let from = match (&e.from, e.change) {
            (Some(f), model::SymbolChange::Renamed) => format!(" from {f}"),
            (Some(f), model::SymbolChange::Moved) => format!(" from {f}"),
            (Some(f), model::SymbolChange::Extracted) => format!(" from {f}"),
            _ => String::new(),
        };
        // fan-in is the number a reviewer acts on; the ids are in the JSON
        let fan = match e.used_by.len() {
            0 => String::new(),
            1 => ", used by 1 hunk".to_string(),
            n => format!(", used by {n} hunks"),
        };
        let _ = writeln!(s, "{} {} — {change}{from}{fan}", loc(by_id, &e.at), e.name);
    }
}

fn pack_order(s: &mut String, out: &Output, by_id: &ById) {
    let _ = writeln!(s, "\n## reading order");
    for o in &out.order {
        let Some((path, h)) = by_id.get(o.hunk.as_str()) else {
            continue;
        };
        let cat = format!("{:?}", h.category).to_lowercase();
        let noise = if h.noise { " · noise" } else { "" };
        let notes = if h.notes.is_empty() {
            String::new()
        } else {
            format!(" · {}", h.notes.join("; "))
        };
        let details = if h.details.is_empty() {
            String::new()
        } else {
            format!(" ({})", h.details.join("; "))
        };
        let _ = writeln!(
            s,
            "{path}:L{} [{cat}{noise}] {}{details}{notes}",
            h.new_range[0], h.rationale
        );
    }
}

// everything anyone noticed, whoever noticed it
fn pack_findings(s: &mut String, out: &Output) {
    let found: Vec<(String, &Finding)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks.iter().flat_map(move |h| {
                h.findings
                    .iter()
                    .map(move |a| (format!("{}:L{}", f.path, h.new_range[0]), a))
            })
        })
        .collect();
    if found.is_empty() {
        return;
    }
    let _ = writeln!(s, "\n## findings");
    for (at, a) in found {
        let mark = match a.level {
            Level::Verdict | Level::Warn => " ⚠",
            Level::Note => "",
        };
        let _ = writeln!(s, "{at}  {} ({}){mark}", a.name, a.source.as_str());
        for line in a.message.lines() {
            let _ = writeln!(s, "  {line}");
        }
    }
}

/// Compute hunks + semantics for one change. Full semantics whenever complete
/// new content is available: `new` given, or reconstructed from `old`+`diff`
/// (L1), or a full-context diff (L2 — inside `parse_file_diff`). A partial
/// diff-only change stays positional (L3). Unsupported/unparsable language
/// degrades to "other" semantics (file order preserved).
/// Returns (hunks, semantics, degraded, comment_only). `degraded` = the change
/// carried a diff but full content couldn't be obtained, so ordering is
/// positional only. `comment_only` parallels `hunks`: true when every changed
/// line is a comment (drives the "adds/edits comment" rationale fallback).
type ChangeParts = (
    Vec<RawHunk>,
    Vec<HunkSem>,
    bool,
    Vec<bool>,
    Vec<Option<SideShift>>,
);

// Blank the jinja out of every templated file whose *underlying* format has a
// grammar of its own, so `values.yaml.j2` is analyzed as the yaml it renders
// to. A bare `.j2` (or one over a format with no grammar) is parsed as jinja
// itself and is left alone. `old` is masked too: rename/move/removal matching
// compares the two sides, and one masked side against one raw side would read
// every statement line as a change.
/// What a template's jinja said, once it has been blanked out of the text the
/// grammars see: the variables it reads, and the rows that held the statements.
#[derive(Default)]
struct TemplateFacts {
    uses: Vec<(usize, String)>,
    /// 0-based new-side rows that were blanked
    masked: HashSet<usize>,
}

fn mask_templates(mut input: Input) -> (Input, Vec<TemplateFacts>) {
    let facts = input.changes.iter_mut().map(template_facts).collect();
    (input, facts)
}

/// Blank one change's template statements on both sides, and keep what the
/// new side's statements said before they went.
fn template_facts(c: &mut Change) -> TemplateFacts {
    let Some(tspec) = lang::template_lang(&c.path) else {
        return TemplateFacts::default();
    };
    // a bare `.j2` (or one over a format with no grammar of its own) is
    // parsed as jinja itself — nothing to mask, and the ordinary walk
    // already reads its variables, its macro names and its parameters
    // properly. Harvesting them a second time here would re-add a macro's
    // own name and parameters as uses of themselves.
    if lang::for_path(&c.path).is_some_and(|s| s.name == tspec.name) {
        return TemplateFacts::default();
    }
    // read the jinja *before* blanking it, or the variables this exists
    // to report would already be gone
    let mut f = TemplateFacts {
        uses: c
            .new
            .as_deref()
            .map(|n| extract::template_uses(tspec, n))
            .unwrap_or_default(),
        masked: HashSet::new(),
    };
    let mask = |side: &Option<String>| {
        side.as_deref()
            .and_then(|t| extract::mask_template(tspec, t))
    };
    if let Some((text, _)) = mask(&c.old) {
        c.old = Some(text);
    }
    if let Some((text, rows)) = mask(&c.new) {
        c.new = Some(text);
        f.masked = rows;
    }
    f
}

// A template's `{{ … }}` and `{% … %}` name variables the file consumes; a
// hunk that touches those rows uses them. Uses only — a template defines
// nothing, so `{{ db_host }}` can link to wherever `db_host` is actually set
// without a second template ever claiming to define it.
fn apply_template_facts(f: &TemplateFacts, path: &str, raw: &[RawHunk], sem: &mut [HunkSem]) {
    if f.uses.is_empty() && f.masked.is_empty() {
        return;
    }
    // a blanked row is whitespace to the underlying grammar, so a hunk that
    // touches only jinja — a `{% if %}` guard put around a block, a loop
    // rewritten — would otherwise read as "formatting only" and be dimmed as
    // noise. It is the substance of a template, not its formatting.
    let generated = lang::is_generated_path(path);
    for (h, s) in raw.iter().zip(sem.iter_mut()) {
        let Some(r0) = h.new_r0 else { continue };
        let rows = r0..=h.new_r1;
        for (_, name) in f.uses.iter().filter(|(row, _)| rows.contains(row)) {
            if !s.uses.contains(name) {
                s.uses.push(name.clone());
            }
        }
        if !generated && rows.clone().any(|r| f.masked.contains(&r)) {
            s.noise = false;
        }
    }
}

fn build_change(change: &Change, full_context: bool) -> ChangeParts {
    let old = change.old.as_deref();
    let (raw, new, degraded): (Vec<RawHunk>, String, bool) = if let Some(new) = &change.new {
        (compute_hunks(old.unwrap_or(""), new), new.clone(), false)
    } else if let Some(diff) = &change.diff {
        // `fill_sides` rebuilds both sides whenever the diff allows it, so a
        // change still carrying only a diff here is one it could not rebuild:
        // the hunks are the `@@` ranges and the order is positional (L3)
        (
            patch::parse_file_diff(diff, full_context).hunks,
            String::new(),
            true,
        )
    } else {
        (vec![], String::new(), false)
    };
    // `analyze` hands back the hunks it read: an added file arrives as one hunk
    // however long it is, and comes back cut at the constructs inside it
    let (raw, mut sems) = match lang::for_path(&change.path)
        .and_then(|spec| analyze(spec, &new, &raw, &change.path))
    {
        Some(pair) => pair,
        None => {
            let sems = raw.iter().map(HunkSem::other).collect();
            (raw, sems)
        }
    };
    // P12.2 noise: generated/vendored path, or a formatting-only hunk
    let generated = lang::is_generated_path(&change.path);
    let old_lines: Vec<&str> = old.unwrap_or("").lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    mark_shape(
        &raw,
        &mut sems,
        (&old_lines, &new_lines),
        generated,
        degraded,
    );
    let ext = change.path.rsplit('.').next().unwrap_or("");
    let spec = lang::for_path(&change.path);
    let old_doc = old.and_then(|o| spec.and_then(|s| ts_comment_lines(s, o)));
    let new_doc = spec.and_then(|s| ts_comment_lines(s, &new));
    let comment_only: Vec<bool> = raw
        .iter()
        .map(|h| {
            comment_only_hunk(
                h,
                &old_lines,
                &new_lines,
                ext,
                old_doc.as_ref(),
                new_doc.as_ref(),
            )
        })
        .collect();
    if let Some(spec) = lang::for_path(&change.path) {
        fill_details(&raw, &mut sems, old, spec);
    }
    let switched: Vec<Option<SideShift>> = raw
        .iter()
        .map(|h| side_shift(h, &old_lines, &new_lines, ext))
        .collect();
    (raw, sems, degraded, comment_only, switched)
}

/// Two facts a hunk carries about its own shape rather than its semantics:
/// whether it is skippable, and whether it covers the old file entire.
///
/// A degraded file has no sides to compare — every hunk looked identical on
/// both of them, and the whole file was dimmed as "formatting only", which is
/// the one thing it certainly is not.
fn mark_shape(
    raw: &[RawHunk],
    sems: &mut [HunkSem],
    (old_lines, new_lines): (&[&str], &[&str]),
    generated: bool,
    degraded: bool,
) {
    for (h, sem) in raw.iter().zip(sems) {
        sem.noise = generated || (!degraded && formatting_only(h, old_lines, new_lines));
        let [o0, o1] = h.old_range;
        sem.whole_old_file = !old_lines.is_empty() && o0 == 1 && o1 >= old_lines.len();
    }
}

/// P15 detail layer: what the hunk did to the members of its container. The
/// container name is already on the hunk as `enclosing`; only the old side's
/// members need a second parse.
fn fill_details(raw: &[RawHunk], sems: &mut [HunkSem], old: Option<&str>, spec: &lang::LangSpec) {
    let old_members = old
        .map(|o| extract::member_rows(spec, o))
        .unwrap_or_default();
    let phrases: Vec<Vec<String>> = raw
        .iter()
        .zip(sems.iter())
        .map(|(h, sem)| {
            order::detail_phrases(sem, h, &old_members, spec.prose, spec.prose || spec.data)
        })
        .collect();
    for (sem, d) in sems.iter_mut().zip(phrases) {
        sem.details = d;
    }
}

// A hunk whose changed lines are all comments: every non-blank line on
// whichever side(s) are present is a comment line — either by
// extension-specific textual syntax (`#`, `//`, …) or, when the language has
// a grammar, by falling inside a tree-sitter comment node or a docstring
// (`old_doc`/`new_doc`, see `ts_comment_lines`). The textual check alone
// can't tell a real comment from prose that merely looks like one (e.g. help
// text inside a raw string literal), and it's line-based so it can't see a
// multi-line docstring's interior prose lines at all — the tree-sitter sets
// cover both. Where no grammar is available (or parsing fails) `old_doc`/
// `new_doc` are `None` and behavior is exactly the prior textual-only check.
fn comment_only_hunk(
    h: &RawHunk,
    old_lines: &[&str],
    new_lines: &[&str],
    ext: &str,
    old_doc: Option<&HashSet<usize>>,
    new_doc: Option<&HashSet<usize>>,
) -> bool {
    let side = |lines: &[&str], r: [usize; 2], doc: Option<&HashSet<usize>>| -> Option<bool> {
        if r[0] == 0 || r[0] > r[1] || r[1] > lines.len() {
            return None; // empty side (pure insert/delete)
        }
        let seg = &lines[r[0] - 1..r[1]];
        if seg.iter().all(|l| l.trim().is_empty()) {
            return None; // nothing but blank lines: not a meaningful side
        }
        let comment = |(line_no, l): (usize, &&str)| {
            // With a parsed grammar the doc set is the authority on strings:
            // a lone `"""` closing a `patch = """…"""` assignment is code,
            // and only the textual check ever called it a comment.
            let textual = is_comment_line(l.trim(), ext)
                && !(doc.is_some() && l.trim().starts_with(['"', '\'']));
            textual || doc.is_some_and(|d| d.contains(&line_no))
        };
        Some((r[0]..=r[1]).zip(seg.iter()).all(comment))
    };
    match (
        side(new_lines, h.new_range, new_doc),
        side(old_lines, h.old_range, old_doc),
    ) {
        (Some(n), Some(o)) => n && o,
        (Some(n), None) => n,
        (None, Some(o)) => o,
        (None, None) => false,
    }
}

// Tree-sitter line coverage for comment-like content: every 1-based line
// fully inside a grammar comment node (`comment`, `line_comment`,
// `block_comment`, `doc_comment`, … — every comment kind across the
// supported grammars contains "comment" in its node kind), plus, for python,
// a docstring: an `expression_statement` whose sole child is a `string`, as
// the first statement of a module/class/function body, or — the Sphinx/attrs
// "attribute docstring" convention — immediately after the assignment it
// documents within that same body. Doc comments in other languages (rust
// `///`/`//!`, java/js `/** */`) are already grammar `comment` nodes, so
// they're covered by the first check without special casing. Returns `None`
// when the source doesn't parse.
fn ts_comment_lines(spec: &LangSpec, src: &str) -> Option<HashSet<usize>> {
    let tree = lang::parse(spec, src)?;
    let mut out = HashSet::new();
    collect_comment_lines(tree.root_node(), spec.name, &mut out);
    Some(out)
}

// an `expression_statement` whose sole child is a `string` — a bare string
// literal used as a statement, python's docstring shape.
fn is_bare_string_stmt(node: Node) -> bool {
    node.kind() == "expression_statement"
        && node.named_child_count() == 1
        && node.named_child(0).is_some_and(|n| n.kind() == "string")
}

fn collect_comment_lines(node: Node, lang_name: &str, out: &mut HashSet<usize>) {
    let kind = node.kind();
    if kind.contains("comment") {
        for row in node.start_position().row..=node.end_position().row {
            out.insert(row + 1);
        }
        return; // no need to recurse inside a comment node
    }
    if matches!(lang_name, "python" | "xonsh") {
        let is_doc_container = kind == "module"
            || (kind == "block"
                && node.parent().is_some_and(|p| {
                    matches!(p.kind(), "function_definition" | "class_definition")
                }));
        if is_doc_container {
            python_docstring_rows(node, out);
        }
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        collect_comment_lines(child, lang_name, out);
    }
}

// the module/class/function's first statement is always a docstring
// candidate; any later one only counts as the "attribute docstring"
// convention (Sphinx/attrs) — a bare string immediately after the assignment
// it documents. A string elsewhere (after a `for`/`if`/`return`/…) is data or
// dead code, not a comment, so it's left alone.
fn python_docstring_rows(container: Node, out: &mut HashSet<usize>) {
    let mut cur = container.walk();
    let mut prev: Option<Node> = None;
    for stmt in container.named_children(&mut cur) {
        let is_attr_doc_site = prev.is_some_and(|p| {
            p.kind() == "expression_statement"
                && p.named_child(0).is_some_and(|a| a.kind() == "assignment")
        });
        if (prev.is_none() || is_attr_doc_site) && is_bare_string_stmt(stmt) {
            out.extend((stmt.start_position().row..=stmt.end_position().row).map(|r| r + 1));
        }
        prev = Some(stmt);
    }
}

/// The comment markers of one file extension, longest first so stripping takes
/// the specific form (`///`, `---`) before the general one it starts with.
///
/// One table for both `is_comment_line` and `strip_comment_marker`: a marker
/// that opened a comment for one and not the other made `CommentedOut` degrade
/// to `CodeToComment` on every xonsh file. `#` is not listed for the C-family
/// default — a preprocessor directive also starts with `#` and is not a comment.
fn comment_markers(ext: &str) -> &'static [&'static str] {
    match ext {
        "py" | "pyi" | "xsh" | "xonsh" | "xonshrc" => &["\"\"\"", "'''", "#"],
        "lua" => &["---", "--"],
        // `#`-comment formats the C-family default would otherwise misread
        "sh" | "bash" | "zsh" | "rb" | "yaml" | "yml" | "toml" | "j2" | "jinja" | "jinja2"
        | "tf" | "tfvars" | "conf" | "pl" | "r" | "jl" | "nix" | "mk" | "dockerfile"
        | "gitignore" | "gitattributes" => &["#"],
        // ini accepts both spellings
        "ini" | "cfg" => &["#", ";"],
        _ => &["///", "//", "/*", "*"],
    }
}

// Whether a line opens (or continues) a comment in a file of this extension.
fn is_comment_line(trimmed: &str, ext: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    // an interpreter line is a comment to every grammar that allows one, and
    // reads as one in a diff; without this it classifies as nothing at all
    if trimmed.starts_with("#!") {
        return true;
    }
    comment_markers(ext)
        .iter()
        .any(|m| trimmed.starts_with(m) && marker_fits(trimmed, m))
}

/// `*` opens a block comment's continuation line, and it also opens a
/// dereference or a wrapped multiplication: `*limitedAlphal/(1 + …)` is code,
/// and reading it as a comment reported a formula rewrite as "replaces 2
/// lines with comments". Only the spelling with a separator after it counts.
fn marker_fits(trimmed: &str, marker: &str) -> bool {
    if marker != "*" {
        return true;
    }
    matches!(
        trimmed.as_bytes().get(1),
        None | Some(b' ' | b'\t' | b'/' | b'*')
    )
}

/// How a hunk moved code across the comment boundary.
///
/// A hunk whose new side is the old side with comment markers added is not an
/// edit and not a comment change: it is code being switched off, which is a
/// thing a reviewer specifically looks for. The test is exact — strip the
/// markers and the two sides must match once whitespace is normalised — so a
/// hunk that replaces code with *unrelated* prose is reported as what it is
/// instead: documentation arriving where code left.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SideShift {
    /// the same code, now commented out
    CommentedOut,
    /// the same code, no longer commented out
    Uncommented,
    /// comments in place of code that is simply gone
    CodeToComment,
}

fn side_shift(h: &RawHunk, old_lines: &[&str], new_lines: &[&str], ext: &str) -> Option<SideShift> {
    let side = |lines: &[&str], r: [usize; 2]| -> Option<Vec<String>> {
        if r[0] == 0 || r[0] > r[1] || r[1] > lines.len() {
            return None;
        }
        let v: Vec<String> = lines[r[0] - 1..r[1]]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(extract::squeeze)
            .collect();
        (!v.is_empty()).then_some(v)
    };
    let (old, new) = (side(old_lines, h.old_range)?, side(new_lines, h.new_range)?);
    let all_comment = |v: &[String]| v.iter().all(|l| is_comment_line(l, ext));
    let stripped = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|l| {
                strip_comment_marker(l, ext)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|l| !l.is_empty())
            .collect()
    };
    match (all_comment(&old), all_comment(&new)) {
        (false, true) if stripped(&new) == old => Some(SideShift::CommentedOut),
        (false, true) => Some(SideShift::CodeToComment),
        (true, false) if stripped(&old) == new => Some(SideShift::Uncommented),
        _ => None,
    }
}

/// The text of a comment line without its marker. Reads the same table
/// `is_comment_line` does, so the two cannot drift apart.
fn strip_comment_marker<'a>(line: &'a str, ext: &str) -> &'a str {
    let t = line.trim();
    let out = comment_markers(ext)
        .iter()
        .find_map(|m| t.strip_prefix(m))
        .unwrap_or(t);
    out.trim_end_matches("*/")
        .trim_end_matches("\"\"\"")
        .trim_end_matches("'''")
        .trim()
}

// A hunk that changes only whitespace/layout: both sides present and equal once
// whitespace is normalized away.
fn formatting_only(h: &RawHunk, old_lines: &[&str], new_lines: &[&str]) -> bool {
    let slice = |lines: &[&str], r: [usize; 2]| -> Option<String> {
        if r[0] == 0 || r[0] > r[1] || r[1] > lines.len() {
            return None; // empty side (pure insert/delete) or out of range
        }
        Some(
            lines[r[0] - 1..r[1]]
                .join("\n")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        )
    };
    let (o, n) = (slice(old_lines, h.old_range), slice(new_lines, h.new_range));
    match (&o, &n) {
        // same text once whitespace is normalised: a reindent or a rewrap
        (Some(o), Some(n)) if !o.is_empty() && o == n => true,
        // nothing but whitespace on either side — blank lines added or removed.
        // `None` is an empty side (a pure insert or delete), which is itself
        // "no text", so a blank-line-only hunk lands here rather than reading
        // as a change with nothing to say about it.
        _ => o.unwrap_or_default().is_empty() && n.unwrap_or_default().is_empty(),
    }
}
