//! ordo — comprehension-optimized ordering of code-change hunks.
//! Public entry: [`run`] takes an [`Input`] and returns the v1 [`Output`].
mod advisories;
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
    let n = input.changes.len();
    // per-file old/new symbol data (rows for positions, sets for membership,
    // bodies for rename/move matching) — drives #3/#5/#7 and P12.1 moves.
    let body_of = |list: &[(String, String)], name: &str| {
        list.iter()
            .find(|(nm, _)| nm == name)
            .map(|(_, b)| b.clone())
    };
    let mut old_defs: Vec<HashSet<String>> = vec![];
    let mut old_imports: Vec<HashSet<String>> = vec![];
    let mut new_defs_v: Vec<HashSet<String>> = vec![];
    let mut new_imports_v: Vec<HashSet<String>> = vec![];
    let mut old_rows: Vec<(Vec<(String, usize)>, Vec<(String, usize)>)> = vec![];
    let mut old_body: Vec<Vec<(String, String)>> = vec![];
    let mut new_body: Vec<Vec<(String, String)>> = vec![];
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
        new_defs_v.push(nd);
        new_imports_v.push(ni);
        old_rows.push(rows);
        old_body.push(ob);
        new_body.push(nb);
    }
    // P12.1: index freshly-appeared new defs by (name, body) → file, for moves
    let mut appeared: HashMap<(String, String), usize> = HashMap::new();
    for (fi, nb) in new_body.iter().enumerate() {
        for (name, body) in nb {
            if body.len() >= 8 && !old_defs[fi].contains(name) {
                appeared.entry((name.clone(), body.clone())).or_insert(fi);
            }
        }
    }

    let mut rename: Vec<HashMap<String, String>> = vec![HashMap::new(); n];
    let mut moved_in: Vec<HashMap<String, String>> = vec![HashMap::new(); n]; // new name → source path
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

        // #7 delete / #5 import remove / P12.1 move-out (else "removes")
        let mut rem = vec![];
        for (name, row) in odr {
            if nd.contains(name) || renamed_old.contains(name) {
                continue;
            }
            match moved_out.get(name) {
                Some(&tgt) => rem.push((*row, format!("moves {name} to {}", paths[tgt]))),
                None => rem.push((*row, format!("removes {name}"))),
            }
        }
        for (name, row) in oir {
            if !ni.contains(name) {
                rem.push((*row, format!("removes import {name}")));
            }
        }
        rename[fi] = ren;
        removals[fi] = rem;
    }
    let ordered = order::order_all(
        &sems,
        &paths,
        &old_defs,
        &old_imports,
        &rename,
        &moved_in,
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
                noise: sems[fi][li].noise,
                notes: sems[fi][li].notes.clone(),
                advisories: sems[fi][li].advisories.clone(),
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
            let _ = writeln!(
                s,
                "{path}:L{} [{cat}{noise}] {}{notes}",
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
    let mut sems = lang::for_path(&change.path)
        .and_then(|spec| analyze(spec, &new, &raw))
        .unwrap_or_else(|| raw.iter().map(HunkSem::other).collect());
    // P12.2 noise: generated/vendored path, or a formatting-only hunk
    let generated = lang::is_generated_path(&change.path);
    let old_lines: Vec<&str> = old.unwrap_or("").lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    for (i, h) in raw.iter().enumerate() {
        sems[i].noise = generated || formatting_only(h, &old_lines, &new_lines);
    }
    (raw, sems, degraded)
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
    match (slice(old_lines, h.old_range), slice(new_lines, h.new_range)) {
        (Some(o), Some(n)) => !o.is_empty() && o == n,
        _ => false,
    }
}

fn from_diff(diff: &str, full_context: bool) -> (Vec<RawHunk>, String, bool) {
    let pf = patch::parse_file_diff(diff, full_context);
    match (pf.old, pf.new) {
        (Some(old), Some(new)) => (compute_hunks(&old, &new), new, false),
        _ => (pf.hunks, String::new(), true), // positional → degraded
    }
}
