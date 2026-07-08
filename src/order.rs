//! Ordering across a whole changeset: group by (file, enclosing definition)
//! (P1), def→use edges between groups (P2) — cross-file when enabled (P4),
//! topological sort (Kahn) with deterministic (import, file, position) tiebreak.
//! Single-file is just the one-file case of this.
use crate::extract::HunkSem;
use crate::model::{Category, Strategy};
use std::collections::{HashMap, HashSet};

pub struct GroupInfo {
    pub reason: String,
    /// global hunk indices
    pub members: Vec<usize>,
}

pub struct OrderedAll {
    /// global hunk index -> (file, local hunk index)
    pub coord: Vec<(usize, usize)>,
    /// global reading order (global indices)
    pub perm: Vec<usize>,
    pub group_idx: Vec<usize>,
    pub groups: Vec<GroupInfo>,
    /// (from global idx, to global idx, why)
    pub edges: Vec<(usize, usize, String)>,
    pub rationale: Vec<String>,
}

fn cat_rank(c: Category) -> u8 {
    match c {
        Category::Import => 0,
        Category::Definition => 1,
        Category::Other => 2,
    }
}

pub fn order_all(files: &[Vec<HunkSem>], strategy: Strategy, cross_file: bool) -> OrderedAll {
    // ---- flatten all files into a global hunk list ----
    let mut coord = vec![];
    let mut sem: Vec<&HunkSem> = vec![];
    for (fi, hs) in files.iter().enumerate() {
        for (li, s) in hs.iter().enumerate() {
            coord.push((fi, li));
            sem.push(s);
        }
    }
    let n = sem.len();

    // ---- group by (file, enclosing definition); top-level hunks are singletons ----
    let mut idx_of_key: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<GroupInfo> = vec![];
    let mut group_idx = vec![0usize; n];
    for i in 0..n {
        let (fi, _) = coord[i];
        let key = match &sem[i].enclosing {
            Some(nm) => format!("{fi}\u{0}def:{nm}"),
            None => format!("{fi}\u{0}top:{i}"),
        };
        let gi = *idx_of_key.entry(key).or_insert_with(|| {
            let reason = match &sem[i].enclosing {
                Some(nm) => format!("same definition: {nm}"),
                None => match sem[i].category {
                    Category::Import => "import".to_string(),
                    Category::Definition => "top-level definition".to_string(),
                    Category::Other => "top-level change".to_string(),
                },
            };
            groups.push(GroupInfo {
                reason,
                members: vec![],
            });
            groups.len() - 1
        });
        group_idx[i] = gi;
        groups[gi].members.push(i);
    }
    let g = groups.len();
    let gfile = |gi: usize, groups: &[GroupInfo]| coord[groups[gi].members[0]].0;
    let grow = |gi: usize, groups: &[GroupInfo]| {
        groups[gi]
            .members
            .iter()
            .map(|&i| sem[i].start_row)
            .min()
            .unwrap_or(0)
    };
    let gimport = |gi: usize, groups: &[GroupInfo]| {
        groups[gi]
            .members
            .iter()
            .all(|&i| sem[i].category == Category::Import)
    };

    // ---- group defines/uses ----
    let mut gdef: Vec<HashSet<String>> = vec![HashSet::new(); g];
    let mut guse: Vec<HashSet<String>> = vec![HashSet::new(); g];
    for i in 0..n {
        let gi = group_idx[i];
        for d in &sem[i].defines {
            gdef[gi].insert(d.clone());
        }
        for u in &sem[i].uses {
            guse[gi].insert(u.clone());
        }
    }

    // ---- def→use edges (union symbol tables; cross-file only when enabled) ----
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut edges: Vec<(usize, usize, String)> = vec![];
    let mut gedges: Vec<(usize, usize)> = vec![];
    for a in 0..g {
        let mut defs_a: Vec<&String> = gdef[a].iter().collect();
        defs_a.sort();
        for b in 0..g {
            if a == b {
                continue;
            }
            if !cross_file && gfile(a, &groups) != gfile(b, &groups) {
                continue;
            }
            for s in &defs_a {
                if guse[b].contains(*s) && seen.insert((a, b)) {
                    let from_h = groups[a]
                        .members
                        .iter()
                        .find(|&&i| sem[i].defines.contains(*s))
                        .copied()
                        .unwrap_or(groups[a].members[0]);
                    let to_h = groups[b]
                        .members
                        .iter()
                        .find(|&&i| sem[i].uses.contains(*s))
                        .copied()
                        .unwrap_or(groups[b].members[0]);
                    edges.push((from_h, to_h, format!("def→use: {s}")));
                    gedges.push((a, b));
                }
            }
        }
    }

    // ---- order groups per strategy ----
    let group_order: Vec<usize> = match strategy {
        Strategy::File => {
            let mut v: Vec<usize> = (0..g).collect();
            v.sort_by_key(|&gi| (gfile(gi, &groups), grow(gi, &groups), gi));
            v
        }
        Strategy::DefsFirst => {
            let gcat = |gi: usize| {
                groups[gi]
                    .members
                    .iter()
                    .map(|&i| cat_rank(sem[i].category))
                    .min()
                    .unwrap_or(2)
            };
            let mut v: Vec<usize> = (0..g).collect();
            v.sort_by_key(|&gi| (gcat(gi), gfile(gi, &groups), grow(gi, &groups), gi));
            v
        }
        Strategy::Comprehension => {
            let mut indeg = vec![0usize; g];
            let mut succ: Vec<Vec<usize>> = vec![vec![]; g];
            for &(a, b) in &gedges {
                succ[a].push(b);
                indeg[b] += 1;
            }
            let key = |gi: usize, groups: &[GroupInfo]| {
                (
                    if gimport(gi, groups) { 0 } else { 1 },
                    gfile(gi, groups),
                    grow(gi, groups),
                    gi,
                )
            };
            let mut done = vec![false; g];
            let mut order = vec![];
            for _ in 0..g {
                let ready: Vec<usize> = (0..g).filter(|&gi| !done[gi] && indeg[gi] == 0).collect();
                let pick = if !ready.is_empty() {
                    *ready.iter().min_by_key(|&&gi| key(gi, &groups)).unwrap()
                } else {
                    // cycle: break by deterministic key
                    (0..g)
                        .filter(|&gi| !done[gi])
                        .min_by_key(|&gi| key(gi, &groups))
                        .unwrap()
                };
                done[pick] = true;
                order.push(pick);
                for &s in &succ[pick] {
                    if indeg[s] > 0 {
                        indeg[s] -= 1;
                    }
                }
            }
            order
        }
    };

    // ---- flatten groups to a global hunk permutation ----
    let mut perm = vec![];
    for &gi in &group_order {
        let mut mem = groups[gi].members.clone();
        mem.sort_by_key(|&i| (coord[i].0, sem[i].start_row, i));
        perm.extend(mem);
    }

    let rationale = (0..n)
        .map(|i| rationale_for(i, &sem, &group_idx, &groups, &guse))
        .collect();

    OrderedAll {
        coord,
        perm,
        group_idx,
        groups,
        edges,
        rationale,
    }
}

fn rationale_for(
    i: usize,
    sem: &[&HunkSem],
    group_idx: &[usize],
    groups: &[GroupInfo],
    guse: &[HashSet<String>],
) -> String {
    let s = sem[i];
    if s.category == Category::Import {
        return if s.imports.is_empty() {
            "import".to_string()
        } else {
            format!("imports {}", s.imports.join(", "))
        };
    }
    if !s.defines.is_empty() {
        let g = guse.len();
        let target = s.defines.iter().find_map(|d| {
            (0..g)
                .find(|&b| b != group_idx[i] && guse[b].contains(d))
                .map(|b| (d.clone(), b))
        });
        return match target {
            Some((d, b)) => match group_name(&groups[b], sem) {
                Some(nm) => format!("defines {d}, used by {nm} below"),
                None => format!("defines {}", s.defines.join(", ")),
            },
            None => format!("defines {}", s.defines.join(", ")),
        };
    }
    if let Some(nm) = &s.enclosing {
        return format!("changes {nm}");
    }
    if !s.uses.is_empty() {
        return format!("uses {}", s.uses.join(", "));
    }
    "change".to_string()
}

fn group_name(g: &GroupInfo, sem: &[&HunkSem]) -> Option<String> {
    g.members
        .iter()
        .find_map(|&i| sem[i].enclosing.clone())
        .or_else(|| {
            g.members
                .iter()
                .find_map(|&i| sem[i].defines.first().cloned())
        })
}
