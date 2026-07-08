//! ordo — comprehension-optimized ordering of code-change hunks.
//! Public entry: [`run`] takes an [`Input`] and returns the v1 [`Output`].
mod extract;
mod lang;
pub mod model;
mod order;
mod patch;

use extract::{analyze, compute_hunks, HunkSem, RawHunk};
use model::*;

pub use patch::split_patch;

pub const SCHEMA_VERSION: u32 = 1;

pub fn run(input: Input) -> Output {
    // per-file hunks + semantics
    let mut raws: Vec<Vec<RawHunk>> = vec![];
    let mut sems: Vec<Vec<HunkSem>> = vec![];
    for change in &input.changes {
        let (raw, sem) = build_change(change);
        raws.push(raw);
        sems.push(sem);
    }

    let ordered = order::order_all(&sems, input.options.strategy, input.options.cross_file);

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

/// Compute hunks + semantics for one change. Prefers old/new (full semantics
/// via `similar` + tree-sitter); falls back to a parsed `diff` (positional for
/// modified files, full for additions — see `patch.rs`). Unsupported/unparsable
/// language degrades to "other" semantics (file order preserved).
fn build_change(change: &Change) -> (Vec<RawHunk>, Vec<HunkSem>) {
    let (raw, new): (Vec<RawHunk>, String) = match (&change.old, &change.new) {
        (Some(old), Some(new)) => (compute_hunks(old, new), new.clone()),
        _ => match &change.diff {
            Some(d) => {
                let pf = patch::parse_file_diff(d);
                (pf.hunks, pf.new.unwrap_or_default())
            }
            None => (vec![], String::new()),
        },
    };
    let sems = lang::for_path(&change.path)
        .and_then(|spec| analyze(spec, &new, &raw))
        .unwrap_or_else(|| raw.iter().map(HunkSem::other).collect());
    (raw, sems)
}
