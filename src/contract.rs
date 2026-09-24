//! Contract checks: a definition whose promise to its callers changed in a way
//! no single line shows — it became async, a property became a method — held
//! against the calls to it in the same change, and in any consumer file the
//! caller hands over. Like the arity check, each one speaks only when it can
//! be exact, and only about the callers it was shown.

use crate::extract::{self, DefFacts};
use crate::lang;
use crate::model::{Change, Consumer, FileOut, LedgerEntry, SymbolChange};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

pub(crate) fn run(
    files: &mut [FileOut],
    ledger: &[LedgerEntry],
    changes: &[Change],
    consumers: &[Consumer],
) {
    // a consumer is a file no side of the change touches: a caller, never a
    // definition that changed, so it rides along as an unchanged file
    let callers: Vec<Change> = changes
        .iter()
        .cloned()
        .chain(consumers.iter().map(|c| Change {
            path: c.path.clone(),
            old: Some(c.content.clone()),
            new: Some(c.content.clone()),
            diff: None,
        }))
        .collect();
    let ix = Index::new(&callers);
    check(files, &ix);
    stale_strings(files, ledger, &callers);
    loosened_tests(files, ledger, changes);
    import_cycles(files, changes);
    duplicated_helpers(files, &ix);
}

fn check(files: &mut [FileOut], ix: &Index) {
    let changes = ix.changes;
    for (fi, c) in changes.iter().enumerate() {
        let (Some(spec), Some(old), Some(new)) =
            (spec_for(&c.path), c.old.as_deref(), c.new.as_deref())
        else {
            continue;
        };
        if old == new {
            continue;
        }
        for (note, row) in enum_values_changed(spec, old, new) {
            note_on(files, &c.path, None, (row, row), note);
        }
        let (olds, news) = (ix.old_defs(fi), ix.defs(fi));
        for d in news.iter().filter(|d| {
            d.is_abstract
                && !olds
                    .iter()
                    .any(|o| o.is_abstract && o.name == d.name && o.owner == d.owner)
        }) {
            if let Some(note) = unimplemented_abstract(d, ix) {
                note_on(files, &c.path, None, d.rows, note);
            }
        }
        for (was, now) in pairs(olds, news) {
            let site = Site {
                path: &c.path,
                name: &was.name,
            };
            for note in [
                became_async(&site, was, now, ix),
                getter_flipped(&site, was, now, ix),
                positions_shifted(&site, was, now, ix),
                keywords_dropped(&site, was, now, ix),
                default_changed(&site, was, now, ix),
                overrides_left(&site, was, now, ix),
                handlers_missed(&site, was, now, ix),
                guarantee_dropped(&site, was, now),
            ]
            .into_iter()
            .flatten()
            {
                note_on(files, &c.path, Some(was.rows), now.rows, note);
            }
        }
    }
}

/// The languages these checks know the shapes of: async, properties, keyword
/// arguments, `raise`, `with`, enums, imports by module. Elsewhere the walks
/// cost time — a large c++ change took half as long again — and find nothing.
fn spec_for(path: &str) -> Option<&'static lang::LangSpec> {
    lang::for_path(path).filter(|s| {
        matches!(
            s.name,
            "python" | "xonsh" | "javascript" | "typescript" | "tsx"
        )
    })
}

/// the definition a check is about: its file and name
struct Site<'a> {
    path: &'a str,
    name: &'a str,
}

/// Each callable matched to itself across the two sides by name and class,
/// when that pair is unique on both: two same-named defs cannot be told
/// apart, and a guess here is a wrong note.
fn pairs<'d>(old: &'d [DefFacts], new: &'d [DefFacts]) -> Vec<(&'d DefFacts, &'d DefFacts)> {
    let key = |d: &DefFacts| (d.name.clone(), d.owner.clone());
    let unique = |side: &'d [DefFacts]| {
        let mut seen: HashMap<(String, Option<String>), Option<&'d DefFacts>> = HashMap::new();
        for d in side {
            seen.entry(key(d))
                .and_modify(|v| *v = None)
                .or_insert(Some(d));
        }
        seen
    };
    let (was, now) = (unique(old), unique(new));
    let mut out: Vec<(&DefFacts, &DefFacts)> = was
        .iter()
        .filter_map(|(k, d)| Some(((*d)?, (*now.get(k)?)?)))
        .collect();
    out.sort_by_key(|(d, _)| d.rows);
    out
}

/// `def f` → `async def f`: a caller that does not await now gets a coroutine,
/// which never runs and is always truthy — nothing fails, the work just stops
/// happening.
fn became_async(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    if old.is_async || !new.is_async {
        return None;
    }
    let rows: Vec<String> = callers(site, new.owner.is_some(), ix)
        .filter(|c| c.call.discarded)
        .map(|c| c.at.clone())
        .collect();
    let (shown, more) = listed(&rows)?;
    Some(format!(
        "{} became async; {} call{} never await{} it, so it never runs ({shown}{more})",
        site.name,
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        if rows.len() == 1 { "s" } else { "" },
    ))
}

/// `@property` dropped: `obj.x` still evaluates, to a bound method, so
/// `if obj.x:` is always true and `obj.x + 1` fails far from here. The
/// reverse makes `obj.x()` call the value, which at least raises.
fn getter_flipped(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    let owner = new.owner.as_deref()?;
    if old.getter == new.getter {
        return None;
    }
    // a read left as an attribute of what is now a method, or a call left on
    // what is now an attribute
    let stale_called = new.getter;
    let rows: Vec<String> = ix
        .changes
        .iter()
        .filter_map(|c| Some((c, spec_for(&c.path)?, c.new.as_deref()?)))
        // another file's `obj.x` is this `x` only if that file names the class
        .filter(|(c, spec, new)| {
            c.path == site.path || !extract::identifier_rows(spec, new, owner).is_empty()
        })
        .flat_map(|(c, spec, new)| {
            extract::member_reads(spec, new, site.name, false)
                .into_iter()
                .filter(move |&(_, called)| called == stale_called)
                .map(move |(r, _)| format!("{}:L{}", c.path, r + 1))
        })
        .collect();
    let (shown, more) = listed(&rows)?;
    let (was, now, how) = if stale_called {
        ("a method", "a property", "still call it")
    } else {
        (
            "a property",
            "a method",
            "still read it without calling, getting the bound method",
        )
    };
    Some(format!(
        "{owner}.{} went from {was} to {now}; {} use{} {how} ({shown}{more})",
        site.name,
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
    ))
}

/// A symbol renamed or moved, against the strings in the change that still
/// spell its old dotted path: `mock.patch("pkg.cfg.parse")`, an entry point
/// `pkg.cfg:parse`, `import_module`. No import checks a string, so a patch
/// target left behind patches nothing — or, when the old module still
/// imports the name, patches a copy the code under test never calls.
fn stale_strings(files: &mut [FileOut], ledger: &[LedgerEntry], changes: &[Change]) {
    for e in ledger {
        let (old_path, old_name) = match (e.change, e.from.as_deref()) {
            (SymbolChange::Renamed, Some(from)) => (e.path.as_str(), from),
            (SymbolChange::Moved, Some(from)) => (from, e.name.as_str()),
            _ => continue,
        };
        let module = module_of(old_path);
        let Ok(quoted) = regex::Regex::new(&format!(
            r#"["']([A-Za-z_][\w.]*)[.:]{}["']"#,
            regex::escape(old_name)
        )) else {
            continue;
        };
        let hit: Vec<String> = changes
            .iter()
            .filter_map(|c| Some((c, c.new.as_deref()?)))
            .flat_map(|(c, new)| {
                let module = &module;
                let quoted = &quoted;
                new.lines().enumerate().flat_map(move |(row, line)| {
                    quoted
                        .captures_iter(line)
                        .filter(|m| names_module(&m[1], module))
                        .map(move |m| format!("{}:L{} {}", c.path, row + 1, &m[0]))
                })
            })
            .collect();
        let Some((shown, more)) = listed(&hit) else {
            continue;
        };
        let how = if e.change == SymbolChange::Renamed {
            format!("was renamed from {old_name}")
        } else {
            format!("moved from {old_path}")
        };
        let note = format!(
            "{} {how}; {} string{} still name{} it at the old place, and no import checks a string ({shown}{more})",
            e.name,
            hit.len(),
            plural(hit.len()),
            if hit.len() == 1 { "s" } else { "" },
        );
        if let Some(h) = files
            .iter_mut()
            .flat_map(|f| f.hunks.iter_mut())
            .find(|h| h.id == e.at)
        {
            h.notes.push(note);
        }
    }
}

/// An import added in this change that closes a cycle among the change's
/// files: at import time one of them runs against the other half-loaded,
/// and which one depends on who imports first.
fn import_cycles(files: &mut [FileOut], changes: &[Change]) {
    let modules: Vec<Vec<String>> = changes.iter().map(|c| module_of(&c.path)).collect();
    let mut by_last: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, m) in modules.iter().enumerate() {
        if let Some(last) = m.last() {
            by_last.entry(last.as_str()).or_default().push(i);
        }
    }
    let graph = |side: fn(&Change) -> Option<&str>| -> Vec<Vec<(usize, String)>> {
        changes
            .iter()
            .map(|c| {
                let (Some(spec), Some(text)) = (spec_for(&c.path), side(c)) else {
                    return vec![];
                };
                let mut to: Vec<(usize, String)> = extract::import_bindings(spec, text)
                    .into_iter()
                    .filter_map(|b| b.module)
                    .filter_map(|m| {
                        let wanted = dotted(&m);
                        let last = wanted.rsplit('.').next().unwrap_or(&wanted);
                        let target = *by_last.get(last)?.iter().find(|&&di| {
                            let d = &changes[di];
                            d.path != c.path
                                && d.new.is_some()
                                && names_module(&wanted, &modules[di])
                        })?;
                        Some((target, m))
                    })
                    .collect();
                to.sort();
                to.dedup_by_key(|(t, _)| *t);
                to
            })
            .collect()
    };
    let (before, after) = (graph(|c| c.old.as_deref()), graph(|c| c.new.as_deref()));
    for (a, edges) in after.iter().enumerate() {
        for (b, module) in edges {
            if before[a].iter().any(|(t, _)| t == b) {
                continue;
            }
            let Some(back) = path_between(&after, *b, a) else {
                continue;
            };
            let ring: Vec<&str> = std::iter::once(a)
                .chain(back)
                .map(|i| changes[i].path.as_str())
                .collect();
            let Some(row) = changes[a].new.as_deref().and_then(|t| {
                t.lines()
                    .position(|l| l.contains("import") && l.contains(module.as_str()))
            }) else {
                continue;
            };
            note_on(
                files,
                &changes[a].path,
                None,
                (row, row),
                format!(
                    "this import closes a cycle: {}; at import time one of them runs against the other half-loaded",
                    ring.join(" → ")
                ),
            );
        }
    }
}

/// an import's module as dotted parts: `..pkg.util` and `./lib/util.js` both
/// end in `util`, which is what a path is matched on
fn dotted(module: &str) -> String {
    let m = module.trim_start_matches(['.', '/']);
    let m = [".js", ".ts", ".jsx", ".tsx", ".mjs", ".py"]
        .iter()
        .find_map(|x| m.strip_suffix(x))
        .unwrap_or(m);
    m.replace("../", "").replace('/', ".")
}

/// the files walked from `from` to `to`, `to` included, breadth first
fn path_between(graph: &[Vec<(usize, String)>], from: usize, to: usize) -> Option<Vec<usize>> {
    let mut prev: Vec<Option<usize>> = vec![None; graph.len()];
    let mut seen = vec![false; graph.len()];
    let mut queue = std::collections::VecDeque::from([from]);
    seen[from] = true;
    while let Some(n) = queue.pop_front() {
        if n == to {
            let mut walk = vec![to];
            while let Some(p) = prev[*walk.last()?] {
                walk.push(p);
            }
            walk.reverse();
            return Some(walk);
        }
        for (m, _) in &graph[n] {
            if !seen[*m] {
                seen[*m] = true;
                prev[*m] = Some(n);
                queue.push_back(*m);
            }
        }
    }
    None
}

/// Smaller than this and two bodies agree by accident: `return None`.
const DUPLICATE_MIN_LINES: usize = 3;
/// How alike two new bodies must be, by lines, to read as one copied.
const DUPLICATE_RATIO: f32 = 0.9;

/// The same function added to two files in one change: copy-paste, which
/// the next fix will reach only one of.
fn duplicated_helpers(files: &mut [FileOut], ix: &Index) {
    let changes = ix.changes;
    let mut added: Vec<(usize, &DefFacts, String)> = vec![];
    for (fi, c) in changes.iter().enumerate() {
        let Some(new) = c.new.as_deref() else {
            continue;
        };
        if lang::is_test_path(&c.path) {
            continue;
        }
        let old = ix.old_defs(fi);
        let lines: Vec<&str> = new.lines().collect();
        for d in ix.defs(fi) {
            let fresh = !old.iter().any(|o| o.name == d.name && o.owner == d.owner);
            if fresh && d.owner.is_none() && d.rows.1 + 1 - d.rows.0 >= DUPLICATE_MIN_LINES {
                let body = lines
                    .get(d.rows.0..=d.rows.1.min(lines.len().saturating_sub(1)))
                    .map_or(String::new(), |b| b.join("\n"));
                added.push((fi, d, body));
            }
        }
    }
    for (i, (fi, d, body)) in added.iter().enumerate() {
        let twin = added[..i].iter().find(|(gi, e, other)| {
            gi != fi
                && e.name == d.name
                && similar::TextDiff::from_lines(other.as_str(), body.as_str()).ratio()
                    >= DUPLICATE_RATIO
        });
        if let Some((gi, e, _)) = twin {
            note_on(
                files,
                &changes[*fi].path,
                None,
                d.rows,
                format!(
                    "{} is also added, nearly verbatim, in {}:L{}; the next fix to one will miss the other",
                    d.name,
                    changes[*gi].path,
                    e.rows.0 + 1
                ),
            );
        }
    }
}

/// A test that lost assertions, or gained a skip, in the same change that
/// edits code it exercises. Only a diff sees "the test was edited to pass";
/// sometimes the old expectation was the wrong one, so this asks rather than
/// judges.
fn loosened_tests(files: &mut [FileOut], ledger: &[LedgerEntry], changes: &[Change]) {
    let edited: Vec<&str> = ledger
        .iter()
        .filter(|e| {
            matches!(e.change, SymbolChange::Signature | SymbolChange::Body)
                && !lang::is_test_path(&e.path)
        })
        .map(|e| e.name.as_str())
        .collect();
    if edited.is_empty() {
        return;
    }
    let assertion = regex::Regex::new(r"\bassert|\bexpect\(|\bt\.(Error|Fatal|Fail)|\.should\b")
        .expect("static pattern");
    let skip = regex::Regex::new(
        r"pytest\.skip|mark\.(skip|xfail)|unittest\.skip|\b(it|test|describe)\.skip\(|\bx(it|describe)\(|#\[ignore\]|\bt\.Skip",
    )
    .expect("static pattern");
    for (c, f) in changes.iter().zip(files.iter_mut()) {
        if !lang::is_test_path(&c.path) {
            continue;
        }
        let (Some(spec), Some(old), Some(new)) =
            (lang::for_path(&c.path), c.old.as_deref(), c.new.as_deref())
        else {
            continue;
        };
        let exercised: Vec<&str> = edited
            .iter()
            .copied()
            .filter(|n| !extract::identifier_rows(spec, new, n).is_empty())
            .collect();
        let Some(first) = exercised.first() else {
            continue;
        };
        let (old_lines, new_lines): (Vec<&str>, Vec<&str>) =
            (old.lines().collect(), new.lines().collect());
        for h in f.hunks.iter_mut() {
            let removed = rows(&old_lines, h.old_range);
            let added = rows(&new_lines, h.new_range);
            let count =
                |ls: &[&str], re: &regex::Regex| ls.iter().filter(|l| re.is_match(l)).count();
            let dropped = count(&removed, &assertion).saturating_sub(count(&added, &assertion));
            let skipped = count(&added, &skip) > count(&removed, &skip);
            let what = match (dropped, skipped) {
                (0, false) => continue,
                (0, true) => "adds a skip".to_string(),
                (n, false) => format!("drops {n} assertion{}", plural(n)),
                (n, true) => format!("drops {n} assertion{} and adds a skip", plural(n)),
            };
            h.notes.push(format!(
                "this test {what} in the same change that edits {first}{}, which it exercises — check it was not loosened to pass",
                if exercised.len() > 1 {
                    format!(" (and {} more)", exercised.len() - 1)
                } else {
                    String::new()
                },
            ));
        }
    }
}

/// the lines a 1-based inclusive hunk range covers; an empty range is none
fn rows<'a>(lines: &[&'a str], r: [usize; 2]) -> Vec<&'a str> {
    if r[1] < r[0] || r[0] == 0 {
        return vec![];
    }
    lines
        .get(r[0] - 1..r[1].min(lines.len()))
        .map_or(vec![], <[&str]>::to_vec)
}

/// `src/pkg/cfg.py` → `[src, pkg, cfg]`; a package's `__init__` is the package
fn module_of(path: &str) -> Vec<String> {
    let stem = path.rsplit_once('.').map_or(path, |(s, _)| s);
    let mut parts: Vec<String> = stem.split('/').map(str::to_string).collect();
    if parts.last().is_some_and(|p| p == "__init__") {
        parts.pop();
    }
    parts
}

/// Does a dotted prefix written in a string name `module`? It may leave off
/// leading parts (`pkg.cfg` for `src/pkg/cfg.py`), never trailing ones.
fn names_module(dotted: &str, module: &[String]) -> bool {
    let parts: Vec<&str> = dotted.split('.').collect();
    parts.len() <= module.len()
        && module[module.len() - parts.len()..]
            .iter()
            .zip(&parts)
            .all(|(a, b)| a == b)
}

/// A call in the change to the definition at `site`, and whether it reads
/// exactly as it did before the change — a call the author left alone.
struct Caller {
    file: usize,
    at: String,
    call: extract::CallFacts,
    untouched: bool,
}

/// What the checks ask of the whole change, gathered once: a big change has
/// hundreds of edited definitions, and each rescanning every file for its
/// callers made this pass quadratic.
struct Index<'a> {
    changes: &'a [Change],
    /// (callee, through `self.`) → its calls
    calls: OnceCell<HashMap<(String, bool), Vec<Caller>>>,
    /// every class, with its file
    classes: OnceCell<Vec<(usize, extract::ClassFacts)>>,
    /// per file, each side's callables
    defs: Vec<OnceCell<Vec<DefFacts>>>,
    old_defs: Vec<OnceCell<Vec<DefFacts>>>,
}

impl<'a> Index<'a> {
    fn new(changes: &'a [Change]) -> Self {
        Index {
            changes,
            calls: OnceCell::new(),
            classes: OnceCell::new(),
            defs: changes.iter().map(|_| OnceCell::new()).collect(),
            old_defs: changes.iter().map(|_| OnceCell::new()).collect(),
        }
    }

    fn sources(
        &self,
    ) -> impl Iterator<Item = (usize, &'a Change, &'static lang::LangSpec, &'a str)> {
        self.changes
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((i, c, spec_for(&c.path)?, c.new.as_deref()?)))
    }

    fn calls(&self) -> &HashMap<(String, bool), Vec<Caller>> {
        self.calls.get_or_init(|| {
            let mut out: HashMap<(String, bool), Vec<Caller>> = HashMap::new();
            for (file, c, spec, new) in self.sources() {
                let before: HashSet<(String, bool, String)> = c
                    .old
                    .as_deref()
                    .map_or(vec![], |old| extract::all_calls(spec, old))
                    .into_iter()
                    .map(|(n, r, k)| (n, r, k.text))
                    .collect();
                for (name, recv, call) in extract::all_calls(spec, new) {
                    let untouched = before.contains(&(name.clone(), recv, call.text.clone()));
                    out.entry((name, recv)).or_default().push(Caller {
                        file,
                        at: format!("{}:L{}", c.path, call.row + 1),
                        call,
                        untouched,
                    });
                }
            }
            out
        })
    }

    fn defs(&self, file: usize) -> &[DefFacts] {
        self.defs[file].get_or_init(|| self.facts(file, self.changes[file].new.as_deref()))
    }

    fn old_defs(&self, file: usize) -> &[DefFacts] {
        self.old_defs[file].get_or_init(|| self.facts(file, self.changes[file].old.as_deref()))
    }

    fn facts(&self, file: usize, text: Option<&str>) -> Vec<DefFacts> {
        match (spec_for(&self.changes[file].path), text) {
            (Some(spec), Some(text)) => extract::def_facts(spec, text),
            _ => vec![],
        }
    }

    /// every class in the change deriving from `base` directly, with its file
    fn subclasses<'s>(
        &'s self,
        base: &'s str,
    ) -> impl Iterator<Item = (usize, &'s extract::ClassFacts)> {
        self.classes
            .get_or_init(|| {
                self.sources()
                    .flat_map(|(i, _, spec, new)| {
                        extract::class_facts(spec, new)
                            .into_iter()
                            .map(move |k| (i, k))
                    })
                    .collect()
            })
            .iter()
            .filter(move |(_, k)| k.bases.iter().any(|b| b == base))
            .map(|(i, k)| (*i, k))
    }
}

fn callers<'i>(site: &Site, method: bool, ix: &'i Index) -> impl Iterator<Item = &'i Caller> {
    let path = site.path.to_string();
    ix.calls()
        .get(&(site.name.to_string(), method))
        .into_iter()
        .flatten()
        // `self.f()` only reaches this `f` from inside its own file
        .filter(move |c| !method || ix.changes[c.file].path == path)
}

fn positional(d: &DefFacts) -> Vec<&str> {
    d.params
        .iter()
        .filter(|p| p.slot == extract::Slot::Positional)
        .map(|p| p.name.as_str())
        .collect()
}

/// A parameter inserted or reordered before the end: every call left as it
/// was that passes that far by position now hands its values to the wrong
/// parameters, and nothing fails when the types happen to agree.
fn positions_shifted(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    let (was, now) = (positional(old), positional(new));
    // the first old parameter still there but at another position; a name
    // that is simply gone was renamed or dropped, which moves nothing
    let k = was
        .iter()
        .enumerate()
        .position(|(j, a)| now.iter().position(|b| b == a).is_some_and(|i| i != j))?;
    let hit: Vec<String> = callers(site, new.owner.is_some(), ix)
        .filter(|c| c.untouched && !c.call.spread && c.call.positional > k)
        .map(|c| c.at.clone())
        .collect();
    let (shown, more) = listed(&hit)?;
    Some(format!(
        "{}'s parameters went from ({}) to ({}); {} call{} left as {} pass{} {} or more by position, which now land on different parameters ({shown}{more})",
        site.name,
        was.join(", "),
        now.join(", "),
        hit.len(),
        plural(hit.len()),
        if hit.len() == 1 { "it was" } else { "they were" },
        if hit.len() == 1 { "es" } else { "" },
        k + 1,
    ))
}

/// A parameter renamed or removed: a call still passing it by name raises,
/// but only when that line runs.
fn keywords_dropped(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    if new
        .params
        .iter()
        .any(|p| p.slot == extract::Slot::AnyKeyword)
    {
        return None;
    }
    let gone: Vec<&str> = old
        .params
        .iter()
        .map(|p| p.name.as_str())
        .filter(|n| new.params.iter().all(|p| p.name != *n))
        .collect();
    if gone.is_empty() {
        return None;
    }
    let mut passed: Vec<&str> = vec![];
    let hit: Vec<String> = callers(site, new.owner.is_some(), ix)
        .filter(|c| {
            let mut any = false;
            for k in gone
                .iter()
                .filter(|k| c.call.keywords.iter().any(|w| w == *k))
            {
                any = true;
                if !passed.contains(k) {
                    passed.push(k);
                }
            }
            any
        })
        .map(|c| c.at.clone())
        .collect();
    let (shown, more) = listed(&hit)?;
    Some(format!(
        "{} no longer takes {}; {} call{} still pass{} it by name ({shown}{more})",
        site.name,
        passed
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", "),
        hit.len(),
        plural(hit.len()),
        if hit.len() == 1 { "es" } else { "" },
    ))
}

/// A default changed: every call that leaves the parameter out changes
/// behaviour without a line of its own in the diff.
fn default_changed(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    let now = positional(new);
    let (p, was, is) = new.params.iter().find_map(|p| {
        let before = old.params.iter().find(|o| o.name == p.name)?;
        match (&before.default, &p.default) {
            (Some(a), Some(b)) if a != b => Some((p, a, b)),
            _ => None,
        }
    })?;
    let index = now.iter().position(|n| *n == p.name);
    let hit: Vec<String> = callers(site, new.owner.is_some(), ix)
        .filter(|c| {
            !c.call.spread
                && !c.call.keywords.contains(&p.name)
                && index.is_none_or(|i| c.call.positional <= i)
        })
        .map(|c| c.at.clone())
        .collect();
    let (shown, more) = listed(&hit)?;
    Some(format!(
        "{}'s default for `{}` changed from {was} to {is}; {} call{} leave{} it out and get the new value ({shown}{more})",
        site.name,
        p.name,
        hit.len(),
        plural(hit.len()),
        if hit.len() == 1 { "s" } else { "" },
    ))
}

/// It raised `KeyError` and now raises `MissingKey`: a caller that caught
/// the old type lets the new one through, and only an error path shows it.
fn handlers_missed(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    let gone: Vec<&String> = old
        .raises
        .iter()
        .filter(|r| !new.raises.contains(r))
        .collect();
    let came: Vec<&String> = new
        .raises
        .iter()
        .filter(|r| !old.raises.contains(r))
        .collect();
    if gone.is_empty() || came.is_empty() {
        return None;
    }
    let method = new.owner.is_some();
    let hit: Vec<String> = ix
        .changes
        .iter()
        .filter_map(|c| Some((c, spec_for(&c.path)?, c.new.as_deref()?)))
        .filter(|(c, ..)| !method || c.path == site.path)
        .flat_map(|(c, spec, text)| {
            extract::guarded_calls(spec, text, site.name, method)
                .into_iter()
                .filter(|(_, caught)| {
                    let catches = |t: &str| caught.iter().any(|x| x == t);
                    gone.iter().any(|g| catches(g))
                        && !came.iter().any(|t| catches(t))
                        && !["*", "Exception", "BaseException"]
                            .iter()
                            .any(|t| catches(t))
                })
                .map(move |(r, _)| format!("{}:L{}", c.path, r + 1))
        })
        .collect();
    let (shown, more) = listed(&hit)?;
    let list = |v: &[&String]| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
    Some(format!(
        "{} now raises {} instead of {}; {} call{} still catch{} only the old type ({shown}{more})",
        site.name,
        list(&came),
        list(&gone),
        hit.len(),
        plural(hit.len()),
        if hit.len() == 1 { "es" } else { "" },
    ))
}

/// An enum member's value changed: every stored, sent or serialized copy of
/// the old value stops mapping back to the member, and no line says so.
fn enum_values_changed(
    spec: &'static lang::LangSpec,
    old: &str,
    new: &str,
) -> Vec<(String, usize)> {
    let before = extract::enum_values(spec, old);
    extract::enum_values(spec, new)
        .into_iter()
        .filter_map(|(owner, name, value, row)| {
            let (.., was, _) = before.iter().find(|(o, n, ..)| *o == owner && *n == name)?;
            (*was != value).then(|| {
                (
                    format!(
                        "{owner}.{name}'s value changed from {was} to {value}; anything stored or sent as {was} no longer maps back to it"
                    ),
                    row,
                )
            })
        })
        .collect()
}

/// A `with` or an `await` gone from a body that is otherwise still there: the
/// code got shorter and quietly lost a lock, a transaction, a cleanup, or the
/// wait for the work to finish.
fn guarantee_dropped(site: &Site, old: &DefFacts, new: &DefFacts) -> Option<String> {
    let mut lost = vec![];
    if new.withs.len() < old.withs.len() {
        lost.extend(
            old.withs
                .iter()
                .filter(|w| !new.withs.contains(w))
                .map(|w| format!("no longer enters `{w}`")),
        );
    }
    lost.extend(
        old.awaited
            .iter()
            .filter(|c| !new.awaited.contains(c) && new.not_awaited.contains(c))
            .map(|c| format!("calls `{c}` without awaiting it")),
    );
    (!lost.is_empty()).then(|| format!("{} {}", site.name, lost.join(" and ")))
}

/// A base method's parameters changed and an override in the change still
/// takes the old ones: a call through the base now hands it arguments it
/// does not expect.
fn overrides_left(site: &Site, old: &DefFacts, new: &DefFacts, ix: &Index) -> Option<String> {
    let base = new.owner.as_deref()?;
    let names = |d: &DefFacts| d.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let (was, now) = (names(old), names(new));
    if was == now {
        return None;
    }
    let hit: Vec<String> = ix
        .subclasses(base)
        .filter_map(|(file, k)| {
            let o = ix
                .defs(file)
                .iter()
                .find(|d| d.name == site.name && d.owner.as_deref() == Some(&k.name))?;
            (names(o) == was).then(|| {
                format!(
                    "{}.{} {}:L{}",
                    k.name,
                    site.name,
                    ix.changes[file].path,
                    o.rows.0 + 1
                )
            })
        })
        .collect();
    let (shown, more) = listed(&hit)?;
    Some(format!(
        "{base}.{}'s parameters went from ({}) to ({}); {} override{} still take{} the old ones ({shown}{more})",
        site.name,
        was.join(", "),
        now.join(", "),
        hit.len(),
        plural(hit.len()),
        if hit.len() == 1 { "s" } else { "" },
    ))
}

/// A new abstract method: a subclass that does not define it can no longer
/// be instantiated, and that fails only where one is made.
fn unimplemented_abstract(d: &DefFacts, ix: &Index) -> Option<String> {
    let base = d.owner.as_deref()?;
    let hit: Vec<String> = ix
        .subclasses(base)
        .filter(|(file, k)| {
            !ix.defs(*file)
                .iter()
                .any(|m| m.name == d.name && m.owner.as_deref() == Some(&k.name))
        })
        .map(|(file, k)| format!("{} {}:L{}", k.name, ix.changes[file].path, k.row + 1))
        .collect();
    let (shown, more) = listed(&hit)?;
    Some(format!(
        "{base} gained abstract method {}; {} subclass{} do{} not define it and can no longer be instantiated ({shown}{more})",
        d.name,
        hit.len(),
        if hit.len() == 1 { "" } else { "es" },
        if hit.len() == 1 { "es" } else { "" },
    ))
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// the first three locations and a count of the rest, or `None` for none
fn listed(rows: &[String]) -> Option<(String, String)> {
    if rows.is_empty() {
        return None;
    }
    let more = if rows.len() > 3 {
        format!(" and {} more", rows.len() - 3)
    } else {
        String::new()
    };
    Some((rows[..rows.len().min(3)].join(", "), more))
}

/// A note belongs on the hunk that changed the definition — that is where a
/// reviewer is standing when the question arises.
fn note_on(
    files: &mut [FileOut],
    path: &str,
    was: Option<(usize, usize)>,
    now: (usize, usize),
    note: String,
) {
    // hunk ranges are 1-based and inclusive; an empty side ends before it starts
    let meets = |r: [usize; 2], (s, e): (usize, usize)| r[1] >= r[0] && r[0] <= e + 1 && s < r[1];
    if let Some(h) = files
        .iter_mut()
        .filter(|f| f.path == path)
        .flat_map(|f| f.hunks.iter_mut())
        .find(|h| was.is_some_and(|w| meets(h.old_range, w)) || meets(h.new_range, now))
    {
        h.notes.push(note);
    }
}
