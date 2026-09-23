//! Ordering across a whole changeset: group by (file, enclosing definition)
//! (P1), def→use edges between groups (P2) — cross-file when enabled (P4),
//! topological sort (Kahn) with deterministic (import, file, position) tiebreak.
//! Single-file is just the one-file case of this.
use crate::extract::{self, BindingUse, HunkSem, RawHunk};
use crate::model::{Category, Options, Removal, RemovalKind, Strategy};
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
///
/// Every removal the hunk covers is named, not just the first: a deleted
/// class or workflow file reads "removes a, b, c" where naming one member of
/// it — whichever happened to come first — claimed the rest survived.
/// Grouped by kind so one hunk does not repeat the verb per name.
fn removal_phrase(rs: &[&Removal]) -> String {
    let mut frags = vec![];
    for (kind, verb) in [
        (RemovalKind::Def, "removes"),
        (RemovalKind::Section, "removes section"),
        (RemovalKind::Import, "removes import"),
    ] {
        let mut names: Vec<&str> = rs
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.name.as_str())
            .collect();
        // one name per thing removed, in the order they were removed: a class
        // and its members can declare the same name twice (a property and its
        // getter), and "removes buffer, buffer" reads as a bug in the tool
        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(*n));
        if !names.is_empty() {
            frags.push(format!("{verb} {}", name_list(&names)));
        }
    }
    // a move keeps its destination, so the names that went to the same place
    // are named together rather than repeating the path per name
    let moved: Vec<(&str, &str)> = rs
        .iter()
        .filter_map(|r| match &r.kind {
            RemovalKind::MovedTo(path) => Some((r.name.as_str(), path.as_str())),
            _ => None,
        })
        .collect();
    for (path, names) in group_by_src(&moved) {
        frags.push(format!("moves {} to {path}", name_list(&names)));
    }
    // A hunk that deleted a whole module has one thing to say, not six: the
    // first two fragments carry it and the rest are counted.
    let more = frags.len().saturating_sub(REMOVAL_FRAGS);
    frags.truncate(REMOVAL_FRAGS);
    let mut out = join_frags(&frags);
    if more > 0 {
        out += &format!(", and {more} more");
    }
    out
}

/// How many kinds of removal one hunk spells out before it starts counting.
const REMOVAL_FRAGS: usize = 2;

/// The longest detail that may stand in for the rationale. Past this it is
/// the detail layer's to show — a css-in-js object's computed keys run to
/// three times the length of the container's own name.
const DETAIL_MAX: usize = 80;

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
    options: &Options,
) -> OrderedAll {
    let FileFacts { symbols, changed } = *facts;
    let flat = flatten(files);
    let (groups, group_idx) = group_hunks(&flat);
    let (gdef, guse, gmember) = group_symbols(&flat, &groups, &group_idx, symbols, paths);
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
        member_only: &gmember,
    };
    let (mut edges, gedges) =
        def_use_edges(&groups, &flat.sem, &gdef, &users, &bind, options.cross_file);
    let (contain_edges, contain_gedges) = containment_edges(&flat, &groups, &group_idx);
    edges.extend(contain_edges);

    let docs: Vec<u8> = group_file
        .iter()
        .map(|&f| docs_rank(&paths[f], options.docs_last))
        .collect();
    let keys = GroupKeys {
        docs: &docs,
        file: &group_file,
        row: &group_row,
    };
    let group_order = order_groups(options.strategy, &groups, &flat.sem, &gedges, &keys);
    // ---- flatten groups to a global hunk permutation ----
    let mut perm = vec![];
    for &gi in &group_order {
        let mut mem = groups[gi].members.clone();
        mem.sort_by_key(|&i| (flat.coord[i].0, flat.sem[i].start_row, i));
        perm.extend(mem);
    }

    // ambiguity is dropped: two files renaming different defs to the same old
    // spelling say nothing about which one an importer followed
    let mut renamed_to: HashMap<&str, Option<&str>> = HashMap::new();
    for c in changed {
        for (new, old) in &c.rename {
            renamed_to
                .entry(old.as_str())
                .and_modify(|e| {
                    if *e != Some(new.as_str()) {
                        *e = None;
                    }
                })
                .or_insert(Some(new.as_str()));
        }
    }
    let renamed_to: HashMap<&str, &str> = renamed_to
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect();

    let replaced = replaced_nearby(&flat);
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
        renamed_to: &renamed_to,
        member_only: &gmember,
        comment: &flat.comment,
        switched: &flat.switched,
        replaced: &replaced,
        cross_file: options.cross_file,
    };
    let rationale = (0..flat.sem.len())
        .map(|i| clamp_rationale(rationale_for(i, &flat.sem, &group_idx, &ctx)))
        .collect();
    // a doc's code fence orders the doc after the code, but never joins its cluster
    let clusters = components(
        &groups,
        gedges.iter().chain(&contain_gedges).filter(|(_, b)| {
            !crate::lang::for_path(&paths[group_file[*b]]).is_some_and(|s| s.prose)
        }),
    );

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

/// How many old-side lines may lie between two hunks and still read as one
/// edit: the blank line and the closing token the splitter left between a
/// deletion and the code that replaced it.
const NEIGHBOUR_GAP: usize = 3;
/// …and how many hunks may sit between them. The splitter breaks at construct
/// boundaries, so the insertion is next door but not always immediately next.
const NEIGHBOUR_HUNKS: usize = 2;
/// How much of what the deletion dropped the neighbour must add back before it
/// counts as the replacement, as a fraction: a 56-line block deleted beside a
/// four-line edit was deleted, whatever the edit did.
const NEIGHBOUR_SHARE: (usize, usize) = (1, 2);

/// Where a pure deletion's replacement went, when the change put it in a
/// neighbouring hunk. The splitter breaks at construct boundaries, so a
/// rewritten dispatch reads as two hunks: one dropping the old lines, one
/// adding the new. On its own the deletion says "removes 3 lines", which is
/// true of its own rows and reads to a reviewer as code disappearing.
fn replaced_nearby(flat: &Flat) -> Vec<Option<&'static str>> {
    let span = |[a, b]: [usize; 2]| (b + 1).saturating_sub(a);
    // what the neighbour added over what it dropped
    let growth = |s: &HunkSem| s.new_len.saturating_sub(span(s.old_range));
    (0..flat.sem.len())
        .map(|i| {
            let s = flat.sem[i];
            let [o0, o1] = s.old_range;
            if !s.new_empty || o1 < o0 {
                return None;
            }
            let (num, den) = NEIGHBOUR_SHARE;
            let enough = |s: &HunkSem| growth(s) * den >= (o1 + 1 - o0) * num;
            let lo = i.saturating_sub(NEIGHBOUR_HUNKS);
            let hi = (i + NEIGHBOUR_HUNKS + 1).min(flat.sem.len());
            (lo..hi)
                .filter(|&j| j != i && flat.coord[j].0 == flat.coord[i].0)
                .filter(|&j| enough(flat.sem[j]))
                .filter_map(|j| {
                    let [n0, n1] = flat.sem[j].old_range;
                    // lines lying between the two hunks, either way round
                    let between = if j < i {
                        let end = n1.max(n0.saturating_sub(1)).min(o0 - 1);
                        o0 - 1 - end
                    } else {
                        n0.max(o1 + 1) - 1 - o1
                    };
                    (between <= NEIGHBOUR_GAP).then_some((between, j))
                })
                // nearest wins; on a tie the earlier hunk does, so the answer
                // does not depend on which word sorts first
                .min()
                .map(|(_, j)| if j < i { "above" } else { "below" })
        })
        .collect()
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
    paths: &[String],
) -> (
    Vec<HashSet<String>>,
    Vec<HashSet<String>>,
    Vec<HashSet<String>>,
) {
    let mut gdef: Vec<HashSet<String>> = vec![HashSet::new(); groups.len()];
    let mut guse: Vec<HashSet<String>> = vec![HashSet::new(); groups.len()];
    // the names a group only ever wrote as `obj.name` — see `Binding::member_only`
    let mut gmember: Vec<HashSet<String>> = vec![HashSet::new(); groups.len()];
    // What this change defines, and where. An import links to it only when
    // the statement's own module names that file: a test file that happens to
    // define `render` must not become the definer of every
    // `import { render } from "lib"` in the change.
    let defined_in: HashMap<&str, usize> = flat
        .sem
        .iter()
        .enumerate()
        .flat_map(|(i, s)| {
            let file = flat.coord[i].0;
            s.defines.iter().map(move |d| (d.as_str(), file))
        })
        .collect();
    for (i, s) in flat.sem.iter().enumerate() {
        let gi = group_idx[i];
        gdef[gi].extend(s.defines.iter().cloned());
        // An added file whose `from a import foo` and `foo()` land in one
        // insertion has nothing in `uses` to link on — the import declares
        // the name the call refers to. The import is the dependency then, as
        // long as the module it names is the file this change defines it in
        // (tasks-3uv.12).
        let my_file = flat.coord[i].0;
        for im in &s.imports {
            let Some(&home) = defined_in.get(im.as_str()) else {
                continue;
            };
            let module = symbols
                .get(my_file)
                .and_then(|f| f.imported_from.get(im.as_str()))
                .and_then(|(_, module)| module.as_deref());
            if home != my_file && module.is_some_and(|m| names_file(m, &paths[home])) {
                guse[gi].insert(im.clone());
            }
        }
        gmember[gi].extend(s.member_uses.iter().cloned());
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
    // a name one hunk of the group wrote plainly is a real reference for the
    // whole group, so only the names no member spelled bare stay member-only
    for (gi, m) in gmember.iter_mut().enumerate() {
        let bare: HashSet<&str> = groups[gi]
            .members
            .iter()
            .flat_map(|&i| {
                let s = flat.sem[i];
                s.uses
                    .iter()
                    .filter(|u| !s.member_uses.contains(u))
                    .map(String::as_str)
            })
            .collect();
        m.retain(|n| !bare.contains(n.as_str()));
    }
    (gdef, guse, gmember)
}

/// Whether an import's module names this file. A module is written the way
/// the language spells it — `./util`, `../a/b`, `pkg.mod`, `a.b.c` — so the
/// comparison is on the path's own segments without its extension, which is
/// all the two spellings share.
fn names_file(module: &str, path: &str) -> bool {
    let stem = path.rsplit('/').next().unwrap_or(path);
    let stem = stem.split_once('.').map_or(stem, |(s, _)| s);
    let tail = module
        .rsplit(['/', '.'])
        .find(|s| !s.is_empty() && *s != "js" && *s != "ts");
    tail == Some(stem)
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
    let mut frontier = Frontier::new(key_v, gedges);
    let mut order = Vec::with_capacity(key_v.len());
    while let Some(pick) = frontier.take() {
        order.push(pick);
        frontier.release(pick);
    }
    order
}

/// Kahn's frontier over the group graph: `ready` is the groups with no unmet
/// predecessor and `left` every group not yet emitted, both keyed so the
/// smallest key is picked first. A cycle leaves `ready` empty with `left`
/// not; the smallest of `left` is picked then, so a cycle is broken rather
/// than dropped.
struct Frontier<'a> {
    key_v: &'a [GroupKey],
    indeg: Vec<usize>,
    succ: Vec<Vec<usize>>,
    ready: BTreeSet<GroupKey>,
    left: BTreeSet<GroupKey>,
}

impl<'a> Frontier<'a> {
    fn new(key_v: &'a [GroupKey], gedges: &[(usize, usize)]) -> Self {
        let g = key_v.len();
        let mut indeg = vec![0usize; g];
        let mut succ: Vec<Vec<usize>> = vec![vec![]; g];
        for &(a, b) in gedges {
            succ[a].push(b);
            indeg[b] += 1;
        }
        let ready = (0..g)
            .filter(|&gi| indeg[gi] == 0)
            .map(|gi| key_v[gi])
            .collect();
        let left = key_v.iter().copied().collect();
        Frontier {
            key_v,
            indeg,
            succ,
            ready,
            left,
        }
    }

    fn take(&mut self) -> Option<usize> {
        let key = self.ready.first().or_else(|| self.left.first()).copied()?;
        self.ready.remove(&key);
        self.left.remove(&key);
        Some(key.4)
    }

    /// `pick` is emitted: each successor loses a predecessor, and one with
    /// none left (and not already emitted) becomes ready
    fn release(&mut self, pick: usize) {
        for i in 0..self.succ[pick].len() {
            let s = self.succ[pick][i];
            if self.indeg[s] == 0 {
                continue;
            }
            self.indeg[s] -= 1;
            if self.indeg[s] == 0 && self.left.contains(&self.key_v[s]) {
                self.ready.insert(self.key_v[s]);
            }
        }
    }
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
    /// per group, the names it only ever wrote as `obj.name`
    member_only: &'a [HashSet<String>],
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
        let same_file = self.group_file[definer] == self.group_file[user];
        definer != user
            && (cross_file || same_file)
            && (same_file || self.reaches_out(user, s))
            && self.resolves(definer, user, s)
    }

    /// May another file answer a use of `s` in `user`? Not for `obj.s`, nor
    /// when the file binds `s` itself and does not import it.
    fn reaches_out(&self, user: usize, s: &str) -> bool {
        let binds = self.symbols.get(self.group_file[user]).is_some_and(|f| {
            !f.imported_from.contains_key(s)
                && (f.new_binds.contains(s) || f.new_locals.contains(s))
        });
        !self.member_only[user].contains(s) && !binds
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
    /// old definition name → the name it was renamed to, across every file of
    /// the change. An import hunk that only drops the old spelling has no
    /// other way to know the name came back renamed next door.
    renamed_to: &'a HashMap<&'a str, &'a str>,
    /// see `group_symbols`; `bind()` needs it to rebuild the edge gate
    member_only: &'a [HashSet<String>],
    comment: &'a [bool],
    /// how the hunk moved code across the comment boundary, if it did
    switched: &'a [Option<crate::SideShift>],
    /// see `replaced_nearby`: for a pure deletion, where the hunk next door
    /// put the lines that took its place
    replaced: &'a [Option<&'static str>],
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
            member_only: self.member_only,
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
    // wording must not claim a link `edge_allowed` refused
    fn reads_name(&self, user: usize, mine: usize, sym: &str) -> bool {
        self.group_file[user] == self.group_file[mine] || self.bind().reaches_out(user, sym)
    }
    // ...and `other` really is where `sym` comes from, by the same rule the
    // def→use graph uses: the rationale must not name a definition the graph
    // refused to draw an edge to
    /// Provenance from the definition's side: `mine` defines `sym`, `other`
    /// uses it. Only the import gate applies — see `Binding::import_allows`.
    fn used_by(&self, mine: usize, other: usize, sym: &str) -> bool {
        self.ok(mine, other)
            && self.reads_name(other, mine, sym)
            && self.bind().import_allows(other, mine, sym)
    }
    fn ok_for(&self, mine: usize, other: usize, sym: &str) -> bool {
        self.ok(mine, other)
            && self.reads_name(mine, other, sym)
            && self.bind().resolves(other, mine, sym)
    }
    // markdown (currently the only prose language): rationale wording says
    // "section" instead of naming a construct kind.
    fn is_prose(&self, file: usize) -> bool {
        crate::lang::for_path(&self.paths[file]).is_some_and(|s| s.prose)
    }
    // #3/#4: verb for a definition hunk. New symbol → "adds"/"adds type"; a
    // pre-existing symbol whose header changed → "changes signature of"/"changes
    // type" (a def-category hunk means the declaration line itself moved).
    fn def_verb(&self, file: usize, sym: &str, is_type: bool, inserted: bool) -> &'static str {
        let existed = !inserted
            && self
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
    /// Whether the hunk's own rows removed something the pipeline tracked —
    /// a definition, a section, a module-level binding — rather than only
    /// lines.
    /// What the hunk's own rows removed, named: a tracked definition, else a
    /// member the detail layer named — a method or a field, which a file's
    /// removal list does not carry.
    fn removed_phrase(&self, file: usize, s: &HunkSem) -> Option<String> {
        let [o0, o1] = s.old_range;
        if o0 > o1 {
            return None;
        }
        let tracked: Vec<&Removal> = self
            .changed
            .get(file)
            .map(|c| {
                c.removals
                    .iter()
                    .filter(|r| r.kind != RemovalKind::Import && (o0..=o1).contains(&r.row))
                    .collect()
            })
            .unwrap_or_default();
        if !tracked.is_empty() {
            return Some(removal_phrase(&tracked));
        }
        s.details.iter().find(|d| d.starts_with("removes")).cloned()
    }

    /// Whether this file's new side declares nothing at all: a deletion, or
    /// a file emptied to the same effect.
    fn file_emptied(&self, file: usize) -> bool {
        self.symbols.get(file).is_some_and(|s| {
            s.new_defs.is_empty() && s.new_imports.is_empty() && s.new_binds.is_empty()
        })
    }

    fn use_of_phrase(&self, sym: &str, mine: usize, b: usize) -> String {
        // a symbol this change introduces is "added", not "defined": the
        // reader would otherwise take the definition for pre-existing code
        let verb = if self.symbols[self.group_file[b]].old_defs.contains(sym) {
            "defined"
        } else {
            "added"
        };
        if self.group_file[b] != self.group_file[mine] {
            format!("uses {sym}, {verb} in {}", self.paths[self.group_file[b]])
        } else {
            let dir = if self.group_row[b] < self.group_row[mine] {
                "above"
            } else {
                "below"
            };
            format!("uses {sym}, {verb} {dir}")
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
        .or_else(|| {
            let shift = ctx.switched.get(i).copied().flatten();
            switch_rationale(s, shift, ctx.removed_phrase(my_file, s))
        })
        .or_else(|| {
            let is_comment = ctx.comment.get(i).copied().unwrap_or(false);
            is_comment.then(|| comment_rationale(s.old_range, s.new_empty, s.enclosing.as_deref()))
        })
        .or_else(|| def_side_rationale(s, sem, mine, ctx))
        .or_else(|| use_side_rationale(s, mine, ctx))
        .or_else(|| scope_rationale(s, ctx, my_file))
        .unwrap_or_else(|| shape_rationale(i, s, ctx, my_file))
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
        let (mut pairs, mut gone): (Vec<String>, Vec<&str>) = (vec![], vec![]);
        for &n in &names {
            match import_renamed_to(ctx, my_file, n) {
                Some(to) => pairs.push(format!("{n} → {to}")),
                None => gone.push(n),
            }
        }
        let frags: Vec<String> = import_rename_phrase(&pairs)
            .into_iter()
            .chain((!gone.is_empty()).then(|| format!("removes import {}", name_list(&gone))))
            .collect();
        return Some(join_frags(&frags));
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
fn switch_rationale(
    s: &HunkSem,
    shift: Option<crate::SideShift>,
    removed: Option<String>,
) -> Option<String> {
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
            let comment = if n == 1 { "a comment" } else { "comments" };
            // What left, when the pipeline knows its name. A method deleted
            // under a new comment header read as "replaces 2 lines with
            // comments": true of the bytes, silent about the code.
            if let Some(what) = removed {
                return Some(format!("{what}, replaced by {comment}"));
            }
            let what = if n == 1 {
                "1 line".to_string()
            } else {
                format!("{n} lines")
            };
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
    s: &HunkSem,
    ctx: &RatCtx<'a>,
    my_file: usize,
) -> DefVerbs<'a> {
    let mut v = DefVerbs::default();
    let changed = ctx.changed.get(my_file);
    // Nothing inside inserted lines can be an edit of what was there: a new
    // yaml document repeating its neighbours' keys, a new overload of an
    // existing name, a second `impl` block. The name may be old; this
    // occurrence of it is not.
    let inserted = s.old_range[0] > s.old_range[1];
    for &d in real {
        if let Some(src) = changed.and_then(|c| c.moved_in.get(d)) {
            v.moved.push((d, src.as_str()));
        } else if let Some(old) = changed.and_then(|c| c.rename.get(d)) {
            v.renamed.push((old.as_str(), d));
        } else if let Some(src) = changed.and_then(|c| c.relocated.get(d)) {
            v.extracted.push((d, src.as_str()));
        } else {
            match ctx.def_verb(my_file, d, s.is_type, inserted) {
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
    let verbs = classify_defs(&real, s, ctx, my_file);
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
            // A test updated to spell a renamed symbol the new way asserts no
            // new coverage: the same test, following the rename. "tests X"
            // would claim the change tests something it only renamed.
            let renamed = ctx
                .changed
                .get(ctx.group_file[b])
                .is_some_and(|c| c.rename.contains_key(u.as_str()));
            if !renamed {
                return Some(format!("tests {u} ({})", ctx.paths[ctx.group_file[b]]));
            }
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
fn shape_rationale(i: usize, s: &HunkSem, ctx: &RatCtx, my_file: usize) -> String {
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
    let [o0, o1] = s.old_range;
    // A file whose new side holds nothing at all, deleted from its first
    // line: naming one of its constructs — or twenty-five of them — buries
    // the fact that the file is gone.
    if o0 == 1 && s.new_empty && ctx.file_emptied(my_file) {
        return "deletes the file".to_string();
    }
    // #5/#7 removal: a deletion hunk whose old lines held a removed symbol
    let removed = ctx
        .changed
        .get(my_file)
        .map(|c| {
            c.removals
                .iter()
                .filter(|r| (o0..=o1).contains(&r.row))
                .collect::<Vec<_>>()
        })
        .filter(|rs| !rs.is_empty())
        .map(|rs| {
            let (renamed, rest) = split_import_renames(&rs, ctx, my_file);
            let frags: Vec<String> = renamed
                .into_iter()
                .chain((!rest.is_empty()).then(|| removal_phrase(&rest)))
                .collect();
            join_frags(&frags)
        });
    removed.unwrap_or_else(|| size_rationale(s, ctx.replaced.get(i).copied().flatten()))
}

/// An import statement that only loses names reads as a removal, and usually
/// is one. It is not when the definition was renamed elsewhere in the same
/// change and this file imports the new spelling in another hunk: nothing
/// left, the name moved. Only a def-side rename counts as evidence — pairing
/// dropped and added imports on spelling alone would turn `User` →
/// `UserProfile` into a rename it never was.
fn import_renamed_to<'c>(ctx: &RatCtx<'c>, my_file: usize, name: &str) -> Option<&'c str> {
    let new_imports = ctx.symbols.get(my_file).map(|f| &f.new_imports)?;
    ctx.renamed_to
        .get(name)
        .filter(|to| new_imports.contains(**to))
        .copied()
}

/// "renames import A → B" for the dropped names the change renamed, and
/// whatever is left for the caller to word as a removal.
fn import_rename_phrase(pairs: &[String]) -> Option<String> {
    (!pairs.is_empty()).then(|| {
        let refs: Vec<&str> = pairs.iter().map(String::as_str).collect();
        format!("renames import {}", name_list(&refs))
    })
}

fn split_import_renames<'r>(
    rs: &[&'r Removal],
    ctx: &RatCtx,
    my_file: usize,
) -> (Option<String>, Vec<&'r Removal>) {
    let mut pairs: Vec<String> = vec![];
    let mut rest: Vec<&Removal> = vec![];
    for r in rs {
        match (r.kind == RemovalKind::Import)
            .then(|| import_renamed_to(ctx, my_file, &r.name))
            .flatten()
        {
            Some(to) => pairs.push(format!("{} → {to}", r.name)),
            None => rest.push(r),
        }
    }
    // one pair per rename: a name can be dropped twice in one hunk, and
    // "renames import X → Y, X → Y" reads as a bug in the tool
    let mut seen = std::collections::HashSet::new();
    pairs.retain(|p| seen.insert(p.clone()));
    (import_rename_phrase(&pairs), rest)
}

// a direction and a size. A pure deletion of body lines (no tracked
// def/import removed) names its size so a large removal isn't hidden behind
// a blank "change"; comment deletions never reach here, the comment-only
// branch already claims them. Anything else is a hunk the grammar recognises
// nothing in: a `#define` (not a definition since macros left `defines`), a
// continuation line inside a shell command, the prose ahead of an added
// document's first heading.
fn size_rationale(s: &HunkSem, replaced: Option<&str>) -> String {
    let [o0, o1] = s.old_range;
    let lines = |n: usize| format!("{n} line{}", if n == 1 { "" } else { "s" });
    if s.new_empty && o1 >= o0 {
        let n = lines(o1 - o0 + 1);
        // the lines came back next door — see `replaced_nearby`. Counting them
        // as gone is true of this hunk's rows and false of the change.
        return match replaced {
            Some(where_) => format!("replaces {n}, added {where_}"),
            None => format!("removes {n}"),
        };
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
    // A removal is what the hunk did, wherever it sits: "edits Foam.MRFZones"
    // hides a deleted method behind the class that still exists. A hunk with
    // no new side is a pure deletion, which `shape_rationale` says shorter,
    // and a detail longer than a line is one a reviewer reads in the detail
    // layer rather than in place of the container's name.
    if !s.new_empty {
        let removal = s
            .details
            .iter()
            .find(|d| d.starts_with("removes") && d.len() <= DETAIL_MAX);
        if let Some(d) = removal {
            return Some(d.clone());
        }
    }
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
    let mut buckets = BindingBuckets::default();
    for b in &items {
        buckets.push(b);
    }
    Some(join_frags(&buckets.frags()))
}

/// The introduced bindings by scope and by whether anything uses them, each
/// bucket worded on its own.
#[derive(Default)]
struct BindingBuckets<'a> {
    /// (scope, names), scope-sorted before wording
    no_uses_scoped: Vec<(&'a str, Vec<&'a str>)>,
    no_uses_file: Vec<&'a str>,
    used_scoped: Vec<(&'a str, &'a [usize])>,
    used_file: Vec<(&'a str, &'a [usize])>,
}

impl<'a> BindingBuckets<'a> {
    fn push(&mut self, b: &'a BindingUse) {
        match (&b.scope, b.uses.is_empty()) {
            (Some(scope), true) => match self.no_uses_scoped.iter_mut().find(|(sc, _)| sc == scope)
            {
                Some((_, names)) => names.push(&b.name),
                None => self.no_uses_scoped.push((scope.as_str(), vec![&b.name])),
            },
            (None, true) => self.no_uses_file.push(&b.name),
            (Some(_), false) => self.used_scoped.push((&b.name, &b.uses)),
            (None, false) => self.used_file.push((&b.name, &b.uses)),
        }
    }

    fn frags(mut self) -> Vec<String> {
        self.no_uses_scoped.sort_by_key(|(scope, _)| *scope);
        let mut frags: Vec<String> = vec![];
        frags.extend(unused_scoped_frag(&self.no_uses_scoped));
        if !self.no_uses_file.is_empty() {
            // scoped fragments above already say "no uses in {scope}" — when
            // both kinds land in the same rationale (P17 composing with
            // def-side wording), repeating that exact phrase at file scope
            // reads as the per-symbol-fragment defect rationale_bounds.rs
            // guards against, even though it's really two distinct groups.
            // Reword only in that mixed case; the lone-fragment wording (no
            // scoped group alongside it) stays as-is.
            let where_ = if self.no_uses_scoped.is_empty() {
                "no uses in this file"
            } else {
                "unused elsewhere in this file"
            };
            frags.push(format!(
                "adds {}, {where_} — check other files",
                name_list(&self.no_uses_file)
            ));
        }
        if !self.used_scoped.is_empty() {
            frags.push(used_frag(&self.used_scoped, "local "));
        }
        if !self.used_file.is_empty() {
            frags.push(used_frag(&self.used_file, ""));
        }
        frags
    }
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

    let empty = HashMap::new();
    containers
        .iter()
        .flat_map(|container| {
            let old = old_by.get(container).unwrap_or(&empty);
            let new = new_by.get(container).unwrap_or(&empty);
            container_phrases(container.as_deref(), old, new, prose)
        })
        .collect()
}

/// One container's member phrases: "adds a, b to Foo", "removes c from Foo".
fn container_phrases(
    container: Option<&str>,
    old: &HashMap<&str, &str>,
    new: &HashMap<&str, &str>,
    prose: bool,
) -> Vec<String> {
    member_diff(container, old, new)
        .into_iter()
        .filter(|(_, _, names)| !names.is_empty())
        .map(|(verb, prep, names)| {
            let list = name_list(&names);
            let list = if prose && verb != "changes" {
                prose_noun(names.len(), &list)
            } else {
                list
            };
            match container {
                Some(c) => format!("{verb} {list} {prep} {c}"),
                None => format!("{verb} {list}"),
            }
        })
        .collect()
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
