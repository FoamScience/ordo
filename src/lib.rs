//! ordo — comprehension-optimized ordering of code-change hunks.
//! Public entry: [`run`] takes an [`Input`] and returns the v1 [`Output`].
mod advisories;
mod extract;
mod lang;
pub mod model;
mod order;
mod patch;
pub mod refine;
mod rules;

use extract::{analyze, compute_hunks, symbol_sets, HunkSem, RawHunk};
use lang::LangSpec;
use model::*;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

pub use lang::is_generated_path;
pub use patch::split_patch;

pub const SCHEMA_VERSION: u32 = 1;

pub fn run(input: Input) -> Output {
    // per-file hunks + semantics
    let mut raws: Vec<Vec<RawHunk>> = vec![];
    let mut sems: Vec<Vec<HunkSem>> = vec![];
    let mut degraded: Vec<bool> = vec![];
    let mut comment_only: Vec<Vec<bool>> = vec![];
    let mut switched: Vec<Vec<Option<SideShift>>> = vec![];
    let mut dropped: Vec<Vec<DroppedHunk>> = vec![];
    for change in &input.changes {
        let (raw, sem, deg, com, sw) = build_change(change, input.options.full_context);
        if deg {
            eprintln!(
                "ordo: {}: diff lacks full context — positional order only (use old/new, or pass a full-context patch with `full_context`/`--full-context`)",
                change.path
            );
        }
        // A pure-import hunk is *noise*, not nothing: it follows from the real
        // change rather than being it, so it never leads the reading order and
        // never seeds a def→use edge — but it stays visible, dimmed, where the
        // diff put it. Dropping it outright made ordo asymmetric in a way
        // reviewers noticed: a removed import was reported ("removes import
        // loguru", from the deletion path) while an added one vanished, so a
        // moved import read as a deletion with no counterpart.
        //
        // `only_comments` still drops, because there the caller asked for a
        // subset; the drop is recorded with its range so `hunks + dropped`
        // accounts for every hunk the diff produced.
        let mut raw_kept = vec![];
        let mut sem_kept = vec![];
        let mut com_kept = vec![];
        let mut gone = vec![];
        let mut sw_kept = vec![];
        for (((r, mut s), c), w) in raw.into_iter().zip(sem).zip(com).zip(sw) {
            if s.category == Category::Import {
                s.noise = true;
            }
            let reason = if input.options.only_comments && !c {
                Some(DropReason::NonComment)
            } else {
                None
            };
            match reason {
                Some(reason) => gone.push(DroppedHunk {
                    reason,
                    old_range: r.old_range,
                    new_range: r.new_range,
                }),
                None => {
                    raw_kept.push(r);
                    sem_kept.push(s);
                    com_kept.push(c);
                    sw_kept.push(w);
                }
            }
        }
        let (raw, sem, com) = (raw_kept, sem_kept, com_kept);
        raws.push(raw);
        sems.push(sem);
        degraded.push(deg);
        comment_only.push(com);
        switched.push(sw_kept);
        dropped.push(gone);
    }

    let paths: Vec<String> = input.changes.iter().map(|c| c.path.clone()).collect();
    let n = input.changes.len();
    // per-file old/new symbol data (rows for positions, sets for membership,
    // bodies for rename/move matching) — drives #3/#5/#7 and P12.1 moves.
    let body_of = |list: &[extract::Body], name: &str| {
        list.iter()
            .find(|(nm, _, _, _)| nm == name)
            .map(|(_, _, b, _)| b.clone())
    };
    let header_of = |list: &[extract::Body], name: &str| {
        list.iter()
            .find(|(nm, _, _, _)| nm == name)
            .map(|(_, h, _, _)| h.clone())
    };
    let mut old_defs: Vec<HashSet<String>> = vec![];
    let mut old_imports: Vec<HashSet<String>> = vec![];
    let mut old_locals: Vec<HashSet<String>> = vec![];
    let mut new_defs_v: Vec<HashSet<String>> = vec![];
    let mut new_imports_v: Vec<HashSet<String>> = vec![];
    let mut old_rows: Vec<extract::SymbolRows> = vec![];
    let mut old_body: Vec<Vec<extract::Body>> = vec![];
    let mut new_body: Vec<Vec<extract::Body>> = vec![];
    // file-scope bindings per side: a removed one is named rather than counted
    let mut old_binds: Vec<Vec<(String, usize)>> = vec![];
    let mut new_binds: Vec<HashSet<String>> = vec![];
    for c in &input.changes {
        let spec = lang::for_path(&c.path);
        let rows = match (c.old.as_deref(), spec) {
            (Some(old), Some(sp)) => extract::symbol_rows(sp, old),
            _ => (vec![], vec![]),
        };
        let (nd, ni) = match (c.new.as_deref(), spec) {
            (Some(new), Some(sp)) => symbol_sets(sp, new),
            _ => (HashSet::new(), HashSet::new()),
        };
        let ob = spec
            .zip(c.old.as_deref())
            .map(|(sp, o)| extract::symbol_bodies(sp, o))
            .unwrap_or_default();
        let nb = spec
            .zip(c.new.as_deref())
            .map(|(sp, nw)| extract::symbol_bodies(sp, nw))
            .unwrap_or_default();
        old_defs.push(rows.0.iter().map(|(nm, _)| nm.clone()).collect());
        old_imports.push(rows.1.iter().map(|(nm, _)| nm.clone()).collect());
        old_locals.push(match (c.old.as_deref(), spec) {
            (Some(old), Some(sp)) => extract::local_names(sp, old),
            _ => HashSet::new(),
        });
        new_defs_v.push(nd);
        new_imports_v.push(ni);
        old_rows.push(rows);
        old_body.push(ob);
        new_body.push(nb);
        old_binds.push(match (c.old.as_deref(), spec) {
            (Some(old), Some(sp)) => extract::top_level_bindings(sp, old),
            _ => vec![],
        });
        new_binds.push(match (c.new.as_deref(), spec) {
            (Some(nw), Some(sp)) => extract::top_level_bindings(sp, nw)
                .into_iter()
                .map(|(n, _)| n)
                .collect(),
            _ => HashSet::new(),
        });
    }
    // P12.1: index freshly-appeared new defs by (name, body) → file, for moves
    let mut appeared: HashMap<(String, String), usize> = HashMap::new();
    for (fi, nb) in new_body.iter().enumerate() {
        for (name, _, body, _) in nb {
            if body.len() >= 8 && !old_defs[fi].contains(name) {
                appeared.entry((name.clone(), body.clone())).or_insert(fi);
            }
        }
    }

    let mut rename: Vec<HashMap<String, String>> = vec![HashMap::new(); n];
    let mut moved_in: Vec<HashMap<String, String>> = vec![HashMap::new(); n]; // new name → source path
    let mut relocated: Vec<HashMap<String, String>> = vec![HashMap::new(); n]; // new name → old def it was extracted from
    let mut body_only: Vec<HashSet<String>> = vec![HashSet::new(); n]; // existing def, unchanged signature → body-only edit
    let mut removals: Vec<Vec<(usize, String)>> = vec![vec![]; n];
    for fi in 0..n {
        let (nd, ni) = (&new_defs_v[fi], &new_imports_v[fi]);
        let od = &old_defs[fi];
        let (odr, oir) = &old_rows[fi];
        let (ob, nb) = (&old_body[fi], &new_body[fi]);
        let mut removed_d: Vec<String> = od.difference(nd).cloned().collect();
        let mut added_d: Vec<String> = nd.difference(od).cloned().collect();
        removed_d.sort();
        added_d.sort();

        // #7 rename (same file): body match, then lone-pair fallback
        let mut ren = HashMap::new();
        if !removed_d.is_empty() && !added_d.is_empty() {
            for r in &removed_d {
                let rb = match body_of(ob, r) {
                    Some(b) if b.len() >= 8 => b,
                    _ => continue,
                };
                if let Some(a) = added_d.iter().find(|a| {
                    !ren.contains_key(*a) && body_of(nb, a).as_deref() == Some(rb.as_str())
                }) {
                    ren.insert(a.clone(), r.clone());
                }
            }
            let rem_left: Vec<&String> = removed_d
                .iter()
                .filter(|r| !ren.values().any(|v| v == *r))
                .collect();
            let add_left: Vec<&String> = added_d.iter().filter(|a| !ren.contains_key(*a)).collect();
            if rem_left.len() == 1 && add_left.len() == 1 {
                ren.insert(add_left[0].clone(), rem_left[0].clone());
            }
        }
        let renamed_old: HashSet<String> = ren.values().cloned().collect();

        // P12.1 moves: a removed (non-renamed) def whose body reappears same-name in another file
        let mut moved_out: HashMap<String, usize> = HashMap::new();
        for r in &removed_d {
            if renamed_old.contains(r) {
                continue;
            }
            let rb = match body_of(ob, r) {
                Some(b) if b.len() >= 8 => b,
                _ => continue,
            };
            if let Some(&tgt) = appeared.get(&(r.clone(), rb)) {
                if tgt != fi {
                    moved_out.insert(r.clone(), tgt);
                    moved_in[tgt].insert(r.clone(), paths[fi].clone());
                }
            }
        }

        // P16 relocation: an added def whose body-lines overlap a still-present
        // old def → it was extracted/relocated out of that def (body may differ).
        let mut reloc = HashMap::new();
        for a in &added_d {
            if ren.contains_key(a) {
                continue;
            }
            let al = match nb
                .iter()
                .find(|(nm, _, _, _)| nm == a)
                .map(|(_, _, _, l)| l)
            {
                Some(l) if l.len() >= 3 => l,
                _ => continue,
            };
            let aset: HashSet<&String> = al.iter().collect();
            let mut best: Option<(&String, usize)> = None;
            for (x, _, _, xl) in ob {
                if x == a || !nd.contains(x) {
                    continue;
                }
                let shared = xl.iter().filter(|l| aset.contains(*l)).count();
                if shared >= 3 && shared * 2 >= al.len() && best.is_none_or(|(_, s)| shared > s) {
                    best = Some((x, shared));
                }
            }
            if let Some((x, _)) = best {
                reloc.insert(a.clone(), x.clone());
            }
        }
        relocated[fi] = reloc;

        // #4: an existing def whose signature is unchanged → body-only edit, not
        // a signature change. Positive-only: unknown headers keep "changes signature of".
        for name in od.intersection(nd) {
            match (header_of(ob, name), header_of(nb, name)) {
                (Some(o), Some(m)) if o == m => {
                    body_only[fi].insert(name.clone());
                }
                _ => {}
            }
        }

        // #7 delete / #5 import remove / P12.1 move-out (else "removes")
        let mut rem = vec![];
        for (name, row) in odr {
            if nd.contains(name) || renamed_old.contains(name) {
                continue;
            }
            let prose = lang::for_path(&paths[fi]).is_some_and(|s| s.prose);
            match moved_out.get(name) {
                Some(&tgt) => rem.push((*row, format!("moves {name} to {}", paths[tgt]))),
                None if prose => rem.push((*row, format!("removes section {name}"))),
                None => rem.push((*row, format!("removes {name}"))),
            }
        }
        for (name, row) in oir {
            if !ni.contains(name) {
                rem.push((*row, format!("removes import {name}")));
            }
        }
        // a removed file-scope binding: not a definition, but naming it beats
        // the "removes N lines" fallback a module constant would get otherwise
        for (name, row) in &old_binds[fi] {
            if !new_binds[fi].contains(name) && !nd.contains(name) && !od.contains(name) {
                rem.push((*row, format!("removes {name}")));
            }
        }
        rename[fi] = ren;
        removals[fi] = rem;
    }
    // A hunk that only deletes has no new side to classify from, so a removed
    // import used to read as a plain `other` hunk while an added one was an
    // import. Classify a pure deletion from the side it actually has.
    for fi in 0..n {
        let (Some(spec), Some(old_src)) =
            (lang::for_path(&paths[fi]), input.changes[fi].old.as_deref())
        else {
            continue;
        };
        let rows = extract::import_row_set(spec, old_src);
        if rows.is_empty() {
            continue;
        }
        for (li, sem) in sems[fi].iter_mut().enumerate() {
            let [o0, o1] = sem.old_range;
            let [n0, n1] = raws[fi][li].new_range;
            let deletes_only = n0 > n1;
            if deletes_only && o0 >= 1 && o0 <= o1 && (o0..=o1).all(|r| rows.contains(&r)) {
                sem.category = Category::Import;
                sem.noise = true;
            }
        }
    }

    // A pure-import hunk whose statements all existed in the old file is a
    // reordering, not an arrival: "moves import pg" rather than "changes".
    for fi in 0..n {
        let (Some(spec), Some(old_src)) =
            (lang::for_path(&paths[fi]), input.changes[fi].old.as_deref())
        else {
            continue;
        };
        let Some(new_src) = input.changes[fi].new.as_deref() else {
            continue;
        };
        let before = extract::import_statements(spec, old_src);
        if before.is_empty() {
            continue;
        }
        let new_lines: Vec<&str> = new_src.lines().collect();
        for (li, sem) in sems[fi].iter_mut().enumerate() {
            if sem.category != Category::Import {
                continue;
            }
            let [r0, r1] = raws[fi][li].new_range;
            if r0 == 0 || r0 > r1 {
                continue;
            }
            // every non-blank line of the hunk has to be an import the old file
            // already had; one new line among them makes this an arrival
            let mut lines = (r0..=r1)
                .filter_map(|r| new_lines.get(r - 1))
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|l| !l.is_empty())
                .peekable();
            sem.import_moved = lines.peek().is_some() && lines.all(|l| before.contains(&l));
        }
    }

    // An import line that moved, leaving a blank line behind, reads as a bare
    // change: the new side carries no rows to classify by, and the old side is
    // where the meaning was. ordo already treats pure-import hunks as
    // bookkeeping, so the residue of reordering them is formatting. A genuinely
    // deleted import is excluded below — that one is named.
    for fi in 0..n {
        let Some(new) = input.changes[fi].new.as_deref() else {
            continue;
        };
        let new_lines: Vec<&str> = new.lines().collect();
        let import_rows: HashSet<usize> = old_rows[fi].1.iter().map(|(_, r)| *r).collect();
        for (li, sem) in sems[fi].iter_mut().enumerate() {
            let [o0, o1] = sem.old_range;
            if sem.noise || o0 == 0 || o0 > o1 {
                continue;
            }
            let [n0, n1] = raws[fi][li].new_range;
            let new_blank = n0 > n1
                || (n0..=n1).all(|r| new_lines.get(r - 1).is_some_and(|l| l.trim().is_empty()));
            // a *deleted* import is a real removal and is named as such; this
            // is only the residue of one that moved, where nothing was removed
            let named = removals[fi].iter().any(|(r, _)| *r >= o0 && *r <= o1);
            if new_blank && !named && (o0..=o1).all(|r| import_rows.contains(&r)) {
                sem.noise = true;
            }
        }
    }

    // ---- reviewing rules (Options.rules) ----
    // Evaluated after the semantics they match on, and before the ordering they
    // can influence. A rule's `noise` and `priority` reach the hunk itself; its
    // notes ride along to the output.
    let mut rule_engine = rules::Rules::new(&input.options.rules);
    let mut rule_hits: Vec<Vec<Vec<RuleHit>>> = vec![vec![]; n];
    if !rule_engine.is_empty() {
        for (fi, change) in input.changes.iter().enumerate() {
            let path = &change.path;
            let query_rows = match (lang::for_path(path), change.new.as_deref()) {
                (Some(spec), Some(new)) => rule_engine.query_rows(spec, new),
                _ => HashMap::new(),
            };
            let mut per_file = vec![];
            for li in 0..raws[fi].len() {
                let sem = &sems[fi][li];
                let [r0, r1] = raws[fi][li].new_range;
                let hits = rule_engine.hits(
                    path,
                    (r0, r1),
                    sem.category,
                    sem.enclosing_kind,
                    &sem.defines,
                    &sem.uses,
                    &sem.imports,
                    sem.noise,
                    comment_only[fi][li],
                    &query_rows,
                );
                per_file.push(hits);
            }
            rule_hits[fi] = per_file;
        }
        rule_engine.finish();
        for fi in 0..n {
            for (li, hits) in rule_hits[fi].iter().enumerate() {
                if rules::Rules::any_noise(hits, &input.options.rules) {
                    sems[fi][li].noise = true;
                }
                sems[fi][li].priority = rules::Rules::priority(hits, &input.options.rules);
            }
        }
    }

    let facts = order::FileFacts {
        old_defs: &old_defs,
        old_imports: &old_imports,
        old_locals: &old_locals,
        rename: &rename,
        moved_in: &moved_in,
        relocated: &relocated,
        body_only: &body_only,
        removals: &removals,
        comment_only: &comment_only,
        switched: &switched,
    };
    let ordered = order::order_all(
        &sems,
        &paths,
        &facts,
        input.options.strategy,
        input.options.cross_file,
    );

    // global hunk id per (file, local) and reverse map to global index
    let mut hid_of: Vec<Vec<String>> = raws.iter().map(|r| vec![String::new(); r.len()]).collect();
    let mut gidx_of: Vec<Vec<usize>> = raws.iter().map(|r| vec![0usize; r.len()]).collect();
    for (i, &(fi, li)) in ordered.coord.iter().enumerate() {
        hid_of[fi][li] = format!("h{i}");
        gidx_of[fi][li] = i;
    }
    let gids: Vec<String> = (0..ordered.groups.len())
        .map(|gi| format!("g{gi}"))
        .collect();

    // per-file order_index = rank within that file across the global reading order
    let mut order_index: Vec<Vec<usize>> = raws.iter().map(|r| vec![0usize; r.len()]).collect();
    let mut file_counter = vec![0usize; raws.len()];
    for &i in &ordered.perm {
        let (fi, li) = ordered.coord[i];
        order_index[fi][li] = file_counter[fi];
        file_counter[fi] += 1;
    }

    // global reading order
    let order: Vec<OrderItem> = ordered
        .perm
        .iter()
        .map(|&i| {
            let (fi, li) = ordered.coord[i];
            OrderItem {
                path: input.changes[fi].path.clone(),
                hunk: hid_of[fi][li].clone(),
            }
        })
        .collect();

    // per-file hunk metadata
    let mut files = vec![];
    for (fi, change) in input.changes.iter().enumerate() {
        let mut hunks = vec![];
        for li in 0..raws[fi].len() {
            let gi = gidx_of[fi][li];
            hunks.push(HunkOut {
                id: hid_of[fi][li].clone(),
                old_range: raws[fi][li].old_range,
                new_range: raws[fi][li].new_range,
                category: sems[fi][li].category,
                enclosing: sems[fi][li].enclosing.clone(),
                enclosing_kind: sems[fi][li].enclosing_kind,
                defines: sems[fi][li].defines.clone(),
                uses: sems[fi][li].uses.clone(),
                group: gids[ordered.group_idx[gi]].clone(),
                order_index: order_index[fi][li],
                rationale: ordered.rationale[gi].clone(),
                noise: sems[fi][li].noise,
                comment: comment_only[fi][li],
                details: sems[fi][li].details.clone(),
                notes: sems[fi][li].notes.clone(),
                advisories: sems[fi][li].advisories.clone(),
                symbols: sems[fi][li].symbols.clone(),
                rules: rule_hits[fi].get(li).cloned().unwrap_or_default(),
            });
        }
        files.push(FileOut {
            path: change.path.clone(),
            hunks,
            degraded: degraded[fi],
            unsupported: lang::for_path(&change.path).is_none(),
            dropped: std::mem::take(&mut dropped[fi]),
        });
    }

    let hid = |gi: usize| {
        let (fi, li) = ordered.coord[gi];
        hid_of[fi][li].clone()
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

    Output {
        schema: SCHEMA_VERSION,
        order,
        files,
        groups: groups_out,
        edges: edges_out,
        clusters,
        problems: {
            // one file kind repeats across a changeset; a broken rule should
            // be reported once, not once per file
            let mut p = rule_engine.problems.clone();
            p.dedup();
            p
        },
    }
}

/// P12.4: render a compact, deterministic review pack from the engine output —
/// reading order + rationale + independent parts + def→use edges — as LLM-ready
/// context an AI reviewer would otherwise re-derive per run.
pub fn pack(out: &Output) -> String {
    use std::fmt::Write;
    let by_id: HashMap<&str, (&str, &HunkOut)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks
                .iter()
                .map(move |h| (h.id.as_str(), (f.path.as_str(), h)))
        })
        .collect();
    let loc = |id: &str| {
        by_id
            .get(id)
            .map(|(p, h)| format!("{p}:L{}", h.new_range[0]))
            .unwrap_or_else(|| id.to_string())
    };
    let total: usize = out.files.iter().map(|f| f.hunks.len()).sum();

    let mut s = String::new();
    let _ = writeln!(
        s,
        "# ordo review pack — {} file(s), {total} hunk(s), {} part(s)",
        out.files.len(),
        out.clusters.len()
    );
    let _ = writeln!(s, "\n## reading order");
    for o in &out.order {
        if let Some((path, h)) = by_id.get(o.hunk.as_str()) {
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
    if out.clusters.len() > 1 {
        let _ = writeln!(
            s,
            "\n## independent parts ({}) — candidate PR split",
            out.clusters.len()
        );
        for (i, c) in out.clusters.iter().enumerate() {
            let locs: Vec<String> = c.iter().map(|id| loc(id)).collect();
            let _ = writeln!(s, "part {}: {}", i + 1, locs.join(", "));
        }
    }
    if !out.edges.is_empty() {
        let _ = writeln!(s, "\n## dependencies");
        for e in &out.edges {
            let _ = writeln!(s, "{} → {}   {}", loc(&e.from), loc(&e.to), e.why);
        }
    }
    // P14: advanced-construct advisories
    let advs: Vec<(String, &crate::model::Advisory)> = out
        .files
        .iter()
        .flat_map(|f| {
            f.hunks.iter().flat_map(move |h| {
                h.advisories
                    .iter()
                    .map(move |a| (format!("{}:L{}", f.path, h.new_range[0]), a))
            })
        })
        .collect();
    if !advs.is_empty() {
        let _ = writeln!(s, "\n## advisories");
        for (at, a) in advs {
            let mark = if a.verdict { " ⚠" } else { "" };
            let _ = writeln!(s, "{at}  {}{mark}", a.construct);
            for line in a.message.lines() {
                let _ = writeln!(s, "  {line}");
            }
        }
    }
    s
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

fn build_change(change: &Change, full_context: bool) -> ChangeParts {
    let old = change.old.as_deref();
    let (raw, new, degraded): (Vec<RawHunk>, String, bool) = if let Some(new) = &change.new {
        (compute_hunks(old.unwrap_or(""), new), new.clone(), false)
    } else if let (Some(old), Some(diff)) = (old, &change.diff) {
        // L1: reconstruct full new content by applying the diff to old
        match patch::apply(old, diff) {
            Some(new) => (compute_hunks(old, &new), new, false),
            None => from_diff(diff, full_context), // apply failed → positional (L3)
        }
    } else if let Some(diff) = &change.diff {
        // diff only: full for additions / opt-in full-context, else positional
        from_diff(diff, full_context)
    } else {
        (vec![], String::new(), false)
    };
    let mut sems = lang::for_path(&change.path)
        .and_then(|spec| analyze(spec, &new, &raw, &change.path))
        .unwrap_or_else(|| raw.iter().map(HunkSem::other).collect());
    // P12.2 noise: generated/vendored path, or a formatting-only hunk
    let generated = lang::is_generated_path(&change.path);
    let old_lines: Vec<&str> = old.unwrap_or("").lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    for (i, h) in raw.iter().enumerate() {
        sems[i].noise = generated || formatting_only(h, &old_lines, &new_lines);
    }
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
    // P15 detail layer: what the hunk did to the members of its container. The
    // container name is already on the hunk as `enclosing`; only the old side's
    // members need a second parse.
    if let Some(spec) = lang::for_path(&change.path) {
        let old_members = old
            .map(|o| extract::member_rows(spec, o))
            .unwrap_or_default();
        let phrases: Vec<Vec<String>> = raw
            .iter()
            .enumerate()
            .map(|(i, h)| detail_phrases(&sems[i], h, &old_members, spec.prose))
            .collect();
        for (i, d) in phrases.into_iter().enumerate() {
            sems[i].details = d;
        }
    }
    let switched: Vec<Option<SideShift>> = raw
        .iter()
        .map(|h| side_shift(h, &old_lines, &new_lines, ext))
        .collect();
    (raw, sems, degraded, comment_only, switched)
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
        Some((r[0]..=r[1]).zip(seg.iter()).all(|(line_no, l)| {
            is_comment_line(l.trim(), ext) || doc.is_some_and(|d| d.contains(&line_no))
        }))
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
            let mut cur = node.walk();
            let mut prev: Option<Node> = None;
            for stmt in node.named_children(&mut cur) {
                // the module/class/function's first statement is always a
                // docstring candidate; any later one only counts as the
                // "attribute docstring" convention (Sphinx/attrs) — a bare
                // string immediately after the assignment it documents. A
                // string elsewhere (after a `for`/`if`/`return`/…) is data or
                // dead code, not a comment, so it's left alone.
                let is_attr_doc_site = prev.is_some_and(|p| {
                    p.kind() == "expression_statement"
                        && p.named_child(0).is_some_and(|a| a.kind() == "assignment")
                });
                if (prev.is_none() || is_attr_doc_site) && is_bare_string_stmt(stmt) {
                    for row in stmt.start_position().row..=stmt.end_position().row {
                        out.insert(row + 1);
                    }
                }
                prev = Some(stmt);
            }
        }
    }
    let mut c = node.walk();
    for child in node.children(&mut c) {
        collect_comment_lines(child, lang_name, out);
    }
}

// Comment-line syntax by file extension. `#` is python-only (a C/C++
// preprocessor directive also starts with `#` but isn't a comment).
fn is_comment_line(trimmed: &str, ext: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    // an interpreter line is a comment to every grammar that allows one, and
    // reads as one in a diff; without this it classifies as nothing at all
    if trimmed.starts_with("#!") {
        return true;
    }
    match ext {
        "py" | "pyi" | "xsh" | "xonsh" | "xonshrc" => {
            trimmed.starts_with('#') || trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
        }
        "lua" => trimmed.starts_with("--"),
        _ => trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*'),
    }
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
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
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

/// The text of a comment line without its marker. Only the markers
/// `is_comment_line` recognises, so the two stay in step.
fn strip_comment_marker<'a>(line: &'a str, ext: &str) -> &'a str {
    let t = line.trim();
    let out = match ext {
        "py" | "pyi" => t.strip_prefix('#'),
        "lua" => t.strip_prefix("---").or_else(|| t.strip_prefix("--")),
        _ => t
            .strip_prefix("///")
            .or_else(|| t.strip_prefix("//"))
            .or_else(|| t.strip_prefix("/*"))
            .or_else(|| t.strip_prefix('*')),
    };
    out.unwrap_or(t).trim_end_matches("*/").trim()
}

/// What a hunk did to the named members of its container(s). New-side members
/// come from `analyze`, each already carrying its own container (P15's
/// attribution fix — see `extract::member_container`); the old side is
/// matched by the hunk's old line range. A name present on both sides of the
/// *same* container was edited; on one side only, added or removed. Members
/// under different containers (e.g. two distinct `add_argument(...)` calls
/// touched by one hunk) are never compared against each other.
fn detail_phrases(
    s: &HunkSem,
    h: &RawHunk,
    old_members: &[extract::MemberRow],
    prose: bool,
) -> Vec<String> {
    let [o0, o1] = h.old_range;
    // Anything the hunk introduces wholesale is already named by the rationale
    // ("adds type Fresh") — relisting the members it was born with says nothing
    // more. An empty old side is what makes it new; a container merely *edited*
    // on the line that declares it (a one-line enum) still earns its details.
    // Prose is the exception: a def (section) is also its parent's member, so
    // a brand-new subsection needs this layer to say which section it landed
    // in — but only when it HAS a parent; a wholly new top-level section still
    // stays silent here, same as every other language.
    if o0 > o1 && !s.defines.is_empty() && !(prose && s.enclosing.is_some()) {
        return vec![];
    }
    // A member with no identifiable container of its own (not in a call, no
    // enclosing definition) falls back to the hunk's enclosing definition,
    // same as before this member-level attribution existed. Placeholder
    // segments never reach the wording (as in the rationale itself).
    let clean =
        |c: String| (!c.split('.').any(|seg| seg == "<anonymous>" || seg == "_")).then_some(c);
    let fallback = s.enclosing.clone();
    let resolve = |c: &Option<String>| c.clone().or_else(|| fallback.clone()).and_then(clean);

    let mut old_by: HashMap<Option<String>, HashMap<&str, &str>> = HashMap::new();
    for (_row, n, t, ctr) in old_members
        .iter()
        .filter(|(row, ..)| o0 <= row + 1 && *row < o1)
    {
        old_by
            .entry(resolve(ctr))
            .or_default()
            .insert(n.as_str(), t.as_str());
    }
    let mut new_by: HashMap<Option<String>, HashMap<&str, &str>> = HashMap::new();
    for (n, t, ctr) in &s.members {
        new_by
            .entry(resolve(ctr))
            .or_default()
            .insert(n.as_str(), t.as_str());
    }

    let mut containers: Vec<Option<String>> = old_by.keys().chain(new_by.keys()).cloned().collect();
    containers.sort();
    containers.dedup();

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort();
        v
    }

    let mut out = vec![];
    let empty = HashMap::new();
    for container in containers {
        let old = old_by.get(&container).unwrap_or(&empty);
        let new = new_by.get(&container).unwrap_or(&empty);
        // A member that IS its own container names nothing new: a js
        // `{ run: () => {} }` makes `run` both the member and — once the
        // arrow is a definition — the enclosing def, which would read "adds
        // run to run". Attributing a member to `stack` before its own def is
        // pushed (see `member_container`) already keeps this from happening
        // for the def-container case; kept as a defensive backstop and to
        // cover a call whose callee or literal happens to equal a member name.
        let self_named = |name: &str| {
            container
                .as_deref()
                .is_some_and(|c| c == name || c.rsplit('.').next() == Some(name))
        };
        let added = sorted(
            new.keys()
                .filter(|n| !old.contains_key(*n) && !self_named(n))
                .copied()
                .collect(),
        );
        let removed = sorted(
            old.keys()
                .filter(|n| !new.contains_key(*n) && !self_named(n))
                .copied()
                .collect(),
        );
        // present on both sides: changed only when its own text moved, so a
        // member that merely shares a line with the real change isn't named
        let changed = sorted(
            new.iter()
                .filter(|(n, t)| old.get(*n).is_some_and(|o| o != *t) && !self_named(n))
                .map(|(n, _)| *n)
                .collect(),
        );
        for (verb, prep, names) in [
            ("adds", "to", added),
            ("removes", "from", removed),
            ("changes", "in", changed),
        ] {
            if names.is_empty() {
                continue;
            }
            let list = name_list(&names);
            let list = if prose && verb != "changes" {
                let noun = if names.len() == 1 {
                    "section"
                } else {
                    "sections"
                };
                format!("{noun} {list}")
            } else {
                list
            };
            out.push(match &container {
                Some(c) => format!("{verb} {list} {prep} {c}"),
                None => format!("{verb} {list}"),
            });
        }
    }
    out
}

// At most three names, then a count — a detail line is a glance, not a listing.
fn name_list(names: &[&str]) -> String {
    const SHOWN: usize = 3;
    // names can be container labels as well as symbols (a test block's label is
    // a sentence), so each one is shortened to what a rationale can afford
    let short: Vec<String> = names.iter().map(|n| order::short_container(n)).collect();
    if short.len() <= SHOWN {
        return short.join(", ");
    }
    format!(
        "{}, and {} more",
        short[..SHOWN].join(", "),
        short.len() - SHOWN
    )
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

fn from_diff(diff: &str, full_context: bool) -> (Vec<RawHunk>, String, bool) {
    let pf = patch::parse_file_diff(diff, full_context);
    match (pf.old, pf.new) {
        (Some(old), Some(new)) => (compute_hunks(&old, &new), new, false),
        _ => (pf.hunks, String::new(), true), // positional → degraded
    }
}
