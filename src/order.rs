//! Ordering across a whole changeset: group by (file, enclosing definition)
//! (P1), def→use edges between groups (P2) — cross-file when enabled (P4),
//! topological sort (Kahn) with deterministic (import, file, position) tiebreak.
//! Single-file is just the one-file case of this.
use crate::extract::{self, BindingUse, HunkSem, RawHunk};
use crate::model::{Category, Removal, RemovalKind, Strategy};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// One-line rationale bound (mirrors tests/rationale_bounds.rs's own
/// MAX_RATIONALE) — used where composing two independently-capped fragments
/// can still exceed it, so the composition itself needs a length check.
const MAX_RATIONALE: usize = 240;

/// Join rationale fragments within the one-line budget. Every fragment is
/// already name-capped, but several capped fragments still overflow together —
/// python test names run past 60 characters, so three of them exceed the bound
/// on their own. Keep what fits and say how many were left out, rather than
/// emitting a line the contract forbids.
fn join_frags(frags: &[String]) -> String {
    const SUFFIX: usize = 12; // room for the "; +N more" tail
    let budget = MAX_RATIONALE - SUFFIX;
    let mut out = String::new();
    let mut dropped = 0usize;
    for f in frags {
        let sep = if out.is_empty() { 0 } else { 2 };
        if out.chars().count() + sep + f.chars().count() <= budget {
            if !out.is_empty() {
                out.push_str("; ");
            }
            out.push_str(f);
        } else {
            dropped += 1;
        }
    }
    if out.is_empty() {
        // even one fragment overflows on its own: keep a truncated head, since
        // a clipped name still orients better than no wording at all
        if let Some(f) = frags.first() {
            out = f.chars().take(budget.saturating_sub(1)).collect::<String>() + "…";
            dropped -= 1;
        }
    }
    if dropped > 0 {
        out.push_str(&format!("; +{dropped} more"));
    }
    out
}

/// Symbols grouped by the verb their change earns: adds, adds type, edits,
/// changes signature of, changes type.
type Verbs<'a> = (
    Vec<&'a str>,
    Vec<&'a str>,
    Vec<&'a str>,
    Vec<&'a str>,
    Vec<&'a str>,
);

/// A container name, shortened to what a rationale can afford.
///
/// `enclosing` carries the full path because a consumer navigating to the hunk
/// wants it, but a rationale is one line: a nested test label
/// (`describe "compiler: transform v-bind" > it "errors on a bad argument"`)
/// spends the whole budget on the container and leaves none for what changed.
/// The innermost segment identifies it; anything still very long is elided
/// rather than allowed to crowd out the rest of the sentence.
pub(crate) fn short_container(name: &str) -> String {
    // Only a *test* path collapses to its innermost segment: its outer levels
    // are sentences a reviewer already read in the file. A markdown section
    // path is the opposite — `Project > Install` is the navigation, and
    // dropping `Project` would point at the wrong heading — so prose keeps its
    // full path. A test label always carries the quotes of the string it was
    // named by; a heading does not.
    let quoted = |s: &str| s.contains('"') || s.contains('`') || s.contains('\'');
    let inner = match name.rsplit_once(" > ") {
        Some((_, last)) if quoted(last) => last,
        _ => name,
    };
    if inner.chars().count() <= CONTAINER_BUDGET {
        return inner.to_string();
    }
    let head: String = inner.chars().take(CONTAINER_BUDGET - 1).collect();
    format!("{head}…")
}

/// How much of a rationale one container name may occupy.
const CONTAINER_BUDGET: usize = 60;

/// Last-resort guard on the one-line bound. `rationale_for` composes through
/// many paths — fragments, provenance clauses, binding clauses — each capped on
/// its own, and no single one of them can see the finished length. Clamping
/// once, where every path lands, is what actually holds the contract.
fn clamp_rationale(r: String) -> String {
    if r.chars().count() <= MAX_RATIONALE {
        return r;
    }
    // cut at a fragment boundary when there is one, so the line still ends on a
    // complete statement rather than mid-name
    let head: String = r.chars().take(MAX_RATIONALE - 2).collect();
    // `rfind` gives a byte offset; the "did we keep enough of the line" test is
    // in characters, so on a non-ASCII rationale comparing the two directly
    // overstated how much was kept and cut at a boundary it should have passed
    match head.rfind("; ") {
        Some(i) if head[..i].chars().count() > MAX_RATIONALE / 2 => {
            format!("{}…", &head[..i])
        }
        _ => format!("{head}…"),
    }
}

/// One fragment per source reads naturally; several sources fold into a single
/// fragment with each name carrying its own source, so the relation phrase is
/// said once however many sources are involved.
fn src_frags(pairs: &[(&str, &str)], verb: &str, sep: &str, rel: &str) -> Vec<String> {
    let groups = group_by_src(pairs);
    match groups.len() {
        0 => vec![],
        1 => {
            let (src, names) = &groups[0];
            vec![format!("{verb} {}{sep}{rel} {src}", name_list(names))]
        }
        _ => {
            let mut each: Vec<String> = groups
                .iter()
                .flat_map(|(src, names)| names.iter().map(move |n| format!("{n} ({rel} {src})")))
                .collect();
            each.sort();
            let refs: Vec<&str> = each.iter().map(String::as_str).collect();
            vec![format!("{verb} {}", name_list(&refs))]
        }
    }
}

/// A group's sort key: rule priority (descending), then file, then source row,
/// then the group index — which makes every key distinct, so a sorted set of
/// them doubles as the ready queue in the topological sort.
type GroupKey = (std::cmp::Reverse<i64>, usize, usize, usize);

/// How a removal reads. The pipeline decides *what* left; this decides how to
/// say it, which is the only place wording belongs.
fn removal_phrase(r: &Removal) -> String {
    match &r.kind {
        RemovalKind::MovedTo(path) => format!("moves {} to {path}", r.name),
        RemovalKind::Section => format!("removes section {}", r.name),
        RemovalKind::Import => format!("removes import {}", r.name),
        RemovalKind::Def => format!("removes {}", r.name),
    }
}

fn cat_rank(c: Category) -> u8 {
    match c {
        Category::Import => 0,
        Category::Definition => 1,
        Category::Other => 2,
    }
}

/// What the caller worked out about each file before ordering, every field
/// indexed by file. Bundled rather than passed one by one: these travel
/// together, are derived together, and the list grows whenever the rationale
/// layer learns to say something new.
pub struct FileFacts<'a> {
    /// what each file's two sides declare (old defs, imports, locals, …)
    pub symbols: &'a [crate::FileSymbols],
    /// what the change did to each file's definitions
    pub changed: &'a [crate::FileChanges],
}

pub fn order_all(
    files: &[crate::PerFileHunks],
    paths: &[String],
    facts: &FileFacts,
    strategy: Strategy,
    cross_file: bool,
) -> OrderedAll {
    let FileFacts { symbols, changed } = *facts;
    // ---- flatten all files into a global hunk list ----
    let mut coord = vec![];
    let mut sem: Vec<&HunkSem> = vec![];
    let mut comment: Vec<bool> = vec![];
    let mut switched_off: Vec<Option<crate::SideShift>> = vec![];
    for (fi, f) in files.iter().enumerate() {
        for (li, s) in f.sem.iter().enumerate() {
            coord.push((fi, li));
            sem.push(s);
            comment.push(f.comment.get(li).copied().unwrap_or(false));
            switched_off.push(f.switched.get(li).copied().flatten());
        }
    }
    let n = sem.len();

    // ---- group by (file, enclosing definition); top-level hunks (no enclosing
    // definition) share one group per file — module constants, imports,
    // exports and similar file-scope hunks all live in the same scope, so
    // they group the same way two hunks in the same function do ----
    let mut idx_of_key: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<GroupInfo> = vec![];
    let mut group_idx = vec![0usize; n];
    for i in 0..n {
        let (fi, _) = coord[i];
        // imports group together, apart from the module-level code they sit
        // among: they are bookkeeping, they read as a block, and mixing them
        // into the top-level group would drag that group up to the first import
        // line — which cost the `file` strategy its positional promise.
        let import = sem[i].category == Category::Import;
        let key = match (&sem[i].enclosing, import) {
            (_, true) => format!("{fi}\u{0}imports"),
            (Some(nm), _) => format!("{fi}\u{0}def:{nm}"),
            (None, _) => format!("{fi}\u{0}top"),
        };
        let gi = *idx_of_key.entry(key).or_insert_with(|| {
            let reason = match (&sem[i].enclosing, import) {
                (_, true) => "same scope: imports".to_string(),
                (Some(nm), _) => format!("same definition: {nm}"),
                (None, _) => "same scope: top-level".to_string(),
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
            // `from lib import helper as h` then `h()`: the definition is
            // called `helper` in the file that has it, so the alias alone can
            // never match one. Record the origin too, and leave the hunk's own
            // `uses` saying what the source says.
            if let Some((origin, _)) = symbols
                .get(coord[i].0)
                .and_then(|f| f.imported_from.get(u.as_str()))
            {
                guse[gi].insert(origin.clone());
            }
        }
    }

    // ---- def→use edges (union symbol tables; cross-file only when enabled) ----
    // `users` inverts guse once (symbol → the groups using it, ascending), so a
    // group's defs look up their users directly instead of scanning every other
    // group: the corpus reaches ~15k hunks in one repo, where g² does not hold.
    let mut users: HashMap<&str, Vec<usize>> = HashMap::new();
    for (b, u) in guse.iter().enumerate() {
        for s in u {
            users.entry(s.as_str()).or_default().push(b);
        }
    }
    let mut definers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (b, d) in gdef.iter().enumerate() {
        for s in d {
            definers.entry(s.as_str()).or_default().push(b);
        }
    }
    // (group, symbol) pairs the symbol is declared *inside another definition*
    // in — a class member, a name in a namespace — rather than at file scope.
    // Built from `symbols`, which is `defines` minus the names that are not
    // declarations of this file's own (an import, a test label): those name
    // something whose scope lives elsewhere, and treating them as file-scope is
    // what lets an imported name keep resolving across files.
    let mut scoped: HashSet<(usize, &str)> = HashSet::new();
    for i in 0..n {
        for sym in &sem[i].symbols {
            if sym.scope.is_some() {
                scoped.insert((group_idx[i], sym.name.as_str()));
            }
        }
    }
    // gfile/grow depend only on group membership, which is fixed by this
    // point — precompute once instead of re-walking every member hunk on
    // every sort/min_by_key comparison below.
    let gfile_v: Vec<usize> = (0..g).map(|gi| gfile(gi, &groups)).collect();
    let grow_v: Vec<usize> = (0..g).map(|gi| grow(gi, &groups)).collect();
    let bind = Binding {
        definers: &definers,
        scoped: &scoped,
        group_file: &gfile_v,
        paths,
        symbols,
    };
    let mut edges: Vec<(usize, usize, String)> = vec![];
    let mut gedges: Vec<(usize, usize)> = vec![];
    for a in 0..g {
        let mut defs_a: Vec<&String> = gdef[a].iter().collect();
        defs_a.sort();
        // one edge per (a, b), named by a's first symbol that b uses
        let mut reached: BTreeMap<usize, &String> = BTreeMap::new();
        for s in &defs_a {
            for &b in users.get(s.as_str()).into_iter().flatten() {
                if b == a || (!cross_file && gfile(a, &groups) != gfile(b, &groups)) {
                    continue;
                }
                if !bind.resolves(a, b, s) {
                    continue;
                }
                reached.entry(b).or_insert(s);
            }
        }
        for (b, s) in reached {
            let from_h = groups[a]
                .members
                .iter()
                .find(|&&i| sem[i].defines.contains(s))
                .copied()
                .unwrap_or(groups[a].members[0]);
            let to_h = groups[b]
                .members
                .iter()
                .find(|&&i| sem[i].uses.contains(s))
                .copied()
                .unwrap_or(groups[b].members[0]);
            edges.push((from_h, to_h, format!("def→use: {s}")));
            gedges.push((a, b));
        }
    }

    // ---- containment edges: a def introduced here whose `scope` names
    // another group's own enclosing definition — i.e. this def lives nested
    // inside that other def, and both changed in this diff. Barnett et al.,
    // ICSE 2015 (ClusterChanges) model containment as a distinct edge in the
    // same graph as def→use, specifically so it feeds connected components
    // without flattening the grouping key to the outermost def — two
    // unrelated sibling methods of one class never share a group just for
    // sharing a container, only a def and the parent def it nests inside do.
    // Cluster membership only (see order_all's doc comment) — kept out of
    // `gedges`/Kahn's indegree, which drives P2 reading order: a containment
    // edge that also gated the topo sort could conflict with a def→use edge
    // running the other way (a nested helper the parent def calls), forcing
    // a cycle-break where none exists today; measured against the corpus,
    // clustering the two together already recovers the "these changed
    // together" signal without risking that.
    let mut def_group: HashMap<(usize, String), usize> = HashMap::new();
    for i in 0..n {
        if let Some(nm) = &sem[i].enclosing {
            def_group
                .entry((coord[i].0, nm.clone()))
                .or_insert(group_idx[i]);
        }
    }
    let mut seen_contain: HashSet<(usize, usize)> = HashSet::new();
    let mut contain_gedges: Vec<(usize, usize)> = vec![];
    for i in 0..n {
        let cgi = group_idx[i];
        for sym in &sem[i].symbols {
            let Some(sc) = &sym.scope else { continue };
            let Some(&pgi) = def_group.get(&(coord[i].0, sc.clone())) else {
                continue;
            };
            if pgi == cgi || !seen_contain.insert((pgi, cgi)) {
                continue;
            }
            let from_h = groups[pgi]
                .members
                .iter()
                .find(|&&h| sem[h].enclosing.as_deref() == Some(sc.as_str()))
                .copied()
                .unwrap_or(groups[pgi].members[0]);
            edges.push((from_h, i, format!("encloses: {}", sym.name)));
            contain_gedges.push((pgi, cgi));
        }
    }

    // gfile/grow depend only on group membership, which is fixed by this
    // point — precompute once instead of re-walking every member hunk on
    // every sort/min_by_key comparison below (and again further down for
    // group_file/group_row, which are the same values).
    let gfile_v: Vec<usize> = (0..g).map(|gi| gfile(gi, &groups)).collect();
    let grow_v: Vec<usize> = (0..g).map(|gi| grow(gi, &groups)).collect();

    // ---- order groups per strategy ----
    let group_order: Vec<usize> = match strategy {
        Strategy::File => {
            let mut v: Vec<usize> = (0..g).collect();
            v.sort_by_key(|&gi| (gfile_v[gi], grow_v[gi], gi));
            v
        }
        Strategy::DefsFirst => {
            let gcat_v: Vec<u8> = (0..g)
                .map(|gi| {
                    groups[gi]
                        .members
                        .iter()
                        .map(|&i| cat_rank(sem[i].category))
                        .min()
                        .unwrap_or(2)
                })
                .collect();
            let mut v: Vec<usize> = (0..g).collect();
            v.sort_by_key(|&gi| (gcat_v[gi], gfile_v[gi], grow_v[gi], gi));
            v
        }
        Strategy::Comprehension => {
            let mut indeg = vec![0usize; g];
            let mut succ: Vec<Vec<usize>> = vec![vec![]; g];
            for &(a, b) in &gedges {
                succ[a].push(b);
                indeg[b] += 1;
            }
            // A group's rule priority is the highest any of its hunks carries.
            // It enters the key *after* the import rank and *before* file
            // position, so it replaces the positional tiebreaker among groups
            // the graph has already freed — never the graph itself. A rule
            // cannot pull a use ahead of its definition.
            let gprio_v: Vec<i64> = (0..g)
                .map(|gi| {
                    groups[gi]
                        .members
                        .iter()
                        .map(|&i| sem[i].priority)
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            // Imports sort where they live, not first. Ranking them ahead of
            // everything made sense while they were dropped and never seen; now
            // that they are visible noise, leading with forty dimmed rows
            // buries the change they came with. A reviewer who does want them
            // first can say so with a rule (`priority`).
            // Static for the whole Kahn loop below (depends only on group
            // membership, not on which groups are done/ready), so it is
            // computed once here rather than per ready-set comparison.
            let key_v: Vec<GroupKey> = (0..g)
                .map(|gi| (std::cmp::Reverse(gprio_v[gi]), gfile_v[gi], grow_v[gi], gi))
                .collect();
            // The ready set is carried across iterations rather than rebuilt by
            // scanning every group on each of the g rounds: a group joins it
            // exactly when its indegree reaches zero. `key_v` ends in `gi`, so
            // its values are distinct and a sorted set of them is a priority
            // queue that also supports the removals the cycle break needs.
            // Same reason `users` exists above — the corpus reaches ~15k hunks
            // in one repo, where g² does not hold.
            let mut ready: BTreeSet<GroupKey> = (0..g)
                .filter(|&gi| indeg[gi] == 0)
                .map(|gi| key_v[gi])
                .collect();
            let mut left: BTreeSet<GroupKey> = (0..g).map(|gi| key_v[gi]).collect();
            let mut order = vec![];
            for _ in 0..g {
                // lowest key among the freed groups, else — a cycle — the
                // lowest key still standing, which is the same deterministic
                // tiebreak applied to a set the graph never released
                let key = match ready.first().or_else(|| left.first()) {
                    Some(k) => *k,
                    None => break,
                };
                let pick = key.3;
                ready.remove(&key);
                left.remove(&key);
                order.push(pick);
                for &s in &succ[pick] {
                    if indeg[s] > 0 {
                        indeg[s] -= 1;
                        if indeg[s] == 0 && left.contains(&key_v[s]) {
                            ready.insert(key_v[s]);
                        }
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

    // per-group file + source position, for provenance and above/below
    // wording — same values as gfile_v/grow_v above, reused rather than
    // re-walking every member hunk a second time.
    let ctx = RatCtx {
        groups: &groups,
        definers: &definers,
        users: &users,
        bind: Binding {
            definers: &definers,
            scoped: &scoped,
            group_file: &gfile_v,
            paths,
            symbols,
        },
        group_file: &gfile_v,
        group_row: &grow_v,
        paths,
        symbols,
        changed,
        comment: &comment,
        switched: &switched_off,
        cross_file,
    };
    let rationale = (0..n)
        .map(|i| clamp_rationale(rationale_for(i, &sem, &group_idx, &ctx)))
        .collect();

    // P12.3: connected components of the group def→use graph = independent
    // parts; containment edges join components here too (a nested def and
    // its parent def are part of the same change even with no def→use edge
    // between them) without joining the groups themselves.
    let mut parent: Vec<usize> = (0..g).collect();
    for &(a, b) in gedges.iter().chain(contain_gedges.iter()) {
        let (ra, rb) = (uf_find(&mut parent, a), uf_find(&mut parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for (gi, group) in groups.iter().enumerate() {
        let r = uf_find(&mut parent, gi);
        by_root
            .entry(r)
            .or_default()
            .extend(group.members.iter().copied());
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

/// Can a use in group `b` be attributed to the definition of `s` in group `a`?
///
/// Two shapes make that a guess rather than a fact, and a guess sends the
/// reviewer to the wrong definition:
///
///   * several groups define `s` — nothing here says which one the use means;
///   * `s` is declared inside another definition (a class member, a name in a
///     namespace) and the use is in a different file — resolving that needs the
///     imports, qualifications and overload rules this engine does not read.
///
/// Both are common in c++ header code, where short member names (`View`,
/// `name`, `at`, `i`) repeat in every class, but neither is language-specific.
/// The two groups are NOT interchangeable: it is the *definer's* scope that
/// decides, so `definer` and `user` must be passed the way round their names
/// say (pinned by `a_use_side_scope_does_not_block_the_edge`).
/// The def→use gate: everything an edge from `definer` to `user` for symbol
/// `s` has to survive. Bundled because it needs five tables and a method reads
/// better than six arguments at two call sites.
struct Binding<'a> {
    definers: &'a HashMap<&'a str, Vec<usize>>,
    scoped: &'a HashSet<(usize, &'a str)>,
    group_file: &'a [usize],
    paths: &'a [String],
    symbols: &'a [crate::FileSymbols],
}

impl Binding<'_> {
    /// Can a use in group `b` be attributed to the definition of `s` in group
    /// `a`?
    ///
    /// Two shapes make that a guess rather than a fact, and a guess sends the
    /// reviewer to the wrong definition:
    ///
    ///   * several groups define `s` — nothing here says which one the use
    ///     means;
    ///   * `s` is declared inside another definition (a class member, a name in
    ///     a namespace) and the use is in a different file — resolving that
    ///     needs the qualifications and overload rules this engine does not
    ///     read.
    ///
    /// Both are common in c++ header code, where short member names (`View`,
    /// `name`, `at`, `i`) repeat in every class, but neither is
    /// language-specific.
    ///
    /// An import narrows the first of those rather than widening it: when the
    /// using file says `from two import save`, the definers in any other module
    /// are not candidates at all, so one surviving candidate is an answer even
    /// though the name is defined twice in the change.
    ///
    /// The two groups are NOT interchangeable: it is the *definer's* scope that
    /// decides, so `definer` and `user` must be passed the way round their
    /// names say (pinned by `a_use_side_scope_does_not_block_the_edge`).
    fn resolves(&self, definer: usize, user: usize, s: &str) -> bool {
        let all = self.definers.get(s).map_or(&[][..], Vec::as_slice);
        match self.module_for(user, s) {
            Some(module) => {
                let mut ok = all
                    .iter()
                    .filter(|&&d| module_matches(&self.paths[self.group_file[d]], module));
                // exactly one definer answers to the module the import names
                if ok.next() != Some(&definer) || ok.next().is_some() {
                    return false;
                }
            }
            None if all.len() > 1 => return false,
            None => {}
        }
        !(self.scoped.contains(&(definer, s)) && self.group_file[definer] != self.group_file[user])
    }

    /// Does the using file's own import statement allow `definer` to be where
    /// `s` comes from? The narrow half of `resolves`: it rules out a definer
    /// the source contradicts, and says nothing about ambiguity. The
    /// definer-side provenance wants exactly this and not the rest — a name a
    /// dozen files define is still unambiguous to a reader when the use is
    /// three lines below it.
    fn import_allows(&self, user: usize, definer: usize, s: &str) -> bool {
        match self.module_for(user, s) {
            Some(m) => module_matches(&self.paths[self.group_file[definer]], m),
            None => true,
        }
    }

    /// The module the using group's file imports `s` from, when it says.
    /// `imported_from` is keyed by the name as written *and* by the origin
    /// (see `FileSymbols::of`), so this stays one lookup inside the edge loop.
    fn module_for(&self, user: usize, s: &str) -> Option<&str> {
        let f = self.symbols.get(*self.group_file.get(user)?)?;
        f.imported_from.get(s).and_then(|(_, m)| m.as_deref())
    }
}

/// Does `path` hold the module an import names? Compared on the last segment
/// only: `from pkg.utils import x` is answered by `pkg/utils.py`, `utils.py`
/// or `a/b/utils.ts`, and by nothing called anything else. A module that names
/// no file in the change matches nothing, which is the point — it says the
/// definition is somewhere the reviewer was not shown.
fn module_matches(path: &str, module: &str) -> bool {
    let want = module
        .rsplit(['.', '/', ':'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(module);
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    // `.d.ts` and `.test.js` leave a second extension on the stem
    let stem = stem.split('.').next().unwrap_or(stem);
    !want.is_empty() && want == stem
}

struct RatCtx<'a> {
    groups: &'a [GroupInfo],
    /// symbol → the groups defining / using it, ascending. Provenance asks
    /// "who else touches this name" once per hunk; scanning every group for
    /// the answer is O(hunks × groups).
    definers: &'a HashMap<&'a str, Vec<usize>>,
    users: &'a HashMap<&'a str, Vec<usize>>,
    /// the edge gate — which holds the `scoped` and `definers` tables it
    /// needs — so the rationale layer answers the same question the graph did
    /// rather than rebuilding it per call
    bind: Binding<'a>,
    group_file: &'a [usize],
    group_row: &'a [usize],
    paths: &'a [String],
    symbols: &'a [crate::FileSymbols],
    changed: &'a [crate::FileChanges],
    comment: &'a [bool],
    /// how the hunk moved code across the comment boundary, if it did
    switched: &'a [Option<crate::SideShift>],
    cross_file: bool,
}

impl RatCtx<'_> {
    // groups defining / using `sym`, in group order
    fn definers(&self, sym: &str) -> &[usize] {
        self.definers.get(sym).map_or(&[], Vec::as_slice)
    }
    fn users(&self, sym: &str) -> &[usize] {
        self.users.get(sym).map_or(&[], Vec::as_slice)
    }
    // a candidate group is usable as provenance if it's another group and (when
    // cross_file is off) lives in the same file as the hunk's group
    fn ok(&self, mine: usize, other: usize) -> bool {
        other != mine && (self.cross_file || self.group_file[other] == self.group_file[mine])
    }
    // ...and `other` really is where `sym` comes from, by the same rule the
    // def→use graph uses: the rationale must not name a definition the graph
    // refused to draw an edge to
    /// Provenance from the definition's side: `mine` defines `sym`, `other`
    /// uses it. Only the import gate applies — see `Binding::import_allows`.
    fn used_by(&self, mine: usize, other: usize, sym: &str) -> bool {
        self.ok(mine, other) && self.bind.import_allows(other, mine, sym)
    }
    fn ok_for(&self, mine: usize, other: usize, sym: &str) -> bool {
        self.ok(mine, other) && self.bind.resolves(other, mine, sym)
    }
    // markdown (currently the only prose language): rationale wording says
    // "section" instead of naming a construct kind.
    fn is_prose(&self, file: usize) -> bool {
        crate::lang::for_path(&self.paths[file]).is_some_and(|s| s.prose)
    }
    // #3/#4: verb for a definition hunk. New symbol → "adds"/"adds type"; a
    // pre-existing symbol whose header changed → "changes signature of"/"changes
    // type" (a def-category hunk means the declaration line itself moved).
    fn def_verb(&self, file: usize, sym: &str, is_type: bool) -> &'static str {
        let existed = self
            .symbols
            .get(file)
            .is_some_and(|s| s.old_defs.contains(sym));
        let body_only = self
            .changed
            .get(file)
            .is_some_and(|c| c.body_only.contains(sym));
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

// group (name, src) pairs by shared src, preserving first-seen src order, so
// "adds A, extracted from tests" / "adds B, extracted from tests" collapse
// into one "adds A, B, extracted from tests" fragment instead of repeating.
fn group_by_src<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<(&'a str, Vec<&'a str>)> {
    let mut groups: Vec<(&str, Vec<&str>)> = vec![];
    for &(name, src) in pairs {
        match groups.iter_mut().find(|(s, _)| *s == src) {
            Some(g) => g.1.push(name),
            None => groups.push((src, vec![name])),
        }
    }
    groups
}

// prose wording: "section Install" / "sections Install, Usage" ahead of a
// name list, singular/plural on the count named.
fn prose_noun(count: usize, list: &str) -> String {
    let noun = if count == 1 { "section" } else { "sections" };
    format!("{noun} {list}")
}

fn rationale_for(i: usize, sem: &[&HunkSem], group_idx: &[usize], ctx: &RatCtx) -> String {
    let s = sem[i];
    let mine = group_idx[i];
    let my_file = ctx.group_file[mine];

    // P12.2: noise hunks are skippable — say why, skip semantic wording.
    //
    // An import hunk is noise too, but it usually has something better to say:
    // which import arrived or changed (its own branch, below), or which one
    // left (the removal branch further down). What it has *nothing* better to
    // say about is the empty half of a move — the old line of an import that
    // still exists elsewhere in the file — and for that "formatting only" is
    // exactly right, where "removes 1 line" would claim something left.
    let import_speaks = s.category == Category::Import
        && (!s.new_empty
            || ctx.changed.get(my_file).is_some_and(|c| {
                c.removals
                    .iter()
                    .any(|r| r.row >= s.old_range[0] && r.row <= s.old_range[1])
            }));
    if s.noise && !import_speaks {
        return if crate::lang::is_generated_path(&ctx.paths[my_file]) {
            "generated file".to_string()
        } else {
            "formatting only".to_string()
        };
    }

    // a deleted import has no new-side names to report: its wording comes from
    // the removal path below ("removes import logger"), which knows what left
    if s.category == Category::Import && !s.new_empty {
        if s.imports.is_empty() {
            return "import".to_string();
        }
        // #5 (add side): new import(s) → "adds import"; a touched existing one →
        // "changes import"; the same statement, somewhere else in the file →
        // "moves import", which is what a reordered import block really did
        let all_new = s.imports.iter().all(|im| {
            !ctx.symbols
                .get(my_file)
                .is_some_and(|o| o.old_imports.contains(im))
        });
        let verb = match (s.import_moved, all_new) {
            (true, _) => "moves import",
            (false, true) => "adds import",
            (false, false) => "changes import",
        };
        let names: Vec<&str> = s.imports.iter().map(String::as_str).collect();
        return format!("{verb} {}", name_list(&names));
    }

    // comment-only hunk: wins over every "edits {name}" / body-edit wording
    // below, definition-side included — the changed lines are comment text,
    // not code, however deep inside a known definition they sit. A pure
    // deletion says "removes comment from X" rather than falling through to
    // the generic "removes N lines" further down: knowing it was comment text
    // is strictly more useful than a bare line count.
    // code switched off (or back on) is neither an edit nor a comment change:
    // it is the reviewer-visible act of disabling code, and saying so beats
    // "adds comment", which is what the comment branch below would call it
    if let Some(shift) = ctx.switched.get(i).copied().flatten() {
        let [o0, o1] = s.old_range;
        let verb = match shift {
            crate::SideShift::CommentedOut => "comments out",
            crate::SideShift::Uncommented => "uncomments",
            // comments where code used to be: both halves are worth saying, and
            // "removes N lines" is the vocabulary the deletion branch uses
            crate::SideShift::CodeToComment => {
                let n = o1.saturating_sub(o0) + 1;
                let what = if n == 1 {
                    "1 line".to_string()
                } else {
                    format!("{n} lines")
                };
                let comment = if n == 1 { "a comment" } else { "comments" };
                return match s.enclosing.as_deref() {
                    Some(nm) => {
                        format!("replaces {what} with {comment} in {}", short_container(nm))
                    }
                    None => format!("replaces {what} with {comment}"),
                };
            }
        };
        return match s.enclosing.as_deref() {
            Some(nm) => format!("{verb} code in {}", short_container(nm)),
            None => format!("{verb} code"),
        };
    }
    if ctx.comment.get(i).copied().unwrap_or(false) {
        return comment_rationale(s.old_range, s.new_empty, s.enclosing.as_deref());
    }

    // definition side: report EVERY construct the hunk touches, not just one.
    // Classify each defined symbol (moved-in / renamed / extracted / added /
    // changed), group same-verb symbols, and append provenance once.
    // placeholder names (unnamed closures, `_`-bound throwaways) carry no
    // navigational signal — drop them from the wording and fall through to the
    // enclosing-edit branch when a hunk defines nothing else.
    let real: Vec<&String> = s
        .defines
        .iter()
        .filter(|d| *d != "<anonymous>" && *d != "_")
        .collect();
    if !real.is_empty() {
        let prose = ctx.is_prose(my_file);
        let mut move_pairs: Vec<(&str, &str)> = vec![];
        let mut rename_pairs: Vec<(&str, &str)> = vec![];
        let mut extract_pairs: Vec<(&str, &str)> = vec![];
        // one bucket per verb: same-verb symbols are listed together, so a hunk
        // that adds two functions reads "adds a, b" rather than twice over
        let (mut adds, mut adds_ty, mut edits, mut ch_sig, mut ch_ty): Verbs = Default::default();
        for d in real.iter().copied() {
            if let Some(src) = ctx
                .changed
                .get(my_file)
                .map(|c| &c.moved_in)
                .and_then(|m| m.get(d))
            {
                move_pairs.push((d.as_str(), src.as_str()));
            } else if let Some(old) = ctx
                .changed
                .get(my_file)
                .map(|c| &c.rename)
                .and_then(|m| m.get(d))
            {
                rename_pairs.push((old.as_str(), d.as_str()));
            } else if let Some(src) = ctx
                .changed
                .get(my_file)
                .map(|c| &c.relocated)
                .and_then(|m| m.get(d))
            {
                extract_pairs.push((d.as_str(), src.as_str()));
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
        // several symbols sharing a source collapse into one fragment naming
        // the source once (e.g. "adds A, B, extracted from tests") instead of
        // repeating "extracted from tests" per symbol.
        // …and several *sources* must still name the relation once, or the line
        // repeats "extracted from"/"from" per source — grouped within a source
        // but not across them. Each name then carries its own source inline.
        let extracts = src_frags(&extract_pairs, "adds", ", ", "extracted from");
        let moves = src_frags(&move_pairs, "moves", " ", "from");
        // renames are inherently pairwise (old → new), so grouping by target is
        // meaningless; instead cap the *number* of rename pairs shown, same as
        // name_list caps any other list.
        let renames: Vec<String> = if rename_pairs.is_empty() {
            vec![]
        } else {
            let items: Vec<String> = rename_pairs
                .iter()
                .map(|(old, new)| format!("{old} → {new}"))
                .collect();
            let refs: Vec<&str> = items.iter().map(String::as_str).collect();
            vec![format!("renames {}", name_list(&refs))]
        };
        let mut frags: Vec<String> = vec![];
        frags.extend(extracts);
        frags.extend(renames);
        frags.extend(moves);
        for (verb, items) in [("adds", &adds), ("adds type", &adds_ty), ("edits", &edits)] {
            if items.is_empty() {
                continue;
            }
            let list = name_list(items);
            let label = if prose {
                prose_noun(items.len(), &list)
            } else {
                list
            };
            frags.push(format!("{verb} {label}"));
        }
        // "changes signature of f" only for something that *has* a signature.
        // A value touched on its own declaration line — a cmake `set()`, a
        // make variable, a yaml key, a rust `const` — simply changes. The kind
        // comes from the hunk's own symbols; a name with no symbol entry keeps
        // the weaker wording rather than claiming a signature it may not have.
        let (sig, plain): (Vec<&str>, Vec<&str>) = ch_sig.iter().partition(|d| {
            s.symbols
                .iter()
                .find(|sy| sy.name.as_str() == **d)
                .is_some_and(|sy| crate::lang::has_signature(&sy.kind))
        });
        if !sig.is_empty() {
            frags.push(format!("changes signature of {}", name_list(&sig)));
        }
        if !plain.is_empty() {
            frags.push(format!("changes {}", name_list(&plain)));
        }
        if !ch_ty.is_empty() {
            frags.push(format!("changes type {}", name_list(&ch_ty)));
        }
        let mut out = join_frags(&frags);
        // provenance: a defined symbol used by another group. Name it only when
        // several constructs are listed (otherwise "used by X" is unambiguous).
        // a definer the using file's import contradicts must not claim the use
        // (`used in caller.py` when the caller imports the name from elsewhere).
        // Only that: the graph's ambiguity rule does not belong here, and
        // applying it dropped 274 provenance phrases in one corpus repo.
        if let Some((d, b)) = real.iter().find_map(|d| {
            ctx.users(d)
                .iter()
                .copied()
                .find(|&b| ctx.used_by(mine, b, d))
                .map(|b| ((*d).clone(), b))
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
        // P17 composes with def-side wording rather than being suppressed by
        // it: a hunk that both defines a real symbol and introduces bindings
        // (e.g. a function plus the module-level constants beside it) must
        // name both, or the constants silently vanish from the rationale.
        // The def-side prefix is already list-capped, but a long qualified
        // scope name (nested test class/function) can still push the
        // composed line past the one-line bound — degrade the same way
        // name_list itself does (a count instead of the full listing)
        // rather than let the line grow unboundedly.
        if let Some(r) = binding_rationale(s, &ctx.symbols[my_file].old_locals) {
            if out.chars().count() + 2 + r.chars().count() <= MAX_RATIONALE {
                out += "; ";
                out += &r;
            } else {
                let n = s
                    .bindings
                    .iter()
                    .filter(|b| !ctx.symbols[my_file].old_locals.contains(&b.name))
                    .count();
                let summary = format!("; +{n} more binding{}", if n == 1 { "" } else { "s" });
                if out.chars().count() + summary.chars().count() <= MAX_RATIONALE {
                    out += &summary;
                }
            }
        }
        return out;
    }

    // use side: a symbol used here that some other group defines (this change)
    if !s.uses.is_empty() {
        // #6: from a test file, prefer a symbol defined in a non-test file
        if is_test_path(&ctx.paths[my_file]) {
            if let Some((u, b)) = s.uses.iter().find_map(|u| {
                ctx.definers(u)
                    .iter()
                    .copied()
                    .find(|&b| {
                        ctx.ok_for(mine, b, u)
                            && ctx.group_file[b] != my_file
                            && !is_test_path(&ctx.paths[ctx.group_file[b]])
                    })
                    .map(|b| (u.clone(), b))
            }) {
                return format!("tests {u} ({})", ctx.paths[ctx.group_file[b]]);
            }
        }
        if let Some((u, b)) = s.uses.iter().find_map(|u| {
            ctx.definers(u)
                .iter()
                .copied()
                .find(|&b| ctx.ok_for(mine, b, u))
                .map(|b| (u.clone(), b))
        }) {
            return ctx.use_of_phrase(&u, mine, b);
        }
        // P17: a local binding this hunk introduces beats the bare "edits
        // {enclosing}" fallback — naming the binding and where it's used (or
        // that it isn't) is more useful than restating the enclosing def.
        if let Some(r) = binding_rationale(s, &ctx.symbols[my_file].old_locals) {
            return r;
        }
        if let Some(nm) = &s.enclosing {
            return format!("edits {}", short_container(nm));
        }
        let names: Vec<&str> = s.uses.iter().map(String::as_str).collect();
        return format!("uses {}", name_list(&names));
    }

    if let Some(r) = binding_rationale(s, &ctx.symbols[my_file].old_locals) {
        return r;
    }

    if let Some(nm) = &s.enclosing {
        // "section" is the word for a prose *definition*; a region already
        // names what it is ("preamble", "front matter", "#ifdef X"), so
        // prefixing it would read as "edits section front matter"
        let is_section = ctx.is_prose(my_file) && s.enclosing_kind.is_none();
        let nm = short_container(nm);
        return if is_section {
            format!("edits {}", prose_noun(1, &nm))
        } else {
            format!("edits {nm}")
        };
    }
    // #5/#7 removal: a deletion hunk whose old lines held a removed symbol
    let [o0, o1] = s.old_range;
    if let Some(label) = ctx.changed.get(my_file).and_then(|c| {
        c.removals
            .iter()
            .find(|r| r.row >= o0 && r.row <= o1)
            .map(removal_phrase)
    }) {
        return label;
    }
    // a pure deletion of body lines (no tracked def/import removed) — name the
    // size so a large removal isn't hidden behind a blank "change". Comment
    // deletions never reach here: the comment-only check above already claims
    // them.
    if s.new_empty && o1 >= o0 {
        let n = o1 - o0 + 1;
        return format!("removes {n} line{}", if n == 1 { "" } else { "s" });
    }
    // nothing above found a construct to name. Say what the hunk did to the
    // file — a direction and a size — rather than the bare word "change",
    // which told a reviewer nothing at all. Reached by a hunk the grammar
    // recognises nothing in: a `#define` (not a definition since macros left
    // `defines`), a continuation line inside a shell command, the prose ahead
    // of an added document's first heading.
    let n = s.new_len.max(1);
    let plural = if n == 1 { "" } else { "s" };
    if o1 < o0 {
        format!("adds {n} line{plural}")
    } else {
        format!("edits {n} line{plural}")
    }
}

// comment-only wording, verb/preposition matched the way P15's detail_phrases
// pairs them (adds…to, removes…from); "edits" keeps its existing vocabulary
// but takes "in" for the same reason.
fn comment_rationale(old_range: [usize; 2], new_empty: bool, enclosing: Option<&str>) -> String {
    let [o0, o1] = old_range;
    let (verb, prep) = if o0 > o1 {
        ("adds", "to")
    } else if new_empty {
        ("removes", "from")
    } else {
        ("edits", "in")
    };
    match enclosing {
        Some(nm) => format!("{verb} comment {prep} {}", short_container(nm)),
        None => format!("{verb} comment"),
    }
}

// One-line rendering for a "used" group of bindings. The rationale names the
// bindings and counts their uses; the positions themselves ride on
// `HunkOut::uses_at`, where a consumer can mark them in the code rather than
// make the reader carry eighteen line numbers across two panes.
fn used_frag(items: &[(&str, &[usize])], prefix: &str) -> String {
    let strs: Vec<String> = items
        .iter()
        .map(|(name, uses)| {
            let n = uses.len();
            format!("{name} ({n} use{})", if n == 1 { "" } else { "s" })
        })
        .collect();
    let refs: Vec<&str> = strs.iter().map(String::as_str).collect();
    format!("adds {prefix}{}", name_list(&refs))
}

// P17: "where is this binding used?" wording for a hunk that introduces one
// or more local-variable bindings (see `lang::LangSpec::locals`). ordo
// parses one file, so an absence of uses is scoped honestly to what it
// actually checked — the enclosing function for a function-local, the whole
// file for a module/script-level name — rather than an unsupportable
// "unused" claim (that's ruff/clippy's job, with their suppression
// conventions).
//
// Names already locally bound somewhere in the old file are excluded: a
// hunk that only edits an existing variable's value (`x = 1` → `x = 2`)
// isn't introducing `x` — that stays the bare "edits {enclosing}" wording.
//
// Several bindings sharing the same outcome (same scope, same uses-or-not)
// collapse into one fragment naming the outcome once — the same
// `group_by_src` idea used for extraction provenance above — so a block of
// module-level constants doesn't repeat "no uses in this file — check other
// files" once per name. `bindings` (and therefore each group) is walked in
// name order, so the grouping and every `name_list` it feeds are
// deterministic without touching a HashMap.
/// The bindings a hunk actually introduces — what both the rationale and
/// `HunkOut::uses_at` speak about, so the gutter never marks a name the
/// rationale refuses to name.
///
/// Names already locally bound somewhere in the old file are excluded: a hunk
/// that only edits an existing variable's value (`x = 1` → `x = 2`) isn't
/// introducing `x`. A binding inside a def this same hunk introduces is that
/// def's own implementation detail — the rationale already says "adds
/// _wrap_fan_deg", so naming the locals it was born with adds nothing. Same
/// rule the P15 detail layer applies to the members of a wholly new container.
pub(crate) fn introduced_bindings<'a>(
    s: &'a HunkSem,
    old_locals: &'a HashSet<String>,
) -> impl Iterator<Item = &'a BindingUse> {
    let born_here = move |b: &BindingUse| {
        b.scope.as_deref().is_some_and(|sc| {
            s.defines
                .iter()
                .any(|d| sc == d || sc.starts_with(&format!("{d}.")))
        })
    };
    s.bindings
        .iter()
        .filter(move |b| !old_locals.contains(&b.name) && !born_here(b))
}

fn binding_rationale(s: &HunkSem, old_locals: &HashSet<String>) -> Option<String> {
    let mut items: Vec<&BindingUse> = introduced_bindings(s, old_locals).collect();
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));

    let mut no_uses_scoped: Vec<(&str, Vec<&str>)> = vec![]; // (scope, names), scope-sorted below
    let mut no_uses_file: Vec<&str> = vec![];
    let mut used_scoped: Vec<(&str, &[usize])> = vec![];
    let mut used_file: Vec<(&str, &[usize])> = vec![];
    for b in &items {
        match (&b.scope, b.uses.is_empty()) {
            (Some(scope), true) => match no_uses_scoped.iter_mut().find(|(sc, _)| sc == scope) {
                Some((_, names)) => names.push(&b.name),
                None => no_uses_scoped.push((scope.as_str(), vec![&b.name])),
            },
            (None, true) => no_uses_file.push(&b.name),
            (Some(_), false) => used_scoped.push((&b.name, &b.uses)),
            (None, false) => used_file.push((&b.name, &b.uses)),
        }
    }
    no_uses_scoped.sort_by_key(|(scope, _)| *scope);

    let mut frags: Vec<String> = vec![];
    // One scope reads naturally; several must still say the phrase once, or the
    // line repeats "no uses in ..." per scope — grouped per scope, but not
    // grouped across them.
    match no_uses_scoped.len() {
        0 => {}
        1 => {
            let (scope, names) = &no_uses_scoped[0];
            frags.push(format!(
                "adds local {}, no uses in {scope} — check nested scopes",
                name_list(names)
            ));
        }
        _ => {
            let mut each: Vec<String> = no_uses_scoped
                .iter()
                .flat_map(|(scope, names)| names.iter().map(move |n| format!("{n} (in {scope})")))
                .collect();
            each.sort();
            let refs: Vec<&str> = each.iter().map(String::as_str).collect();
            frags.push(format!(
                "adds local {}, no uses in their own scopes — check nested scopes",
                name_list(&refs)
            ));
        }
    }
    if !no_uses_file.is_empty() {
        // scoped fragments above already say "no uses in {scope}" — when both
        // kinds land in the same rationale (P17 composing with def-side
        // wording), repeating that exact phrase at file scope reads as the
        // per-symbol-fragment defect rationale_bounds.rs guards against, even
        // though it's really two distinct groups. Reword only in that mixed
        // case; the lone-fragment wording (no scoped group alongside it)
        // stays as-is.
        frags.push(if no_uses_scoped.is_empty() {
            format!(
                "adds {}, no uses in this file — check other files",
                name_list(&no_uses_file)
            )
        } else {
            format!(
                "adds {}, unused elsewhere in this file — check other files",
                name_list(&no_uses_file)
            )
        });
    }
    if !used_scoped.is_empty() {
        frags.push(used_frag(&used_scoped, "local "));
    }
    if !used_file.is_empty() {
        frags.push(used_frag(&used_file, ""));
    }
    (!frags.is_empty()).then(|| join_frags(&frags))
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

// ---------------------------------------------------------- detail layer

/// What a hunk did to the named members of its container(s). New-side members
/// come from `analyze`, each already carrying its own container (P15's
/// attribution fix — see `extract::member_container`); the old side is
/// matched by the hunk's old line range. A name present on both sides of the
/// *same* container was edited; on one side only, added or removed. Members
/// under different containers (e.g. two distinct `add_argument(...)` calls
/// touched by one hunk) are never compared against each other.
pub(crate) fn detail_phrases(
    s: &HunkSem,
    h: &RawHunk,
    old_members: &[extract::MemberRow],
    prose: bool,
    nests: bool,
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
    //
    // Not for a language where a definition is *also* a member of the one
    // above it — a prose section, a config key. There `None` means top level,
    // and the fallback reports a **sibling** as the parent: two keys side by
    // side read as `adds two to one`.
    let clean =
        |c: String| (!c.split('.').any(|seg| seg == "<anonymous>" || seg == "_")).then_some(c);
    let fallback = if nests { None } else { s.enclosing.clone() };
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
    let short: Vec<String> = names.iter().map(|n| short_container(n)).collect();
    if short.len() <= SHOWN {
        return short.join(", ");
    }
    format!(
        "{}, and {} more",
        short[..SHOWN].join(", "),
        short.len() - SHOWN
    )
}
