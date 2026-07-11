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
    /// P12.3: independent parts — connected components of the group def→use
    /// graph, each a sorted list of global hunk indices.
    pub clusters: Vec<Vec<usize>>,
}

use crate::lang::is_test_path;

fn cat_rank(c: Category) -> u8 {
    match c {
        Category::Import => 0,
        Category::Definition => 1,
        Category::Other => 2,
    }
}

pub fn order_all(
    files: &[Vec<HunkSem>],
    paths: &[String],
    old_defs: &[HashSet<String>],
    old_imports: &[HashSet<String>],
    rename: &[HashMap<String, String>],
    moved_in: &[HashMap<String, String>],
    relocated: &[HashMap<String, String>],
    body_only: &[HashSet<String>],
    removals: &[Vec<(usize, String)>],
    strategy: Strategy,
    cross_file: bool,
) -> OrderedAll {
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

    // per-group file + source position, for provenance and above/below wording
    let group_file: Vec<usize> = (0..g).map(|gi| coord[groups[gi].members[0]].0).collect();
    let group_row: Vec<usize> = (0..g)
        .map(|gi| {
            groups[gi]
                .members
                .iter()
                .map(|&i| sem[i].start_row)
                .min()
                .unwrap_or(0)
        })
        .collect();
    let ctx = RatCtx {
        groups: &groups,
        gdef: &gdef,
        guse: &guse,
        group_file: &group_file,
        group_row: &group_row,
        paths,
        old_defs,
        old_imports,
        rename,
        moved_in,
        relocated,
        body_only,
        removals,
        cross_file,
    };
    let rationale = (0..n)
        .map(|i| rationale_for(i, &sem, &group_idx, &ctx))
        .collect();

    // P12.3: connected components of the group def→use graph = independent parts
    let mut parent: Vec<usize> = (0..g).collect();
    for &(a, b) in &gedges {
        let (ra, rb) = (uf_find(&mut parent, a), uf_find(&mut parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for gi in 0..g {
        let r = uf_find(&mut parent, gi);
        by_root
            .entry(r)
            .or_default()
            .extend(groups[gi].members.iter().copied());
    }
    let mut clusters: Vec<Vec<usize>> = by_root.into_values().collect();
    for c in &mut clusters {
        c.sort_unstable();
    }
    clusters.sort_by_key(|c| c.first().copied().unwrap_or(0));

    OrderedAll {
        coord,
        perm,
        group_idx,
        groups,
        edges,
        rationale,
        clusters,
    }
}

fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

struct RatCtx<'a> {
    groups: &'a [GroupInfo],
    gdef: &'a [HashSet<String>],
    guse: &'a [HashSet<String>],
    group_file: &'a [usize],
    group_row: &'a [usize],
    paths: &'a [String],
    old_defs: &'a [HashSet<String>],
    old_imports: &'a [HashSet<String>],
    rename: &'a [HashMap<String, String>],
    moved_in: &'a [HashMap<String, String>],
    relocated: &'a [HashMap<String, String>],
    body_only: &'a [HashSet<String>],
    removals: &'a [Vec<(usize, String)>],
    cross_file: bool,
}

impl RatCtx<'_> {
    // a candidate group is usable as provenance if it's another group and (when
    // cross_file is off) lives in the same file as the hunk's group
    fn ok(&self, mine: usize, other: usize) -> bool {
        other != mine && (self.cross_file || self.group_file[other] == self.group_file[mine])
    }
    // #3/#4: verb for a definition hunk. New symbol → "adds"/"adds type"; a
    // pre-existing symbol whose header changed → "changes signature of"/"changes
    // type" (a def-category hunk means the declaration line itself moved).
    fn def_verb(&self, file: usize, sym: &str, is_type: bool) -> &'static str {
        let existed = self.old_defs.get(file).is_some_and(|s| s.contains(sym));
        let body_only = self.body_only.get(file).is_some_and(|s| s.contains(sym));
        match (existed, is_type) {
            (false, false) => "adds",
            (false, true) => "adds type",
            // a def whose header is unchanged → body edit, not a signature change
            (true, false) if body_only => "edits",
            (true, false) => "changes signature of",
            (true, true) => "changes type",
        }
    }
    // "uses foo, defined in a.py" / "uses foo, defined above|below"
    fn use_of_phrase(&self, sym: &str, mine: usize, b: usize) -> String {
        if self.group_file[b] != self.group_file[mine] {
            format!("uses {sym}, defined in {}", self.paths[self.group_file[b]])
        } else {
            let dir = if self.group_row[b] < self.group_row[mine] {
                "above"
            } else {
                "below"
            };
            format!("uses {sym}, defined {dir}")
        }
    }
}

fn rationale_for(i: usize, sem: &[&HunkSem], group_idx: &[usize], ctx: &RatCtx) -> String {
    let s = sem[i];
    let mine = group_idx[i];
    let my_file = ctx.group_file[mine];
    let g = ctx.groups.len();

    // P12.2: noise hunks are skippable — say why, skip semantic wording
    if s.noise {
        return if crate::lang::is_generated_path(&ctx.paths[my_file]) {
            "generated file".to_string()
        } else {
            "formatting only".to_string()
        };
    }

    if s.category == Category::Import {
        if s.imports.is_empty() {
            return "import".to_string();
        }
        // #5 (add side): new import(s) → "adds import"; a touched existing one → "changes import"
        let all_new = s
            .imports
            .iter()
            .all(|im| !ctx.old_imports.get(my_file).is_some_and(|o| o.contains(im)));
        let verb = if all_new {
            "adds import"
        } else {
            "changes import"
        };
        return format!("{verb} {}", s.imports.join(", "));
    }

    // definition side: report EVERY construct the hunk touches, not just one.
    // Classify each defined symbol (moved-in / renamed / extracted / added /
    // changed), group same-verb symbols, and append provenance once.
    if !s.defines.is_empty() {
        let (mut moves, mut renames, mut extracts) = (vec![], vec![], vec![]);
        let (mut adds, mut adds_ty, mut edits, mut ch_sig, mut ch_ty): (
            Vec<&str>,
            Vec<&str>,
            Vec<&str>,
            Vec<&str>,
            Vec<&str>,
        ) = Default::default();
        for d in &s.defines {
            if let Some(src) = ctx.moved_in.get(my_file).and_then(|m| m.get(d)) {
                moves.push(format!("moves {d} from {src}"));
            } else if let Some(old) = ctx.rename.get(my_file).and_then(|m| m.get(d)) {
                renames.push(format!("renames {old} → {d}"));
            } else if let Some(src) = ctx.relocated.get(my_file).and_then(|m| m.get(d)) {
                extracts.push(format!("adds {d}, extracted from {src}"));
            } else {
                match ctx.def_verb(my_file, d, s.is_type) {
                    "adds" => adds.push(d),
                    "adds type" => adds_ty.push(d),
                    "edits" => edits.push(d),
                    "changes type" => ch_ty.push(d),
                    _ => ch_sig.push(d),
                }
            }
        }
        let mut frags: Vec<String> = vec![];
        frags.append(&mut extracts);
        frags.append(&mut renames);
        frags.append(&mut moves);
        if !adds.is_empty() {
            frags.push(format!("adds {}", adds.join(", ")));
        }
        if !adds_ty.is_empty() {
            frags.push(format!("adds type {}", adds_ty.join(", ")));
        }
        if !edits.is_empty() {
            frags.push(format!("edits {}", edits.join(", ")));
        }
        if !ch_sig.is_empty() {
            frags.push(format!("changes signature of {}", ch_sig.join(", ")));
        }
        if !ch_ty.is_empty() {
            frags.push(format!("changes type {}", ch_ty.join(", ")));
        }
        let mut out = frags.join("; ");
        // provenance: a defined symbol used by another group. Name it only when
        // several constructs are listed (otherwise "used by X" is unambiguous).
        if let Some((d, b)) = s.defines.iter().find_map(|d| {
            (0..g)
                .find(|&b| ctx.ok(mine, b) && ctx.guse[b].contains(d))
                .map(|b| (d.clone(), b))
        }) {
            let prov = if ctx.group_file[b] != my_file {
                format!("used in {}", ctx.paths[ctx.group_file[b]])
            } else {
                let dir = if ctx.group_row[b] > ctx.group_row[mine] {
                    "below"
                } else {
                    "above"
                };
                match group_name(&ctx.groups[b], sem) {
                    Some(nm) => format!("used by {nm} {dir}"),
                    None => format!("used {dir}"),
                }
            };
            out += &if frags.len() > 1 {
                format!(", {d} {prov}")
            } else {
                format!(", {prov}")
            };
        }
        return out;
    }

    // use side: a symbol used here that some other group defines (this change)
    if !s.uses.is_empty() {
        // #6: from a test file, prefer a symbol defined in a non-test file
        if is_test_path(&ctx.paths[my_file]) {
            if let Some((u, b)) = s.uses.iter().find_map(|u| {
                (0..g)
                    .find(|&b| {
                        ctx.ok(mine, b)
                            && ctx.gdef[b].contains(u)
                            && ctx.group_file[b] != my_file
                            && !is_test_path(&ctx.paths[ctx.group_file[b]])
                    })
                    .map(|b| (u.clone(), b))
            }) {
                return format!("tests {u} ({})", ctx.paths[ctx.group_file[b]]);
            }
        }
        if let Some((u, b)) = s.uses.iter().find_map(|u| {
            (0..g)
                .find(|&b| ctx.ok(mine, b) && ctx.gdef[b].contains(u))
                .map(|b| (u.clone(), b))
        }) {
            return ctx.use_of_phrase(&u, mine, b);
        }
        if let Some(nm) = &s.enclosing {
            return format!("edits {nm}");
        }
        return format!("uses {}", s.uses.join(", "));
    }

    if let Some(nm) = &s.enclosing {
        return format!("edits {nm}");
    }
    // #5/#7 removal: a deletion hunk whose old lines held a removed symbol
    let [o0, o1] = s.old_range;
    if let Some(label) = ctx.removals.get(my_file).and_then(|v| {
        v.iter()
            .find(|(row, _)| *row >= o0 && *row <= o1)
            .map(|(_, l)| l.clone())
    }) {
        return label;
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
