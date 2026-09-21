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

/// (from global idx, to global idx, why)
pub type Edge = (usize, usize, String);

pub struct OrderedAll {
    /// global hunk index -> (file, local hunk index)
    pub coord: Vec<(usize, usize)>,
    /// global reading order (global indices)
    pub perm: Vec<usize>,
    pub group_idx: Vec<usize>,
    pub groups: Vec<GroupInfo>,
    pub edges: Vec<Edge>,
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

/// A container name, shortened to what a rationale can afford.
///
/// `enclosing` carries the full path because a consumer navigating to the hunk
/// wants it, but a rationale is one line: a nested test label
/// (`describe "compiler: transform v-bind" > it "errors on a bad argument"`)
/// spends the whole budget on the container and leaves none for what changed.
/// The innermost segment identifies it; anything still very long is elided
/// rather than allowed to crowd out the rest of the sentence.
/// The last segment of a qualified container name: `old_defs`, `body_only`
/// and friends are keyed by bare name, `enclosing` is scope-qualified with
/// whichever separator the language uses.
pub(crate) fn bare_name(name: &str) -> &str {
    name.rsplit(['.', ':']).next().unwrap_or(name)
}

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

/// A group's sort key: rule priority (descending), then whether it is prose
/// (see `docs_rank`), then file, then source row, then the group index — which
/// makes every key distinct, so a sorted set of them doubles as the ready queue
/// in the topological sort.
type GroupKey = (std::cmp::Reverse<i64>, u8, usize, usize, usize);

/// Where a file's hunks sort among the ones nothing forces an order on.
///
/// A doc describes code, so a reviewer reads the code first and judges the doc
/// against it. The engine cannot see that relation — prose carries no symbols,
/// so a markdown hunk never has an incoming edge and lands wherever the file
/// order puts it, which is alphabetical and so usually first. This is the one
/// place that says otherwise.
///
/// Only prose moves. A data or config file (a schema, a lockfile, a
/// `package.json`) frequently drives the code around it and keeps its place.
fn docs_rank(path: &str, docs_last: bool) -> u8 {
    let prose = crate::lang::for_path(path).is_some_and(|s| s.prose);
    u8::from(prose && docs_last)
}

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
    docs_last: bool,
) -> OrderedAll {
    let FileFacts { symbols, changed } = *facts;
    let flat = flatten(files);
    let (groups, group_idx) = group_hunks(&flat);
    let (gdef, guse) = group_symbols(&flat, &groups, &group_idx, symbols);
    // `users`/`definers` invert the per-group sets once (symbol → the groups
    // using / defining it, ascending), so a group's defs look up their users
    // directly instead of scanning every other group: the corpus reaches ~15k
    // hunks in one repo, where g² does not hold.
    let users = invert(&guse);
    let definers = invert(&gdef);
    let scoped = scoped_defs(&flat, &group_idx);
    // per-group file + source position: fixed by membership, so computed once
    // rather than re-walking every member hunk on every sort comparison and
    // every rationale below
    let group_file: Vec<usize> = groups
        .iter()
        .map(|gr| flat.coord[gr.members[0]].0)
        .collect();
    let group_row: Vec<usize> = groups
        .iter()
        .map(|gr| {
            gr.members
                .iter()
                .map(|&i| flat.sem[i].start_row)
                .min()
                .unwrap_or(0)
        })
        .collect();
    let bind = Binding {
        definers: &definers,
        scoped: &scoped,
        group_file: &group_file,
        paths,
        symbols,
    };
    let (mut edges, gedges) = def_use_edges(&groups, &flat.sem, &gdef, &users, &bind, cross_file);
    let (contain_edges, contain_gedges) = containment_edges(&flat, &groups, &group_idx);
    edges.extend(contain_edges);

    let docs: Vec<u8> = group_file
        .iter()
        .map(|&f| docs_rank(&paths[f], docs_last))
        .collect();
    let keys = GroupKeys {
        docs: &docs,
        file: &group_file,
        row: &group_row,
    };
    let group_order = order_groups(strategy, &groups, &flat.sem, &gedges, &keys);
    // ---- flatten groups to a global hunk permutation ----
    let mut perm = vec![];
    for &gi in &group_order {
        let mut mem = groups[gi].members.clone();
        mem.sort_by_key(|&i| (flat.coord[i].0, flat.sem[i].start_row, i));
        perm.extend(mem);
    }

    let ctx = RatCtx {
        groups: &groups,
        definers: &definers,
        users: &users,
        scoped: &scoped,
        group_file: &group_file,
        group_row: &group_row,
        paths,
        symbols,
        changed,
        comment: &flat.comment,
        switched: &flat.switched,
        cross_file,
    };
    let rationale = (0..flat.sem.len())
        .map(|i| clamp_rationale(rationale_for(i, &flat.sem, &group_idx, &ctx)))
        .collect();
    let clusters = components(&groups, gedges.iter().chain(&contain_gedges));

    OrderedAll {
        coord: flat.coord,
        perm,
        group_idx,
        groups,
        edges,
        rationale,
        clusters,
    }
}

/// Every hunk of the change in one list, with what the caller worked out per
/// hunk, so the rest of the ordering indexes by one global hunk number.
struct Flat<'a> {
    /// global hunk index -> (file, local hunk index)
    coord: Vec<(usize, usize)>,
    sem: Vec<&'a HunkSem>,
    comment: Vec<bool>,
    switched: Vec<Option<crate::SideShift>>,
}

fn flatten(files: &[crate::PerFileHunks]) -> Flat<'_> {
    let (mut coord, mut sem, mut comment, mut switched) = (vec![], vec![], vec![], vec![]);
    for (fi, f) in files.iter().enumerate() {
        for (li, s) in f.sem.iter().enumerate() {
            coord.push((fi, li));
            sem.push(s);
            comment.push(f.comment.get(li).copied().unwrap_or(false));
            switched.push(f.switched.get(li).copied().flatten());
        }
    }
    Flat {
        coord,
        sem,
        comment,
        switched,
    }
}

/// P1: group by (file, enclosing definition); top-level hunks (no enclosing
/// definition) share one group per file — module constants, imports, exports
/// and similar file-scope hunks all live in the same scope, so they group the
/// same way two hunks in the same function do. Returns the groups in
/// first-seen order and each hunk's group.
fn group_hunks(flat: &Flat) -> (Vec<GroupInfo>, Vec<usize>) {
    let mut idx_of_key: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<GroupInfo> = vec![];
    let mut group_idx = vec![0usize; flat.sem.len()];
    for (i, s) in flat.sem.iter().enumerate() {
        let fi = flat.coord[i].0;
        // imports group together, apart from the module-level code they sit
        // among: they are bookkeeping, they read as a block, and mixing them
        // into the top-level group would drag that group up to the first import
        // line — which cost the `file` strategy its positional promise.
        let import = s.category == Category::Import;
        let (key, reason) = match (&s.enclosing, import) {
            (_, true) => (
                format!("{fi}\u{0}imports"),
                "same scope: imports".to_string(),
            ),
            (Some(nm), _) => (
                format!("{fi}\u{0}def:{nm}"),
                format!("same definition: {nm}"),
            ),
            (None, _) => (format!("{fi}\u{0}top"), "same scope: top-level".to_string()),
        };
        let gi = *idx_of_key.entry(key).or_insert_with(|| {
            groups.push(GroupInfo {
                reason,
                members: vec![],
            });
            groups.len() - 1
        });
        group_idx[i] = gi;
        groups[gi].members.push(i);
    }
    (groups, group_idx)
}

/// What each group defines and what it uses, as symbol sets.
fn group_symbols(
    flat: &Flat,
    groups: &[GroupInfo],
    group_idx: &[usize],
    symbols: &[crate::FileSymbols],
) -> (Vec<HashSet<String>>, Vec<HashSet<String>>) {
    let mut gdef: Vec<HashSet<String>> = vec![HashSet::new(); groups.len()];
    let mut guse: Vec<HashSet<String>> = vec![HashSet::new(); groups.len()];
    for (i, s) in flat.sem.iter().enumerate() {
        let gi = group_idx[i];
        gdef[gi].extend(s.defines.iter().cloned());
        for u in &s.uses {
            guse[gi].insert(u.clone());
            // `from lib import helper as h` then `h()`: the definition is
            // called `helper` in the file that has it, so the alias alone can
            // never match one. Record the origin too, and leave the hunk's own
            // `uses` saying what the source says.
            if let Some((origin, _)) = symbols
                .get(flat.coord[i].0)
                .and_then(|f| f.imported_from.get(u.as_str()))
            {
                guse[gi].insert(origin.clone());
            }
        }
    }
    (gdef, guse)
}

/// symbol → the groups whose set holds it, ascending
fn invert(sets: &[HashSet<String>]) -> HashMap<&str, Vec<usize>> {
    let mut by: HashMap<&str, Vec<usize>> = HashMap::new();
    for (gi, set) in sets.iter().enumerate() {
        for s in set {
            by.entry(s.as_str()).or_default().push(gi);
        }
    }
    by
}

/// (group, symbol) pairs the symbol is declared *inside another definition*
/// in — a class member, a name in a namespace — rather than at file scope.
/// Built from `symbols`, which is `defines` minus the names that are not
/// declarations of this file's own (an import, a test label): those name
/// something whose scope lives elsewhere, and treating them as file-scope is
/// what lets an imported name keep resolving across files.
fn scoped_defs<'a>(flat: &Flat<'a>, group_idx: &[usize]) -> HashSet<(usize, &'a str)> {
    let mut scoped = HashSet::new();
    for (i, s) in flat.sem.iter().enumerate() {
        for sym in &s.symbols {
            if sym.scope.is_some() {
                scoped.insert((group_idx[i], sym.name.as_str()));
            }
        }
    }
    scoped
}

/// The first member hunk `pred` accepts, else the group's first hunk: where an
/// edge attaches when the group is what the graph knows.
fn member_where(group: &GroupInfo, pred: impl Fn(usize) -> bool) -> usize {
    group
        .members
        .iter()
        .copied()
        .find(|&i| pred(i))
        .unwrap_or(group.members[0])
}

/// P2: def→use edges between groups, one per (definer, user) pair named by
/// the definer's first symbol the user takes — cross-file only when enabled
/// (P4). Returns the hunk-level edges (for output) and the group-level ones
/// (for the sort and the clusters).
fn def_use_edges(
    groups: &[GroupInfo],
    sem: &[&HunkSem],
    gdef: &[HashSet<String>],
    users: &HashMap<&str, Vec<usize>>,
    bind: &Binding,
    cross_file: bool,
) -> (Vec<Edge>, Vec<(usize, usize)>) {
    let mut edges = vec![];
    let mut gedges = vec![];
    for a in 0..groups.len() {
        let mut defs_a: Vec<&String> = gdef[a].iter().collect();
        defs_a.sort();
        // one edge per (a, b), named by a's first symbol that b uses
        let mut reached: BTreeMap<usize, &String> = BTreeMap::new();
        for s in &defs_a {
            let takers = users.get(s.as_str()).into_iter().flatten().copied();
            for b in takers.filter(|&b| bind.edge_allowed(a, b, s, cross_file)) {
                reached.entry(b).or_insert(s);
            }
        }
        for (b, s) in reached {
            let from_h = member_where(&groups[a], |i| sem[i].defines.contains(s));
            let to_h = member_where(&groups[b], |i| sem[i].uses.contains(s));
            edges.push((from_h, to_h, format!("def→use: {s}")));
            gedges.push((a, b));
        }
    }
    (edges, gedges)
}

/// Containment edges: a def introduced here whose `scope` names another
/// group's own enclosing definition — i.e. this def lives nested inside that
/// other def, and both changed in this diff. Barnett et al., ICSE 2015
/// (ClusterChanges) model containment as a distinct edge in the same graph as
/// def→use, specifically so it feeds connected components without flattening
/// the grouping key to the outermost def — two unrelated sibling methods of
/// one class never share a group just for sharing a container, only a def and
/// the parent def it nests inside do.
///
/// Cluster membership only — the group pairs
/// returned here stay out of Kahn's indegree, which drives P2 reading order: a
/// containment edge that also gated the topo sort could conflict with a
/// def→use edge running the other way (a nested helper the parent def calls),
/// forcing a cycle-break where none exists today; measured against the
/// corpus, clustering the two together already recovers the "these changed
/// together" signal without risking that.
fn containment_edges(
    flat: &Flat,
    groups: &[GroupInfo],
    group_idx: &[usize],
) -> (Vec<Edge>, Vec<(usize, usize)>) {
    let mut def_group: HashMap<(usize, &str), usize> = HashMap::new();
    for (i, s) in flat.sem.iter().enumerate() {
        if let Some(nm) = &s.enclosing {
            def_group
                .entry((flat.coord[i].0, nm.as_str()))
                .or_insert(group_idx[i]);
        }
    }
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut edges = vec![];
    let mut contain_gedges = vec![];
    for (i, s) in flat.sem.iter().enumerate() {
        let cgi = group_idx[i];
        for sym in &s.symbols {
            let Some(sc) = &sym.scope else { continue };
            let Some(&pgi) = def_group.get(&(flat.coord[i].0, sc.as_str())) else {
                continue;
            };
            if pgi == cgi || !seen.insert((pgi, cgi)) {
                continue;
            }
            let from_h = member_where(&groups[pgi], |h| {
                flat.sem[h].enclosing.as_deref() == Some(sc.as_str())
            });
            edges.push((from_h, i, format!("encloses: {}", sym.name)));
            contain_gedges.push((pgi, cgi));
        }
    }
    (edges, contain_gedges)
}

/// Per-group facts every strategy sorts on. Every strategy answers to
/// `docs` (from `docs_rank`): it says where a doc belongs relative to code,
/// which is as true of a file-order read as of a comprehension one.
struct GroupKeys<'a> {
    docs: &'a [u8],
    file: &'a [usize],
    row: &'a [usize],
}

/// The groups in reading order, per strategy.
fn order_groups(
    strategy: Strategy,
    groups: &[GroupInfo],
    sem: &[&HunkSem],
    gedges: &[(usize, usize)],
    k: &GroupKeys,
) -> Vec<usize> {
    let mut v: Vec<usize> = (0..groups.len()).collect();
    match strategy {
        Strategy::File => v.sort_by_key(|&gi| (k.docs[gi], k.file[gi], k.row[gi], gi)),
        Strategy::DefsFirst => {
            let gcat_v: Vec<u8> = groups
                .iter()
                .map(|gr| {
                    gr.members
                        .iter()
                        .map(|&i| cat_rank(sem[i].category))
                        .min()
                        .unwrap_or(2)
                })
                .collect();
            v.sort_by_key(|&gi| (k.docs[gi], gcat_v[gi], k.file[gi], k.row[gi], gi));
        }
        Strategy::Comprehension => {
            // A group's rule priority is the highest any of its hunks carries.
            // It enters the key *after* the import rank and *before* file
            // position, so it replaces the positional tiebreaker among groups
            // the graph has already freed — never the graph itself. A rule
            // cannot pull a use ahead of its definition.
            // Imports sort where they live, not first. Ranking them ahead of
            // everything made sense while they were dropped and never seen; now
            // that they are visible noise, leading with forty dimmed rows
            // buries the change they came with. A reviewer who does want them
            // first can say so with a rule (`priority`).
            let key_v: Vec<GroupKey> = groups
                .iter()
                .enumerate()
                .map(|(gi, gr)| {
                    let prio = gr
                        .members
                        .iter()
                        .map(|&i| sem[i].priority)
                        .max()
                        .unwrap_or(0);
                    (
                        std::cmp::Reverse(prio),
                        k.docs[gi],
                        k.file[gi],
                        k.row[gi],
                        gi,
                    )
                })
                .collect();
            v = topo_order(&key_v, gedges);
        }
    }
    v
}

/// Kahn's topological sort over `gedges`, lowest key first among the freed
/// groups; on a cycle, the lowest key still standing — the same deterministic
/// tiebreak applied to a set the graph never released.
///
/// The ready set is carried across iterations rather than rebuilt by scanning
/// every group each round: a group joins it exactly when its indegree reaches
/// zero. A `GroupKey` ends in the group index, so keys are distinct and a
/// sorted set of them is a priority queue that also supports the removals the
/// cycle break needs. Same reason `users` exists — the corpus reaches ~15k
/// hunks in one repo, where g² does not hold. Every round removes the pick
/// from `left` (`ready` only ever holds members of `left`), so the loop ends
/// after exactly g rounds, cycle or not.
fn topo_order(key_v: &[GroupKey], gedges: &[(usize, usize)]) -> Vec<usize> {
    let g = key_v.len();
    let mut indeg = vec![0usize; g];
    let mut succ: Vec<Vec<usize>> = vec![vec![]; g];
    for &(a, b) in gedges {
        succ[a].push(b);
        indeg[b] += 1;
    }
    let mut ready: BTreeSet<GroupKey> = (0..g)
        .filter(|&gi| indeg[gi] == 0)
        .map(|gi| key_v[gi])
        .collect();
    let mut left: BTreeSet<GroupKey> = key_v.iter().copied().collect();
    let mut order = Vec::with_capacity(g);
    while let Some(key) = ready.first().or_else(|| left.first()).copied() {
        let pick = key.4;
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

/// P12.3: connected components of the group graph = independent parts, each
/// a sorted list of global hunk indices, in order of first hunk. Containment
/// edges join components here too (a nested def and its parent def are part
/// of the same change even with no def→use edge between them) without joining
/// the groups themselves.
fn components<'a>(
    groups: &[GroupInfo],
    links: impl Iterator<Item = &'a (usize, usize)>,
) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..groups.len()).collect();
    for &(a, b) in links {
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
    let mut parts: Vec<Vec<usize>> = by_root.into_values().collect();
    for c in &mut parts {
        c.sort_unstable();
    }
    parts.sort_by_key(|c| c.first().copied().unwrap_or(0));
    parts
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
#[derive(Clone, Copy)]
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

    /// The whole gate for a def→use edge from `definer` to `user`: not the
    /// same group, same file unless cross-file edges are on, and `resolves`.
    fn edge_allowed(&self, definer: usize, user: usize, s: &str, cross_file: bool) -> bool {
        definer != user
            && (cross_file || self.group_file[definer] == self.group_file[user])
            && self.resolves(definer, user, s)
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
    /// see `scoped_defs`; with `definers`, what `Binding` needs so the
    /// rationale layer answers the same question the graph did
    scoped: &'a HashSet<(usize, &'a str)>,
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

impl<'a> RatCtx<'a> {
    /// the edge gate, over the same tables the graph used
    fn bind(&self) -> Binding<'a> {
        Binding {
            definers: self.definers,
            scoped: self.scoped,
            group_file: self.group_file,
            paths: self.paths,
            symbols: self.symbols,
        }
    }
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
        self.ok(mine, other) && self.bind().import_allows(other, mine, sym)
    }
    fn ok_for(&self, mine: usize, other: usize, sym: &str) -> bool {
        self.ok(mine, other) && self.bind().resolves(other, mine, sym)
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
    // a hunk that stays above the body of an existing callable changed its
    // signature — a parameter, a return annotation, a storage class — even
    // though the `def` line itself is outside the hunk, so `defines` is empty
    // and the wording would otherwise fall to "edits f"
    fn header_edit(&self, file: usize, s: &HunkSem) -> Option<String> {
        if !s.in_header {
            return None;
        }
        let nm = s.enclosing.as_deref()?;
        let existed = self
            .symbols
            .get(file)
            .is_some_and(|f| f.old_defs.contains(bare_name(nm)));
        existed.then(|| format!("changes signature of {}", short_container(nm)))
    }
    // a file-scope hunk that touches module-level bindings: "changes STRING"
    // for a constant whose declaration line changed (an annotation, a value),
    // "adds X" for a new one. What the hunk did to a name, where "uses Final,
    // STRING" only listed the identifiers on the line.
    fn file_bind_phrase(&self, file: usize, s: &HunkSem) -> Option<String> {
        if s.enclosing.is_some() || s.new_empty {
            return None;
        }
        let f = self.symbols.get(file)?;
        let rows = s.start_row + 1..=s.start_row + s.new_len;
        let (mut changed, mut added): (Vec<&str>, Vec<&str>) = (vec![], vec![]);
        for (nm, _) in f.new_bind_rows.iter().filter(|(_, row)| rows.contains(row)) {
            let bucket = if f.old_binds.iter().any(|(o, _)| o == nm) {
                &mut changed
            } else {
                &mut added
            };
            if !bucket.contains(&nm.as_str()) {
                bucket.push(nm);
            }
        }
        let frags: Vec<String> = [("adds", added), ("changes", changed)]
            .iter()
            .filter(|(_, names)| !names.is_empty())
            .map(|(verb, names)| format!("{verb} {}", name_list(names)))
            .collect();
        (!frags.is_empty()).then(|| frags.join("; "))
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

/// One line saying what hunk `i` did. Each branch family below answers for
/// the hunks it recognises and passes on the rest; the order here is the
/// precedence: noise, then imports, then commented-out or comment-only text,
/// then what the hunk defines, then what it uses (which falls back to the
/// scope wording on its own), then what a hunk using nothing did within its
/// scope, and last the shape of the change itself.
fn rationale_for(i: usize, sem: &[&HunkSem], group_idx: &[usize], ctx: &RatCtx) -> String {
    let s = sem[i];
    let mine = group_idx[i];
    let my_file = ctx.group_file[mine];
    noise_rationale(s, ctx, my_file)
        .or_else(|| import_rationale(s, ctx, my_file))
        .or_else(|| switch_rationale(s, ctx.switched.get(i).copied().flatten()))
        .or_else(|| {
            let is_comment = ctx.comment.get(i).copied().unwrap_or(false);
            is_comment.then(|| comment_rationale(s.old_range, s.new_empty, s.enclosing.as_deref()))
        })
        .or_else(|| def_side_rationale(s, sem, mine, ctx))
        .or_else(|| use_side_rationale(s, mine, ctx))
        .or_else(|| scope_rationale(s, ctx, my_file))
        .unwrap_or_else(|| shape_rationale(s, ctx, my_file))
}

// P12.2: noise hunks are skippable — say why, skip semantic wording.
//
// An import hunk is noise too, but it usually has something better to say:
// which import arrived or changed (`import_rationale`), or which one left
// (there, or the removal branch in `shape_rationale`). What it has *nothing*
// better to say about is the empty half of a move — the old line of an import
// that still exists elsewhere in the file — and for that "formatting only" is
// exactly right, where "removes 1 line" would claim something left.
fn noise_rationale(s: &HunkSem, ctx: &RatCtx, my_file: usize) -> Option<String> {
    let import_speaks = s.category == Category::Import
        && (!s.new_empty
            || !s.imports.is_empty()
            || ctx.changed.get(my_file).is_some_and(|c| {
                c.removals
                    .iter()
                    .any(|r| r.row >= s.old_range[0] && r.row <= s.old_range[1])
            }));
    if !s.noise || import_speaks {
        return None;
    }
    Some(if crate::lang::is_generated_path(&ctx.paths[my_file]) {
        "generated file".to_string()
    } else {
        "formatting only".to_string()
    })
}

// #5 (add side): new import(s) → "adds import"; a touched existing one →
// "changes import"; the same statement, somewhere else in the file → "moves
// import", which is what a reordered import block really did. A deleted
// import names what left when `classify_imports` could tell (the dropped
// member of a multi-line import, matched by name); otherwise its wording comes
// from the removal path in `shape_rationale` ("removes import logger").
fn import_rationale(s: &HunkSem, ctx: &RatCtx, my_file: usize) -> Option<String> {
    if s.category != Category::Import {
        return None;
    }
    if s.new_empty && s.imports.is_empty() {
        return None;
    }
    if s.imports.is_empty() {
        return Some("import".to_string());
    }
    let names: Vec<&str> = s.imports.iter().map(String::as_str).collect();
    if s.new_empty {
        return Some(format!("removes import {}", name_list(&names)));
    }
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
    Some(format!("{verb} {}", name_list(&names)))
}

// code switched off (or back on) is neither an edit nor a comment change: it
// is the reviewer-visible act of disabling code, and saying so beats "adds
// comment", which is what the comment branch would call it
fn switch_rationale(s: &HunkSem, shift: Option<crate::SideShift>) -> Option<String> {
    let [o0, o1] = s.old_range;
    let shift = shift?;
    let container = s.enclosing.as_deref().map(short_container);
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
            return Some(match container {
                Some(nm) => format!("replaces {what} with {comment} in {nm}"),
                None => format!("replaces {what} with {comment}"),
            });
        }
    };
    Some(match container {
        Some(nm) => format!("{verb} code in {nm}"),
        None => format!("{verb} code"),
    })
}

/// The symbols a hunk defines, one bucket per verb their change earns, so
/// same-verb symbols are listed together: a hunk that adds two functions
/// reads "adds a, b" rather than twice over.
#[derive(Default)]
struct DefVerbs<'a> {
    /// (name, file it came from)
    moved: Vec<(&'a str, &'a str)>,
    /// (old name, new name)
    renamed: Vec<(&'a str, &'a str)>,
    /// (name, definition it was extracted from)
    extracted: Vec<(&'a str, &'a str)>,
    added: Vec<&'a str>,
    added_types: Vec<&'a str>,
    edited: Vec<&'a str>,
    signature_changed: Vec<&'a str>,
    type_changed: Vec<&'a str>,
}

// definition side: classify each defined symbol (moved-in / renamed /
// extracted / added / changed)
fn classify_defs<'a>(
    real: &[&'a str],
    is_type: bool,
    ctx: &RatCtx<'a>,
    my_file: usize,
) -> DefVerbs<'a> {
    let mut v = DefVerbs::default();
    let changed = ctx.changed.get(my_file);
    for &d in real {
        if let Some(src) = changed.and_then(|c| c.moved_in.get(d)) {
            v.moved.push((d, src.as_str()));
        } else if let Some(old) = changed.and_then(|c| c.rename.get(d)) {
            v.renamed.push((old.as_str(), d));
        } else if let Some(src) = changed.and_then(|c| c.relocated.get(d)) {
            v.extracted.push((d, src.as_str()));
        } else {
            match ctx.def_verb(my_file, d, is_type) {
                "adds" => v.added.push(d),
                "adds type" => v.added_types.push(d),
                "edits" => v.edited.push(d),
                "changes type" => v.type_changed.push(d),
                _ => v.signature_changed.push(d),
            }
        }
    }
    v
}

// one fragment per verb bucket, in the order they read
fn def_frags(v: &DefVerbs, s: &HunkSem, prose: bool) -> Vec<String> {
    // several symbols sharing a source collapse into one fragment naming
    // the source once (e.g. "adds A, B, extracted from tests") instead of
    // repeating "extracted from tests" per symbol.
    // …and several *sources* must still name the relation once, or the line
    // repeats "extracted from"/"from" per source — grouped within a source
    // but not across them. Each name then carries its own source inline.
    let mut frags = src_frags(&v.extracted, "adds", ", ", "extracted from");
    // renames are inherently pairwise (old → new), so grouping by target is
    // meaningless; instead cap the *number* of rename pairs shown, same as
    // name_list caps any other list.
    if !v.renamed.is_empty() {
        let items: Vec<String> = v
            .renamed
            .iter()
            .map(|(old, new)| format!("{old} → {new}"))
            .collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        frags.push(format!("renames {}", name_list(&refs)));
    }
    frags.extend(src_frags(&v.moved, "moves", " ", "from"));
    for (verb, items) in [
        ("adds", &v.added),
        ("adds type", &v.added_types),
        ("edits", &v.edited),
    ] {
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
    let (sig, plain): (Vec<&str>, Vec<&str>) = v.signature_changed.iter().partition(|d| {
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
    if !v.type_changed.is_empty() {
        frags.push(format!("changes type {}", name_list(&v.type_changed)));
    }
    frags
}

// provenance: a defined symbol used by another group. Name the symbol only
// when several constructs are listed (otherwise "used by X" is unambiguous).
// A definer the using file's import contradicts must not claim the use
// (`used in caller.py` when the caller imports the name from elsewhere).
// Only that: the graph's ambiguity rule does not belong here, and applying it
// dropped 274 provenance phrases in one corpus repo.
fn provenance<'a>(
    real: &[&'a str],
    mine: usize,
    sem: &[&HunkSem],
    ctx: &RatCtx,
) -> Option<(&'a str, String)> {
    let (d, b) = real.iter().find_map(|d| {
        ctx.users(d)
            .iter()
            .copied()
            .find(|&b| ctx.used_by(mine, b, d))
            .map(|b| (*d, b))
    })?;
    let my_file = ctx.group_file[mine];
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
    Some((d, prov))
}

// P17 composes with def-side wording rather than being suppressed by it: a
// hunk that both defines a real symbol and introduces bindings (e.g. a
// function plus the module-level constants beside it) must name both, or the
// constants silently vanish from the rationale. The def-side prefix is
// already list-capped, but a long qualified scope name (nested test
// class/function) can still push the composed line past the one-line bound —
// degrade the same way name_list itself does (a count instead of the full
// listing) rather than let the line grow unboundedly.
fn with_binding_clause(mut out: String, s: &HunkSem, old_locals: &HashSet<String>) -> String {
    let Some(r) = binding_rationale(s, old_locals) else {
        return out;
    };
    if out.chars().count() + 2 + r.chars().count() <= MAX_RATIONALE {
        out += "; ";
        out += &r;
        return out;
    }
    let n = s
        .bindings
        .iter()
        .filter(|b| !old_locals.contains(&b.name))
        .count();
    let summary = format!("; +{n} more binding{}", if n == 1 { "" } else { "s" });
    if out.chars().count() + summary.chars().count() <= MAX_RATIONALE {
        out += &summary;
    }
    out
}

// definition side: report EVERY construct the hunk touches, not just one.
// Classify each defined symbol, group same-verb symbols, and append
// provenance once. Placeholder names (unnamed closures, `_`-bound throwaways)
// carry no navigational signal — drop them from the wording and fall through
// to the use side and enclosing-edit wording when a hunk defines nothing else.
fn def_side_rationale(s: &HunkSem, sem: &[&HunkSem], mine: usize, ctx: &RatCtx) -> Option<String> {
    let real: Vec<&str> = s
        .defines
        .iter()
        .map(String::as_str)
        .filter(|d| *d != "<anonymous>" && *d != "_")
        .collect();
    if real.is_empty() {
        return None;
    }
    let my_file = ctx.group_file[mine];
    let verbs = classify_defs(&real, s.is_type, ctx, my_file);
    let frags = def_frags(&verbs, s, ctx.is_prose(my_file));
    let mut out = join_frags(&frags);
    if let Some((d, prov)) = provenance(&real, mine, sem, ctx) {
        out += &if frags.len() > 1 {
            format!(", {d} {prov}")
        } else {
            format!(", {prov}")
        };
    }
    Some(with_binding_clause(
        out,
        s,
        &ctx.symbols[my_file].old_locals,
    ))
}

// use side: a symbol used here that some other group defines (this change)
fn use_side_rationale(s: &HunkSem, mine: usize, ctx: &RatCtx) -> Option<String> {
    if s.uses.is_empty() {
        return None;
    }
    let my_file = ctx.group_file[mine];
    // #6: from a test file, prefer a symbol defined in a non-test file
    if is_test_path(&ctx.paths[my_file]) {
        let from_code =
            |b: usize| ctx.group_file[b] != my_file && !is_test_path(&ctx.paths[ctx.group_file[b]]);
        if let Some((u, b)) = defined_elsewhere(s, mine, ctx, from_code) {
            return Some(format!("tests {u} ({})", ctx.paths[ctx.group_file[b]]));
        }
    }
    if let Some((u, b)) = defined_elsewhere(s, mine, ctx, |_| true) {
        return Some(ctx.use_of_phrase(u, mine, b));
    }
    Some(
        scope_rationale(s, ctx, my_file).unwrap_or_else(|| match &s.enclosing {
            Some(nm) => format!("edits {}", short_container(nm)),
            None => {
                let names: Vec<&str> = s.uses.iter().map(String::as_str).collect();
                format!("uses {}", name_list(&names))
            }
        }),
    )
}

// the first of the hunk's uses that another group `accept`s defines, by the
// same rule the def→use graph drew its edges with
fn defined_elsewhere<'a>(
    s: &'a HunkSem,
    mine: usize,
    ctx: &RatCtx,
    accept: impl Fn(usize) -> bool,
) -> Option<(&'a String, usize)> {
    s.uses.iter().find_map(|u| {
        ctx.definers(u)
            .iter()
            .copied()
            .find(|&b| ctx.ok_for(mine, b, u) && accept(b))
            .map(|b| (u, b))
    })
}

// what the hunk did within its own scope, tried in order: P17, a local
// binding this hunk introduces beats the bare "edits {enclosing}" fallback —
// naming the binding and where it's used (or that it isn't) is more useful
// than restating the enclosing def; then a signature edit above a body; then
// module-level bindings; then, at file scope or inside a call's argument list,
// the detail layer, which already says what moved ("adds action, help to
// add_argument(...)") where "uses action, add_argument, help" would only list
// the identifiers
fn scope_rationale(s: &HunkSem, ctx: &RatCtx, my_file: usize) -> Option<String> {
    binding_rationale(s, &ctx.symbols[my_file].old_locals)
        .or_else(|| ctx.header_edit(my_file, s))
        .or_else(|| ctx.file_bind_phrase(my_file, s))
        .or_else(|| detail_rationale(s))
}

// nothing named a construct: say what the hunk did to its container, else to
// the file — a removal, a direction and a size — rather than the bare word
// "change", which told a reviewer nothing at all
fn shape_rationale(s: &HunkSem, ctx: &RatCtx, my_file: usize) -> String {
    if let Some(nm) = &s.enclosing {
        // "section" is the word for a prose *definition*; a region already
        // names what it is ("preamble", "front matter", "#ifdef X"), so
        // prefixing it would read as "edits section front matter"
        let nm = short_container(nm);
        return if ctx.is_prose(my_file) && s.enclosing_kind.is_none() {
            format!("edits {}", prose_noun(1, &nm))
        } else {
            format!("edits {nm}")
        };
    }
    // #5/#7 removal: a deletion hunk whose old lines held a removed symbol
    let [o0, o1] = s.old_range;
    let removed = ctx.changed.get(my_file).and_then(|c| {
        c.removals
            .iter()
            .find(|r| r.row >= o0 && r.row <= o1)
            .map(removal_phrase)
    });
    removed.unwrap_or_else(|| size_rationale(s))
}

// a direction and a size. A pure deletion of body lines (no tracked
// def/import removed) names its size so a large removal isn't hidden behind
// a blank "change"; comment deletions never reach here, the comment-only
// branch already claims them. Anything else is a hunk the grammar recognises
// nothing in: a `#define` (not a definition since macros left `defines`), a
// continuation line inside a shell command, the prose ahead of an added
// document's first heading.
fn size_rationale(s: &HunkSem) -> String {
    let [o0, o1] = s.old_range;
    let lines = |n: usize| format!("{n} line{}", if n == 1 { "" } else { "s" });
    if s.new_empty && o1 >= o0 {
        return format!("removes {}", lines(o1 - o0 + 1));
    }
    let n = lines(s.new_len.max(1));
    if o1 < o0 {
        format!("adds {n}")
    } else {
        format!("edits {n}")
    }
}

// The detail layer as the rationale, for a hunk with nothing better to say:
// only at file scope or in a call's arguments, where "edits <container>" has
// no container worth naming. Inside a definition "edits f" stays — it is true
// and short, and the details ride alongside it.
fn detail_rationale(s: &HunkSem) -> Option<String> {
    let bare = s.enclosing.is_none() || s.enclosing_kind == Some(crate::ContainerKind::Call);
    // a pure deletion has its own wording in `shape_rationale` ("removes section Usage"),
    // shorter than the detail that says the same with its container
    if !bare || s.new_empty || s.details.is_empty() {
        return None;
    }
    Some(
        s.details
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join("; "),
    )
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
    frags.extend(unused_scoped_frag(&no_uses_scoped));
    if !no_uses_file.is_empty() {
        // scoped fragments above already say "no uses in {scope}" — when both
        // kinds land in the same rationale (P17 composing with def-side
        // wording), repeating that exact phrase at file scope reads as the
        // per-symbol-fragment defect rationale_bounds.rs guards against, even
        // though it's really two distinct groups. Reword only in that mixed
        // case; the lone-fragment wording (no scoped group alongside it)
        // stays as-is.
        let where_ = if no_uses_scoped.is_empty() {
            "no uses in this file"
        } else {
            "unused elsewhere in this file"
        };
        frags.push(format!(
            "adds {}, {where_} — check other files",
            name_list(&no_uses_file)
        ));
    }
    if !used_scoped.is_empty() {
        frags.push(used_frag(&used_scoped, "local "));
    }
    if !used_file.is_empty() {
        frags.push(used_frag(&used_file, ""));
    }
    Some(join_frags(&frags))
}

// One scope reads naturally; several must still say the phrase once, or the
// line repeats "no uses in ..." per scope — grouped per scope, but not
// grouped across them.
fn unused_scoped_frag(groups: &[(&str, Vec<&str>)]) -> Option<String> {
    match groups {
        [] => None,
        [(scope, names)] => Some(format!(
            "adds local {}, no uses in {scope} — check nested scopes",
            name_list(names)
        )),
        _ => {
            let mut each: Vec<String> = groups
                .iter()
                .flat_map(|(scope, names)| names.iter().map(move |n| format!("{n} (in {scope})")))
                .collect();
            each.sort();
            let refs: Vec<&str> = each.iter().map(String::as_str).collect();
            Some(format!(
                "adds local {}, no uses in their own scopes — check nested scopes",
                name_list(&refs)
            ))
        }
    }
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
    let (old_by, new_by) = members_by_container(s, h, old_members, nests);
    let mut containers: Vec<Option<String>> = old_by.keys().chain(new_by.keys()).cloned().collect();
    containers.sort();
    containers.dedup();

    let mut out = vec![];
    let empty = HashMap::new();
    for container in containers {
        let old = old_by.get(&container).unwrap_or(&empty);
        let new = new_by.get(&container).unwrap_or(&empty);
        for (verb, prep, names) in member_diff(container.as_deref(), old, new) {
            if names.is_empty() {
                continue;
            }
            let list = name_list(&names);
            let list = if prose && verb != "changes" {
                prose_noun(names.len(), &list)
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

/// name → normalized text, per container
type MemberTable<'a> = HashMap<Option<String>, HashMap<&'a str, &'a str>>;

/// The members on each side of the hunk, keyed by the container each belongs
/// to: old-side rows by the hunk's old line range, new-side members as
/// `analyze` attributed them.
fn members_by_container<'a>(
    s: &'a HunkSem,
    h: &RawHunk,
    old_members: &'a [extract::MemberRow],
    nests: bool,
) -> (MemberTable<'a>, MemberTable<'a>) {
    let [o0, o1] = h.old_range;
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

    let mut old_by: MemberTable = HashMap::new();
    for (_row, n, t, ctr) in old_members
        .iter()
        .filter(|(row, ..)| o0 <= row + 1 && *row < o1)
    {
        old_by
            .entry(resolve(ctr))
            .or_default()
            .insert(n.as_str(), t.as_str());
    }
    let mut new_by: MemberTable = HashMap::new();
    for (n, t, ctr) in &s.members {
        new_by
            .entry(resolve(ctr))
            .or_default()
            .insert(n.as_str(), t.as_str());
    }
    (old_by, new_by)
}

/// What changed between the two sides of one container, as (verb,
/// preposition, sorted names): added, removed, then changed.
fn member_diff<'a>(
    container: Option<&str>,
    old: &HashMap<&'a str, &'a str>,
    new: &HashMap<&'a str, &'a str>,
) -> [(&'static str, &'static str, Vec<&'a str>); 3] {
    // A member that IS its own container names nothing new: a js
    // `{ run: () => {} }` makes `run` both the member and — once the
    // arrow is a definition — the enclosing def, which would read "adds
    // run to run". Attributing a member to `stack` before its own def is
    // pushed (see `member_container`) already keeps this from happening
    // for the def-container case; kept as a defensive backstop and to
    // cover a call whose callee or literal happens to equal a member name.
    let self_named =
        |name: &str| container.is_some_and(|c| c == name || c.rsplit('.').next() == Some(name));
    let sorted = |mut v: Vec<&'a str>| {
        v.sort();
        v
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
    [
        ("adds", "to", added),
        ("removes", "from", removed),
        ("changes", "in", changed),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(prio: i64, file: usize, row: usize, gi: usize) -> GroupKey {
        (std::cmp::Reverse(prio), 0, file, row, gi)
    }

    #[test]
    fn topo_order_reads_a_doc_after_the_code_unless_a_rule_says_otherwise() {
        let doc = |prio: i64| (std::cmp::Reverse(prio), 1u8, 0, 0, 0);
        assert_eq!(topo_order(&[doc(0), key(0, 1, 0, 1)], &[]), vec![1, 0]);
        assert_eq!(topo_order(&[doc(5), key(0, 1, 0, 1)], &[]), vec![0, 1]);
    }

    #[test]
    fn topo_order_puts_a_definition_before_its_use_whatever_the_file_order_says() {
        // group 0 (file 0) uses what group 1 (file 1) defines
        let keys = [key(0, 0, 0, 0), key(0, 1, 0, 1)];
        assert_eq!(topo_order(&keys, &[(1, 0)]), vec![1, 0]);
        assert_eq!(topo_order(&keys, &[]), vec![0, 1]);
    }

    #[test]
    fn topo_order_breaks_a_cycle_at_the_lowest_key_and_still_emits_every_group() {
        let keys = [key(0, 0, 0, 0), key(0, 0, 5, 1), key(0, 0, 9, 2)];
        assert_eq!(topo_order(&keys, &[(0, 1), (1, 2), (2, 0)]), vec![0, 1, 2]);
    }

    #[test]
    fn topo_order_lets_a_rule_priority_reorder_only_the_freed_groups() {
        // group 2 carries a priority but depends on group 0: it cannot jump the
        // edge, yet it does jump group 1, which nothing holds back
        let keys = [key(0, 0, 0, 0), key(0, 0, 5, 1), key(9, 0, 9, 2)];
        assert_eq!(topo_order(&keys, &[(0, 2)]), vec![0, 2, 1]);
    }

    #[test]
    fn clusters_are_connected_components_over_hunks_sorted_by_first_hunk() {
        let groups: Vec<GroupInfo> = [vec![3, 4], vec![0], vec![1, 2]]
            .into_iter()
            .map(|members| GroupInfo {
                reason: String::new(),
                members,
            })
            .collect();
        assert_eq!(
            components(&groups, [(0, 2)].iter()),
            vec![vec![0], vec![1, 2, 3, 4]]
        );
        assert_eq!(
            components(&groups, [].iter()),
            vec![vec![0], vec![1, 2], vec![3, 4]]
        );
    }

    #[test]
    fn join_frags_keeps_what_fits_and_counts_the_rest() {
        let long = "x".repeat(100);
        let frags = vec![long.clone(), long.clone(), long.clone()];
        assert_eq!(join_frags(&frags), format!("{long}; {long}; +1 more"));
        let one = vec!["y".repeat(300)];
        let out = join_frags(&one);
        assert!(out.ends_with('…') && out.chars().count() <= MAX_RATIONALE);
    }

    #[test]
    fn clamp_rationale_cuts_at_a_fragment_boundary_when_enough_is_kept() {
        let r = format!("{}; {}", "a".repeat(150), "b".repeat(150));
        assert_eq!(clamp_rationale(r), format!("{}…", "a".repeat(150)));
        let r = format!("{}; {}", "a".repeat(20), "b".repeat(300));
        assert_eq!(clamp_rationale(r).chars().count(), MAX_RATIONALE - 1);
    }

    #[test]
    fn name_list_shows_three_names_then_a_count() {
        assert_eq!(name_list(&["a", "b", "c"]), "a, b, c");
        assert_eq!(name_list(&["a", "b", "c", "d", "e"]), "a, b, c, and 2 more");
    }

    #[test]
    fn short_container_collapses_a_test_label_but_keeps_a_heading_path() {
        assert_eq!(
            short_container("describe \"compiler\" > it \"errors on a bad argument\""),
            "it \"errors on a bad argument\""
        );
        assert_eq!(short_container("Project > Install"), "Project > Install");
        assert!(short_container(&"n".repeat(80)).ends_with('…'));
        assert_eq!(bare_name("Outer::Inner.method"), "method");
    }

    #[test]
    fn module_matches_on_the_last_segment_only() {
        assert!(module_matches("pkg/utils.py", "pkg.utils"));
        assert!(module_matches("a/b/utils.ts", "./utils"));
        assert!(module_matches("types/api.d.ts", "api"));
        assert!(!module_matches("pkg/other.py", "pkg.utils"));
        assert!(!module_matches("utils.py", ""));
    }

    #[test]
    fn src_frags_names_the_relation_once_per_source_and_once_across_sources() {
        assert_eq!(
            src_frags(&[("a", "t"), ("b", "t")], "adds", ", ", "extracted from"),
            vec!["adds a, b, extracted from t".to_string()]
        );
        assert_eq!(
            src_frags(&[("a", "t"), ("b", "u")], "moves", " ", "from"),
            vec!["moves a (from t), b (from u)".to_string()]
        );
        assert!(src_frags(&[], "adds", ", ", "extracted from").is_empty());
    }
}
