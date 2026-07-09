//! ordo — comprehension-optimized ordering of code-change hunks.
//! Public entry: [`run`] takes an [`Input`] and returns the v1 [`Output`].
mod extract;
mod lang;
pub mod model;
mod order;
mod patch;

use extract::{analyze, compute_hunks, symbol_sets, HunkSem, RawHunk};
use model::*;
use std::collections::{HashMap, HashSet};

pub use patch::split_patch;

pub const SCHEMA_VERSION: u32 = 1;

pub fn run(input: Input) -> Output {
    // per-file hunks + semantics
    let mut raws: Vec<Vec<RawHunk>> = vec![];
    let mut sems: Vec<Vec<HunkSem>> = vec![];
    let mut degraded: Vec<bool> = vec![];
    for change in &input.changes {
        let (raw, sem, deg) = build_change(change, input.options.full_context);
        if deg {
            eprintln!(
                "ordo: {}: diff lacks full context — positional order only (use old/new, or pass a full-context patch with `full_context`/`--full-context`)",
                change.path
            );
        }
        raws.push(raw);
        sems.push(sem);
        degraded.push(deg);
    }

    let paths: Vec<String> = input.changes.iter().map(|c| c.path.clone()).collect();
    // old-side symbols (#3 add-vs-edit, #5 import remove, #7 rename/delete) —
    // one parse of old (with rows) and new (sets) per file.
    let mut old_defs: Vec<HashSet<String>> = vec![];
    let mut old_imports: Vec<HashSet<String>> = vec![];
    let mut rename: Vec<HashMap<String, String>> = vec![];
    let mut removals: Vec<Vec<(usize, String)>> = vec![]; // (old-line, "removes …")
    for c in &input.changes {
        let spec = lang::for_path(&c.path);
        let (odr, oir) = match (c.old.as_deref(), spec) {
            (Some(old), Some(sp)) => extract::symbol_rows(sp, old),
            _ => (vec![], vec![]),
        };
        let (new_defs, new_imports) = match (c.new.as_deref(), spec) {
            (Some(new), Some(sp)) => symbol_sets(sp, new),
            _ => (HashSet::new(), HashSet::new()),
        };
        let od: HashSet<String> = odr.iter().map(|(n, _)| n.clone()).collect();
        let oi: HashSet<String> = oir.iter().map(|(n, _)| n.clone()).collect();
        // #7 rename: match removed↔added defs by body (P11.2), then a 1:1 fallback
        let mut removed_d: Vec<String> = od.difference(&new_defs).cloned().collect();
        let mut added_d: Vec<String> = new_defs.difference(&od).cloned().collect();
        removed_d.sort();
        added_d.sort();
        let mut ren = HashMap::new();
        if !removed_d.is_empty() && !added_d.is_empty() {
            let ob = spec
                .map(|sp| extract::symbol_bodies(sp, c.old.as_deref().unwrap_or("")))
                .unwrap_or_default();
            let nb = spec
                .map(|sp| extract::symbol_bodies(sp, c.new.as_deref().unwrap_or("")))
                .unwrap_or_default();
            let body = |list: &[(String, String)], name: &str| {
                list.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone())
            };
            // a removed def whose (non-trivial) body reappears under a new name
            for r in &removed_d {
                let rb = match body(&ob, r) {
                    Some(b) if b.len() >= 8 => b,
                    _ => continue,
                };
                if let Some(a) = added_d
                    .iter()
                    .find(|a| !ren.contains_key(*a) && body(&nb, a).as_deref() == Some(rb.as_str()))
                {
                    ren.insert(a.clone(), r.clone());
                }
            }
            // lone unmatched removed+added → rename even if the body changed
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
        // #7 delete / #5 import remove: gone from new, not a rename
        let mut rem = vec![];
        for (n, row) in &odr {
            if !new_defs.contains(n) && !renamed_old.contains(n) {
                rem.push((*row, format!("removes {n}")));
            }
        }
        for (n, row) in &oir {
            if !new_imports.contains(n) {
                rem.push((*row, format!("removes import {n}")));
            }
        }
        old_defs.push(od);
        old_imports.push(oi);
        rename.push(ren);
        removals.push(rem);
    }
    let ordered = order::order_all(
        &sems,
        &paths,
        &old_defs,
        &old_imports,
        &rename,
        &removals,
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
                defines: sems[fi][li].defines.clone(),
                uses: sems[fi][li].uses.clone(),
                group: gids[ordered.group_idx[gi]].clone(),
                order_index: order_index[fi][li],
                rationale: ordered.rationale[gi].clone(),
            });
        }
        files.push(FileOut {
            path: change.path.clone(),
            hunks,
            degraded: degraded[fi],
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

    Output {
        schema: SCHEMA_VERSION,
        order,
        files,
        groups: groups_out,
        edges: edges_out,
    }
}

/// Compute hunks + semantics for one change. Full semantics whenever complete
/// new content is available: `new` given, or reconstructed from `old`+`diff`
/// (L1), or a full-context diff (L2 — inside `parse_file_diff`). A partial
/// diff-only change stays positional (L3). Unsupported/unparsable language
/// degrades to "other" semantics (file order preserved).
/// Returns (hunks, semantics, degraded). `degraded` = the change carried a diff
/// but full content couldn't be obtained, so ordering is positional only.
fn build_change(change: &Change, full_context: bool) -> (Vec<RawHunk>, Vec<HunkSem>, bool) {
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
    let sems = lang::for_path(&change.path)
        .and_then(|spec| analyze(spec, &new, &raw))
        .unwrap_or_else(|| raw.iter().map(HunkSem::other).collect());
    (raw, sems, degraded)
}

fn from_diff(diff: &str, full_context: bool) -> (Vec<RawHunk>, String, bool) {
    let pf = patch::parse_file_diff(diff, full_context);
    match (pf.old, pf.new) {
        (Some(old), Some(new)) => (compute_hunks(&old, &new), new, false),
        _ => (pf.hunks, String::new(), true), // positional → degraded
    }
}
