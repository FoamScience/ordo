//! The change as a whole, in one label, a verdict and two sentences: what the
//! hunks' facts add up to, read off them by fixed rules rather than guessed.
//!
//! A hunk counts towards "behaviour-preserving" only on evidence the hunk's
//! own lines give: whitespace that separates no tokens, a comment, a rename
//! whose substitution turns every removed line into its added one, a moved
//! definition whose lines left another file, an extraction whose lines left
//! the definition it came from. Anything else leaves the verdict unknown
//! unless a recognised edit pattern says behaviour changed.
use crate::extract::{HunkSem, RawHunk};
use crate::intent::Intent;
use crate::lang;
use crate::model::{
    Category, Change, Label, LedgerEntry, RemovalKind, Summary, SymbolChange, Verdict,
};
use crate::refine::Role;
use crate::{FileChanges, PerFileHunks};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

/// What a path is, by where it lives and what it is written in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PathKind {
    Docs,
    Test,
    Chore,
    Code,
}

/// The documentation a repository's root carries whatever its layout.
const DOC_NAMES: &[&str] = &[
    "readme",
    "changelog",
    "changes",
    "license",
    "licence",
    "contributing",
    "authors",
    "notice",
    "history",
    "security",
    "code_of_conduct",
];
const DOC_DIRS: &[&str] = &["doc", "docs", "documentation"];
/// not `txt`: `requirements.txt` and its kin are dependencies, not prose
const DOC_EXTS: &[&str] = &["adoc", "rst", "md", "markdown"];
/// build and ci configuration that has no grammar of its own
const CHORE_DIRS: &[&str] = &[".github", ".gitlab", ".circleci", ".husky", ".devcontainer"];
const BUILD_LANGS: &[&str] = &["make", "cmake"];
/// languages where a line's indentation is its block: a reindent there moves
/// a statement in or out of one
const OFFSIDE: &[&str] = &["python", "xonsh", "yaml", "make"];

fn path_kind(path: &str) -> PathKind {
    let spec = lang::for_path(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let in_dir = |dirs: &[&str]| path.split('/').rev().skip(1).any(|d| dirs.contains(&d));
    if spec.is_some_and(|s| s.prose) || spec.is_none() && DOC_NAMES.contains(&stem.as_str()) {
        PathKind::Docs
    } else if lang::is_test_path(path) {
        PathKind::Test
    } else if spec.is_none() && (in_dir(DOC_DIRS) || DOC_EXTS.contains(&ext)) {
        PathKind::Docs
    } else if spec.is_none_or(|s| s.data || BUILD_LANGS.contains(&s.name))
        || in_dir(CHORE_DIRS)
        || lang::is_generated_path(path)
    {
        PathKind::Chore
    } else {
        PathKind::Code
    }
}

/// What one hunk is, as far as its lines prove.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// changes no token: whitespace, blank lines, a copyright notice
    Format,
    Comment,
    Rename,
    Move,
    /// a definition moved, whole, elsewhere in its own file
    Reorder,
    Extract,
    /// a literal replaced by the constant that holds it, or that constant's
    /// definition
    Constant,
    /// a pattern that changes what the code does
    Pattern,
    /// a literal or config value replaced by another
    Value,
    /// nothing above explains it
    Edit,
}

impl Kind {
    fn preserves(self) -> bool {
        matches!(
            self,
            Kind::Format
                | Kind::Comment
                | Kind::Rename
                | Kind::Move
                | Kind::Reorder
                | Kind::Extract
                | Kind::Constant
        )
    }
}

/// Edit patterns that tighten or correct what code does — the shape of a fix.
fn fix_shaped(i: &Intent) -> bool {
    matches!(
        i,
        Intent::Guard { .. }
            | Intent::WrapsTry { .. }
            | Intent::Condition { .. }
            | Intent::Boundary { .. }
            | Intent::Inverts { .. }
    )
}

/// Edit patterns whose code does something it did not before. A literal
/// named as a constant and a parameter appended with a default keep every
/// existing caller's behaviour, as far as the hunk shows.
fn changes_behaviour(i: &Intent) -> bool {
    !matches!(i, Intent::NamesLiteral { .. } | Intent::DefaultParam { .. })
}

/// What the whole change knows about names and lines, for the per-hunk checks.
struct Evidence<'a> {
    /// old name → new name, every rename in the change that gives an old
    /// name one new one: two files renaming `foo` differently say nothing
    /// about which a reference elsewhere followed
    renames: HashMap<&'a str, &'a str>,
    /// per file, old name → new name for the renames made in it, which its own
    /// lines follow whatever another file did
    renames_in: Vec<HashMap<&'a str, &'a str>>,
    /// every distinct (old, new) rename in the change
    rename_pairs: Vec<(&'a str, &'a str)>,
    moved: HashSet<&'a str>,
    /// per file, the definitions that left it (old side) and arrived in it
    /// (new side) from another file
    moved_here: Vec<[HashSet<&'a str>; 2]>,
    /// constant name → the literal it holds, for each literal the change
    /// replaced by a constant whose every binding in the change holds it
    constants: HashMap<&'a str, &'a str>,
    /// extracted name → the definition it came out of
    extracted: HashMap<&'a str, &'a str>,
    /// removed lines (renames applied) per file, and added lines per file
    removed: Vec<HashSet<String>>,
    added: Vec<HashSet<String>>,
    /// the lines each extracted definition's hunks added, and the lines the
    /// hunks inside each definition it came out of removed
    extracted_lines: HashMap<(usize, &'a str), HashSet<String>>,
    source_lines: HashMap<(usize, &'a str), HashSet<String>>,
    changes: &'a [Change],
    /// per file, what each side's syntax tree says about its rows. Parsed
    /// only for a file one of whose hunks needs it.
    sides: Vec<OnceCell<[Side; 2]>>,
    /// per file, the definitions whose place may change and whose lines did
    /// not
    intact: Vec<OnceCell<HashSet<String>>>,
}

/// One side of a file, row by row (1-based).
#[derive(Default)]
struct Side {
    /// inside a string that spans lines: whitespace there is text
    strings: HashSet<usize>,
    /// nothing but a comment
    comments: HashSet<usize>,
    /// nothing but a decorator or attribute on the definition below it
    decorations: HashSet<usize>,
    defs: Vec<Def>,
}

struct Def {
    name: String,
    kind: String,
    rows: [usize; 2],
    /// the nearest definition it lies in, by its index in `Side::defs`
    parent: Option<usize>,
    /// inside another definition
    nested: bool,
    /// inside a function: no caller outside it can reach it
    local: bool,
}

struct Facts<'a> {
    /// index into the change's files
    fi: usize,
    path: &'a str,
    kind: PathKind,
    hunks: Vec<(Kind, &'a HunkSem)>,
}

pub(crate) fn summarize(
    changes: &[Change],
    hunks: &[PerFileHunks],
    changed: &[FileChanges],
    ledger: &[LedgerEntry],
) -> Option<Summary> {
    let lines: Vec<[Vec<&str>; 2]> = changes
        .iter()
        .map(|c| {
            [
                c.old.as_deref().unwrap_or("").lines().collect(),
                c.new.as_deref().unwrap_or("").lines().collect(),
            ]
        })
        .collect();
    // a key renamed or moved in a config or a section in prose is not a
    // definition code can call: only code files give evidence
    let code: Vec<bool> = changes
        .iter()
        .map(|c| lang::for_path(&c.path).is_some_and(|s| !s.prose && !s.data))
        .collect();
    let code_changes = || {
        changed
            .iter()
            .zip(&code)
            .filter(|(_, c)| **c)
            .map(|(f, _)| f)
    };
    let mut rename_pairs: Vec<(&str, &str)> = code_changes()
        .flat_map(|c| c.rename.iter().map(|(n, o)| (o.as_str(), n.as_str())))
        .collect();
    rename_pairs.sort_unstable();
    rename_pairs.dedup();
    let mut renames: HashMap<&str, Option<&str>> = HashMap::new();
    for &(o, n) in &rename_pairs {
        renames
            .entry(o)
            .and_modify(|e| {
                if *e != Some(n) {
                    *e = None;
                }
            })
            .or_insert(Some(n));
    }
    let mut ev = Evidence {
        renames: renames
            .into_iter()
            .filter_map(|(o, n)| Some((o, n?)))
            .collect(),
        renames_in: changed
            .iter()
            .zip(&code)
            .map(|(c, code)| match code {
                true => c
                    .rename
                    .iter()
                    .map(|(n, o)| (o.as_str(), n.as_str()))
                    .collect(),
                false => HashMap::new(),
            })
            .collect(),
        rename_pairs,
        moved: code_changes()
            .flat_map(|c| {
                c.moved_in.keys().map(String::as_str).chain(
                    c.removals
                        .iter()
                        .filter(|r| matches!(r.kind, RemovalKind::MovedTo(_)))
                        .map(|r| r.name.as_str()),
                )
            })
            .collect(),
        moved_here: changed
            .iter()
            .zip(&code)
            .map(|(c, code)| match code {
                true => [
                    c.removals
                        .iter()
                        .filter(|r| matches!(r.kind, RemovalKind::MovedTo(_)))
                        .map(|r| r.name.as_str())
                        .collect(),
                    c.moved_in.keys().map(String::as_str).collect(),
                ],
                false => Default::default(),
            })
            .collect(),
        constants: constants(changes, hunks, &code),
        extracted: code_changes()
            .flat_map(|c| c.relocated.iter().map(|(n, f)| (n.as_str(), f.as_str())))
            .collect(),
        removed: vec![],
        added: vec![],
        extracted_lines: HashMap::new(),
        source_lines: HashMap::new(),
        changes,
        sides: changes.iter().map(|_| OnceCell::new()).collect(),
        intact: changes.iter().map(|_| OnceCell::new()).collect(),
    };
    let sources: HashSet<&str> = ev.extracted.values().copied().collect();
    for (fi, f) in hunks.iter().enumerate() {
        let [old, new] = &lines[fi];
        let (mut rm, mut ad) = (HashSet::new(), HashSet::new());
        for (h, s) in f.raw.iter().zip(&f.sem).filter(|_| code[fi]) {
            let (o, n): (Vec<String>, Vec<String>) = (
                side(old, h.old_range)
                    .into_iter()
                    .map(|l| ev.renamed(fi, l))
                    .collect(),
                side(new, h.new_range)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            );
            for d in &s.defines {
                if let Some((nm, _)) = ev.extracted.get_key_value(d.as_str()) {
                    ev.extracted_lines
                        .entry((fi, nm))
                        .or_default()
                        .extend(n.iter().cloned());
                }
            }
            // `inside`: the enclosing name itself, or one of its qualified tails
            if let Some(e) = s.enclosing.as_deref() {
                let tails =
                    std::iter::once(e).chain(e.match_indices('.').map(|(i, _)| &e[i + 1..]));
                for from in tails.filter_map(|t| sources.get(t)) {
                    ev.source_lines
                        .entry((fi, from))
                        .or_default()
                        .extend(o.iter().cloned());
                }
            }
            rm.extend(o);
            ad.extend(n);
        }
        ev.removed.push(rm);
        ev.added.push(ad);
    }
    let files: Vec<Facts> = changes
        .iter()
        .enumerate()
        .filter(|(fi, _)| !hunks[*fi].sem.is_empty())
        .map(|(fi, c)| {
            let f = &hunks[fi];
            let spec = lang::for_path(&c.path);
            let hunks = f
                .raw
                .iter()
                .zip(&f.sem)
                .enumerate()
                .map(|(li, (h, s))| {
                    let kind = if f.degraded {
                        Kind::Edit
                    } else if f.comment[li] {
                        Kind::Comment
                    } else {
                        let offside = spec.is_some_and(|s| OFFSIDE.contains(&s.name));
                        ev.kind(fi, h, s, &lines[fi], offside, code[fi])
                    };
                    (kind, s)
                })
                .collect();
            Facts {
                fi,
                path: &c.path,
                kind: path_kind(&c.path),
                hunks,
            }
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    let verdict = verdict(&files);
    let added: Vec<&str> = ledger
        .iter()
        .filter(|e| e.change == SymbolChange::Added && path_kind(&e.path) == PathKind::Code)
        .filter(|e| !e.name.starts_with('_'))
        .map(|e| e.name.as_str())
        .collect();
    let label = label(&files, &added);
    let exposed = match verdict {
        Verdict::Preserving => ev.exposed_renames(changed, &code, &lines),
        _ => vec![],
    };
    let sentences = vec![
        what(label, &files, &added, &ev, &lines),
        why(verdict, &files, &exposed),
    ];
    Some(Summary {
        label,
        verdict,
        sentences,
    })
}

/// The label, by the first rule that holds — what the files are, then what
/// the code hunks did.
fn label(files: &[Facts], added: &[&str]) -> Label {
    let any = |k: PathKind| files.iter().any(|f| f.kind == k);
    let code: Vec<Kind> = code_hunks(files).map(|(k, _)| k).collect();
    let tested = any(PathKind::Test);
    let fix = code_hunks(files).any(|(k, s)| fix_pattern(k, s).is_some());
    let commented = code.contains(&Kind::Comment)
        && code
            .iter()
            .all(|k| *k == Kind::Comment || *k == Kind::Format);
    match () {
        _ if !any(PathKind::Code) && !tested && !any(PathKind::Chore) => Label::Docs,
        _ if !any(PathKind::Code) && tested => Label::Test,
        _ if !any(PathKind::Code) => Label::Chore,
        _ if commented && tested => Label::Test,
        _ if commented => Label::Docs,
        _ if code.iter().all(|k| k.preserves()) => Label::Refactor,
        _ if fix => Label::FixCandidate,
        _ if !added.is_empty() => Label::Feature,
        _ if tested => Label::FixCandidate,
        _ => Label::Change,
    }
}

/// The fix-shaped pattern a hunk is, when that is what it is.
fn fix_pattern(k: Kind, s: &HunkSem) -> Option<&Intent> {
    s.intent
        .as_ref()
        .filter(|i| k == Kind::Pattern && fix_shaped(i))
}

fn code_hunks<'a>(files: &'a [Facts]) -> impl Iterator<Item = (Kind, &'a HunkSem)> + 'a {
    files
        .iter()
        .filter(|f| f.kind == PathKind::Code)
        .flat_map(|f| f.hunks.iter().copied())
}

/// Preserving only when every hunk, in every file, proves it; changed when
/// code or configuration shows a pattern or a value that changes behaviour;
/// otherwise unknown.
fn verdict(files: &[Facts]) -> Verdict {
    let all = files
        .iter()
        .all(|f| f.kind == PathKind::Docs || f.hunks.iter().all(|(k, _)| k.preserves()));
    let changes = files
        .iter()
        .filter(|f| matches!(f.kind, PathKind::Code | PathKind::Chore))
        .flat_map(|f| &f.hunks)
        .any(|(k, _)| matches!(k, Kind::Pattern | Kind::Value));
    if all {
        Verdict::Preserving
    } else if changes {
        Verdict::Changes
    } else {
        Verdict::Unknown
    }
}

impl Evidence<'_> {
    fn kind(
        &self,
        fi: usize,
        h: &RawHunk,
        s: &HunkSem,
        [old, new]: &[Vec<&str>; 2],
        offside: bool,
        code: bool,
    ) -> Kind {
        let (o, n) = (side(old, h.old_range), side(new, h.new_range));
        if s.copyright || o.is_empty() && n.is_empty() {
            return Kind::Format;
        }
        if same_tokens(&o, &n, offside) && !self.in_string(fi, h) {
            return Kind::Format;
        }
        if code
            && !self.rename_pairs.is_empty()
            && o.len() == n.len()
            && o.iter().zip(&n).all(|(a, b)| self.renamed(fi, a) == *b)
        {
            return Kind::Rename;
        }
        if code && s.category == Category::Import {
            return self.import_kind(s);
        }
        let sides = [old.as_slice(), new.as_slice()];
        if code && self.moves(fi, h, sides, &o, &n) {
            return Kind::Move;
        }
        if code && self.reorders(fi, h, sides, &o, &n) {
            return Kind::Reorder;
        }
        if code && self.extracts(fi, s, &o, &n) {
            return Kind::Extract;
        }
        if code && self.names_constant(fi, h, s, new, [&o, &n]) {
            return Kind::Constant;
        }
        if s.intent.as_ref().is_some_and(changes_behaviour) {
            return Kind::Pattern;
        }
        if s.single.as_ref().is_some_and(|c| {
            !c.old.is_empty()
                && !c.new.is_empty()
                && c.roles
                    .iter()
                    .all(|r| matches!(r, Some(Role::Num | Role::Str | Role::Bool)))
        }) {
            return Kind::Value;
        }
        Kind::Edit
    }

    /// `old → new` for each rename of a definition callers outside the
    /// change may reach: nothing in it, or in the language's way of marking
    /// one, keeps it to its own file, module or crate.
    fn exposed_renames(
        &self,
        changed: &[FileChanges],
        code: &[bool],
        lines: &[[Vec<&str>; 2]],
    ) -> Vec<String> {
        let mut out = vec![];
        for (fi, c) in changed.iter().enumerate().filter(|(fi, _)| code[*fi]) {
            let Some(spec) = lang::for_path(&self.changes[fi].path) else {
                continue;
            };
            if c.rename.is_empty() {
                continue;
            }
            let new_lines = &lines[fi][1];
            let defs = &self.sides(fi)[1].defs;
            let mut by_name: HashMap<&str, Vec<&Def>> = HashMap::new();
            for d in defs {
                by_name.entry(d.name.as_str()).or_default().push(d);
            }
            // the names on a line that exports: what a js/ts definition needs
            // to be reachable from outside its module
            let exported: HashSet<&str> = new_lines
                .iter()
                .filter(|l| has_word(l, "export") || l.contains("module.exports"))
                .flat_map(|l| words(l))
                .collect();
            for (new, old) in &c.rename {
                let named = by_name.get(new.as_str()).map_or(&[][..], Vec::as_slice);
                let reachable = named.iter().any(|&d| {
                    // a rust trait's items are as public as the trait
                    let d = match d.parent.map(|p| &defs[p]) {
                        Some(t) if spec.name == "rust" && t.kind == "trait_item" => t,
                        _ => d,
                    };
                    // the definition's own lines up to its name: its
                    // modifiers, wherever its decorations put them
                    let span = new_lines.get(d.rows[0] - 1..d.rows[1]).unwrap_or_default();
                    let end = span
                        .iter()
                        .position(|l| has_word(l, &d.name))
                        .map_or(span.len(), |i| i + 1);
                    !private(spec.name, &d.name, d, &span[..end].join(" "), &exported)
                });
                if reachable {
                    out.push(format!("{old} → {new}"));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn sides(&self, fi: usize) -> &[Side; 2] {
        self.sides[fi].get_or_init(|| {
            let c = &self.changes[fi];
            let side = |text: Option<&str>| {
                lang::for_path(&c.path)
                    .zip(text)
                    .and_then(|(spec, t)| {
                        Some(side_facts(spec, t, lang::parse(spec, t)?.root_node()))
                    })
                    .unwrap_or_default()
            };
            [side(c.old.as_deref()), side(c.new.as_deref())]
        })
    }

    /// Either side of the hunk lies in a string that spans lines.
    fn in_string(&self, fi: usize, h: &RawHunk) -> bool {
        let [old, new] = self.sides(fi);
        let within = |rows: &HashSet<usize>, [a, b]: [usize; 2]| (a..=b).any(|r| rows.contains(&r));
        within(&old.strings, h.old_range) || within(&new.strings, h.new_range)
    }

    /// An import that follows a rename, move or extraction elsewhere in the
    /// change brings in nothing new.
    fn import_kind(&self, s: &HunkSem) -> Kind {
        let known = |n: &String| {
            let n = n.as_str();
            self.moved.contains(n)
                || self.extracted.contains_key(n)
                || self.rename_pairs.iter().any(|(o, v)| *o == n || *v == n)
        };
        if !s.imports.is_empty()
            && s.imports
                .iter()
                .all(|n| known(n) || self.constants.contains_key(n.as_str()))
        {
            Kind::Move
        } else {
            Kind::Edit
        }
    }

    /// A line of file `fi` with every renamed identifier spelled its new way:
    /// as that file renamed it, else as the change renamed it everywhere.
    fn renamed(&self, fi: usize, line: &str) -> String {
        let here = &self.renames_in[fi];
        if self.renames.is_empty() && here.is_empty() {
            return line.to_string();
        }
        let mut out = String::with_capacity(line.len());
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            let w = word.as_str();
            out.push_str(
                here.get(w)
                    .or_else(|| self.renames.get(w))
                    .copied()
                    .unwrap_or(w),
            );
            word.clear();
        };
        // a string that spells the old name is text, not a reference
        let mut quote: Option<char> = None;
        let mut prev = ' ';
        for c in line.chars() {
            if let Some(q) = quote {
                out.push(c);
                if c == q && prev != '\\' {
                    quote = None;
                }
            } else if c.is_alphanumeric() || c == '_' || c == '$' {
                word.push(c);
            } else {
                flush(&mut word, &mut out);
                if matches!(c, '"' | '\'' | '`') {
                    quote = Some(c);
                }
                out.push(c);
            }
            prev = c;
        }
        flush(&mut word, &mut out);
        out
    }

    /// A hunk of nothing but definitions that moved to (or from) another file
    /// and their comments and decorations, whose every line left (or arrived
    /// in) another file of the change. A statement beside them is not part of
    /// the move, even when its line moved too.
    fn moves(&self, fi: usize, h: &RawHunk, lines: [&[&str]; 2], o: &[&str], n: &[&str]) -> bool {
        let names = &self.moved_here[fi];
        if names.iter().all(HashSet::is_empty) {
            return false;
        }
        let elsewhere = |sets: &[HashSet<String>], l: String| {
            sets.iter()
                .enumerate()
                .any(|(fj, set)| fj != fi && set.contains(&l))
        };
        self.made_of(
            fi,
            h,
            lines,
            |k, d| names[k].contains(d.name.as_str()),
            true,
        ) && n.iter().all(|l| elsewhere(&self.removed, l.to_string()))
            && o.iter()
                .all(|l| elsewhere(&self.added, self.renamed(fi, l)))
    }

    /// A hunk of nothing but definitions whose lines are the same on both
    /// sides, and their comments: what it changed is where they stand. A decorator may
    /// register what it decorates, in the order the file runs it: a decorated
    /// definition moving is not proven to keep behaviour.
    fn reorders(
        &self,
        fi: usize,
        h: &RawHunk,
        lines: [&[&str]; 2],
        o: &[&str],
        n: &[&str],
    ) -> bool {
        o.iter()
            .all(|l| self.added[fi].contains(&self.renamed(fi, l)))
            && n.iter().all(|l| self.removed[fi].contains(*l))
            && {
                let names = self.intact(fi, lines);
                !names.is_empty()
                    && self.made_of(fi, h, lines, |_, d| names.contains(&d.name), false)
            }
    }

    /// Every non-blank changed row of the hunk lies in a definition `pick`
    /// accepts, or is a comment (or, when `decorated`, a decoration), and at
    /// least one lies in such a definition.
    fn made_of(
        &self,
        fi: usize,
        h: &RawHunk,
        lines: [&[&str]; 2],
        pick: impl Fn(usize, &Def) -> bool,
        decorated: bool,
    ) -> bool {
        let sides = self.sides(fi);
        let mut any = false;
        for (k, [a, b]) in [h.old_range, h.new_range].into_iter().enumerate() {
            let side = &sides[k];
            for r in
                (a.max(1)..=b.min(lines[k].len())).filter(|r| !lines[k][r - 1].trim().is_empty())
            {
                if side
                    .defs
                    .iter()
                    .any(|d| (d.rows[0]..=d.rows[1]).contains(&r) && pick(k, d))
                {
                    any = true;
                } else if !side.comments.contains(&r)
                    && !(decorated && side.decorations.contains(&r))
                {
                    return false;
                }
            }
        }
        any
    }

    /// The definitions of a file whose place among their siblings nothing
    /// reads, found once on each side, in the same container, with the same
    /// lines indented the same.
    fn intact(&self, fi: usize, [old, new]: [&[&str]; 2]) -> &HashSet<String> {
        self.intact[fi].get_or_init(|| {
            let sides = self.sides(fi);
            fn once<'s>(side: &'s Side, name: &str) -> Option<&'s Def> {
                let mut it = side.defs.iter().filter(|d| d.name == name);
                it.next().filter(|_| it.next().is_none())
            }
            fn parent<'s>(side: &'s Side, d: &Def) -> Option<&'s str> {
                d.parent.map(|p| side.defs[p].name.as_str())
            }
            fn body<'l>(lines: &[&'l str], d: &Def) -> Vec<&'l str> {
                lines
                    .get(d.rows[0] - 1..d.rows[1])
                    .unwrap_or_default()
                    .iter()
                    .map(|l| l.trim_end())
                    .filter(|l| !l.is_empty())
                    .collect()
            }
            let lang = lang::for_path(&self.changes[fi].path).map_or("", |s| s.name);
            sides[0]
                .defs
                .iter()
                .filter(|d| order_free(lang, d))
                .filter_map(|d| {
                    let (a, b) = (once(&sides[0], &d.name)?, once(&sides[1], &d.name)?);
                    (parent(&sides[0], a) == parent(&sides[1], b) && body(old, a) == body(new, b))
                        .then(|| d.name.clone())
                })
                .collect()
        })
    }

    /// A literal replaced by the constant that holds it; or a hunk adding
    /// nothing but such constants' bindings and comments.
    fn names_constant(
        &self,
        fi: usize,
        h: &RawHunk,
        s: &HunkSem,
        new: &[&str],
        [o, n]: [&[&str]; 2],
    ) -> bool {
        if let Some(Intent::NamesLiteral { literal, name }) = &s.intent {
            return self.constants.get(name.as_str()) == Some(&literal.as_str());
        }
        let binds = |l: &str| {
            self.constants
                .iter()
                .any(|(nm, lit)| binding(l, nm) == Some(*lit))
        };
        if !o.is_empty() || !n.iter().any(|l| binds(l)) {
            return false;
        }
        let comments = &self.sides(fi)[1].comments;
        let [a, b] = h.new_range;
        (a.max(1)..=b.min(new.len()))
            .filter(|r| !new[r - 1].trim().is_empty())
            .all(|r| comments.contains(&r) || binds(new[r - 1]))
    }

    /// The extracted definition, every line of it either taken from the old
    /// side or naming it (its header, a closing bracket); or the definition it
    /// came out of, which lost lines the extraction now holds and gained only
    /// calls to it.
    fn extracts(&self, fi: usize, s: &HunkSem, o: &[&str], n: &[&str]) -> bool {
        let named = |l: &str, nm: &str| has_word(l, nm) || !l.chars().any(char::is_alphanumeric);
        if let Some((nm, from)) = s
            .defines
            .iter()
            .find_map(|d| self.extracted.get_key_value(d.as_str()))
        {
            let left = &self.source_lines.get(&(fi, *from));
            return o.is_empty()
                && n.iter()
                    .all(|l| left.is_some_and(|ls| ls.contains(*l)) || named(l, nm));
        }
        let Some(within) = s.enclosing.as_deref() else {
            return false;
        };
        let into: Vec<&str> = self
            .extracted
            .iter()
            .filter(|(_, from)| inside(within, from))
            .map(|(nm, _)| *nm)
            .collect();
        let holds = |l: String| {
            into.iter().any(|nm| {
                self.extracted_lines
                    .get(&(fi, *nm))
                    .is_some_and(|ls| ls.contains(&l))
            })
        };
        !into.is_empty()
            && !o.is_empty()
            && o.iter().all(|l| holds(self.renamed(fi, l)))
            && n.iter().all(|l| into.iter().any(|nm| named(l, nm)))
    }
}

const DECORATIONS: &[&str] = &[
    "decorator",
    "attribute_item",
    "annotation",
    "marker_annotation",
];

/// What one side's tree says about its rows: strings spanning lines,
/// comment and decoration rows, and its definitions.
fn side_facts(spec: &lang::LangSpec, text: &str, root: tree_sitter::Node) -> Side {
    let lines: Vec<&str> = text.lines().collect();
    // a node that is all its rows hold: a trailing comment shares its row
    // with code
    let alone = |n: tree_sitter::Node| {
        let (s, e) = (n.start_position(), n.end_position());
        let line = |r: usize| lines.get(r).copied().unwrap_or("");
        s.column == indent(line(s.row)).len() && e.column >= line(e.row).trim_end().len()
    };
    let mut side = Side::default();
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let (a, b) = (n.start_position().row + 1, n.end_position().row + 1);
        let k = n.kind();
        if k.contains("comment") {
            if alone(n) {
                side.comments.extend(a..=b);
            }
            continue;
        }
        if DECORATIONS.contains(&k) {
            if alone(n) {
                side.decorations.extend(a..=b);
            }
            continue;
        }
        if a != b
            && ["string", "template", "heredoc", "block_scalar"]
                .iter()
                .any(|w| k.contains(w))
        {
            side.strings.extend(a..=b);
            continue;
        }
        stack.extend(n.children(&mut cursor));
    }
    let nodes = crate::extract::def_nodes(root, spec, text.as_bytes());
    let index: HashMap<usize, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, (_, n))| (n.id(), i))
        .collect();
    side.defs = nodes
        .iter()
        .map(|(name, n)| {
            let mut up = n.parent();
            let (mut parent, mut local) = (None, false);
            while let Some(p) = up {
                if let Some(&i) = index.get(&p.id()) {
                    parent.get_or_insert(i);
                    let k = p.kind();
                    local |= k.contains("function") || k.contains("method");
                }
                up = p.parent();
            }
            Def {
                name: name.clone(),
                kind: n.kind().to_string(),
                rows: [n.start_position().row + 1, n.end_position().row + 1],
                nested: parent.is_some(),
                parent,
                local,
            }
        })
        .collect();
    side
}

/// Every literal the change replaced by an unqualified constant, by the
/// constant's name, when each line of the change's code that binds that name
/// binds it to that literal. A qualified one (`Config.TIMEOUT`) may be bound
/// in a container the lines do not show.
fn constants<'a>(
    changes: &'a [Change],
    hunks: &'a [PerFileHunks],
    code: &[bool],
) -> HashMap<&'a str, &'a str> {
    let named: HashMap<&str, &str> = hunks
        .iter()
        .zip(code)
        .filter(|(_, c)| **c)
        .flat_map(|(f, _)| &f.sem)
        .filter_map(|s| match &s.intent {
            Some(Intent::NamesLiteral { literal, name }) if !name.contains(['.', ':']) => {
                Some((name.as_str(), literal.as_str()))
            }
            _ => None,
        })
        .collect();
    if named.is_empty() {
        return named;
    }
    // every value each name is bound to, from one pass over the lines: a
    // line can only bind a name it holds as a whole word
    let mut bound: HashMap<&str, Vec<&str>> = HashMap::new();
    for line in changes
        .iter()
        .zip(code)
        .filter(|(_, c)| **c)
        .filter_map(|(c, _)| c.new.as_deref())
        .flat_map(str::lines)
    {
        let mut words: Vec<&str> = words(line).filter(|w| named.contains_key(w)).collect();
        words.sort_unstable();
        words.dedup();
        for w in words {
            if let Some(v) = binding(line, w) {
                bound.entry(w).or_default().push(v);
            }
        }
    }
    named
        .into_iter()
        .filter(|(name, literal)| {
            bound
                .get(name)
                .is_some_and(|vs| vs.iter().all(|v| v == literal))
        })
        .collect()
}

/// The identifiers in `line`, as `binding` and `has_word` delimit them.
fn words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|w| !w.is_empty())
}

/// A definition whose place among its siblings nothing reads: items the
/// compiler resolves wherever they stand, hoisted functions, methods. Not a
/// css rule, whose order is the cascade, nor a module-level function in a
/// language that runs the module top to bottom.
fn order_free(lang: &str, d: &Def) -> bool {
    let callable = ["function", "method", "constructor"]
        .iter()
        .any(|w| d.kind.contains(w));
    !d.local
        && match lang {
            // a macro_rules! is in scope only below where it stands
            "rust" => !d.kind.contains("macro"),
            // package-level variables initialise in the order they stand
            "go" | "java" | "c" | "cpp" => callable,
            "python" | "xonsh" => d.nested && d.kind == "function_definition",
            "javascript" | "typescript" | "tsx" => {
                matches!(
                    d.kind.as_str(),
                    "function_declaration" | "method_definition"
                )
            }
            _ => false,
        }
}

/// Marked, by convention or by keyword, as out of reach of callers outside
/// its file, module or crate; a local definition is out of reach of any.
fn private(lang: &str, name: &str, d: &Def, head: &str, exported: &HashSet<&str>) -> bool {
    if d.local || name.starts_with(['_', '#']) {
        return true;
    }
    match lang {
        "rust" => !has_word(head, "pub") || head.contains("pub("),
        "go" => name.starts_with(char::is_lowercase),
        "c" | "cpp" => has_word(head, "static") && !d.nested,
        "lua" => has_word(head, "local"),
        "javascript" | "typescript" | "tsx" if !d.nested => !exported.contains(name),
        _ => has_word(head, "private"),
    }
}

/// The value a line binds `name` to, when it declares it: `const NAME = v;`,
/// `pub const NAME: u32 = v;`, `#define NAME v`, or `NAME = v` at file level.
/// Indented and with no keyword, `NAME=v,` may be a keyword argument.
fn binding<'l>(line: &'l str, name: &str) -> Option<&'l str> {
    let t = line.trim();
    let indented = line.starts_with(char::is_whitespace);
    t.match_indices(name).find_map(|(i, _)| {
        let (pre, rest) = (&t[..i], &t[i + name.len()..]);
        let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        if pre.chars().next_back().is_some_and(ident) || rest.chars().next().is_some_and(ident) {
            return None;
        }
        if pre.trim() == "#define" {
            return Some(rest.trim());
        }
        let pre = pre.trim_start().strip_prefix("pub(crate)").unwrap_or(pre);
        if !pre.chars().all(|c| ident(c) || c.is_whitespace()) || indented && pre.trim().is_empty()
        {
            return None;
        }
        let eq = rest.find('=')?;
        let (ty, value) = (&rest[..eq], &rest[eq + 1..]);
        // a type may stand between the name and `=`; an operator may not
        let typed = ty.chars().all(|c| ident(c) || " :<>[]&*.".contains(c));
        (typed && !value.starts_with('=') && !ty.ends_with(['<', '>', '*']))
            .then(|| value.trim().trim_end_matches([';', ',']).trim_end())
    })
}

/// A hunk enclosed by `within` sits in the definition `def`: the same name,
/// or one qualified by its container.
fn inside(within: &str, def: &str) -> bool {
    within == def || within.strip_suffix(def).is_some_and(|q| q.ends_with('.'))
}

/// One side's changed lines, blank ones left out and trailing space trimmed.
fn side<'a>(lines: &[&'a str], [a, b]: [usize; 2]) -> Vec<&'a str> {
    if a == 0 || a > b || b > lines.len() {
        return vec![];
    }
    lines[a - 1..b]
        .iter()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect()
}

fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        !line[..i].chars().next_back().is_some_and(ident)
            && !line[i + word.len()..].chars().next().is_some_and(ident)
    })
}

/// Both sides spell the same tokens: whitespace changed only where it
/// separates nothing — beside punctuation, or a run of it between two words —
/// and never inside a quoted string. Where indentation is syntax, every line
/// keeps its own.
fn same_tokens(o: &[&str], n: &[&str], offside: bool) -> bool {
    let lines_keep_indent =
        || o.len() == n.len() && o.iter().zip(n).all(|(a, b)| indent(a) == indent(b));
    !o.is_empty()
        && (!offside || lines_keep_indent())
        && layout_free(&o.join("\n")) == layout_free(&n.join("\n"))
}

fn indent(l: &str) -> &str {
    &l[..l.len() - l.trim_start().len()]
}

/// `text` with only the whitespace that separates tokens kept: one space
/// between two words, a newline where one may end a statement or a comment.
fn layout_free(text: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let angle = |c: char| c == '<' || c == '>';
    let mut gap = 0u8;
    let mut prev = ' ';
    // a line comment or a directive ends at its newline, and so may a
    // statement ending in a word where a newline can stand for a semicolon:
    // `return` then a value, a name then `(x)`
    let continues = |c: char| word(c) || "([+-/`".contains(c);
    let mut line_ends = false;
    for c in text.chars() {
        if let Some(q) = quote {
            out.push(c);
            // a string may span lines (a template literal, a triple quote):
            // left open, the rest stays verbatim, which only ever makes two
            // sides compare unequal
            if c == q && prev != '\\' {
                quote = None;
            }
            prev = c;
            continue;
        }
        if c.is_whitespace() {
            gap = gap.max(1 + (c == '\n') as u8);
            continue;
        }
        let last = out.chars().next_back();
        if gap > 1 && (line_ends || last.is_some_and(word) && continues(c)) {
            out.push('\n');
            line_ends = false;
        } else if gap > 0 && (angle(c) || last.is_some_and(angle)) {
            // between markup tags whitespace is text: a jsx newline drops it,
            // a space keeps it
            out.push(if gap > 1 { '\n' } else { ' ' });
        } else if gap > 0 && word(c) && last.is_some_and(word) {
            out.push(' ');
        }
        if gap > 1 {
            line_ends = false;
        }
        gap = 0;
        line_ends |= c == '#' || matches!((prev, c), ('/', '/') | ('-', '-'));
        if matches!(c, '"' | '\'' | '`') {
            quote = Some(c);
        }
        out.push(c);
        prev = c;
    }
    out
}

/// Up to this many names or paths in a sentence before the rest are counted.
const LIST_MAX: usize = 3;

fn list<S: AsRef<str>>(items: &[S]) -> String {
    let shown: Vec<&str> = items.iter().take(LIST_MAX).map(AsRef::as_ref).collect();
    match items.len().saturating_sub(LIST_MAX) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

fn place(s: &HunkSem, path: &str) -> String {
    match &s.enclosing {
        Some(nm) => crate::order::short_container(nm),
        None => path.rsplit('/').next().unwrap_or(path).to_string(),
    }
}

/// What an edit pattern did, as a noun phrase a sentence can list.
fn intent_noun(i: &Intent, nm: &str) -> String {
    match i {
        Intent::Guard { .. } => format!("a guard clause in {nm}"),
        Intent::WrapsTry { word, .. } => format!("a {word} around existing code in {nm}"),
        Intent::Condition { narrows: true, .. } => format!("a narrowed condition in {nm}"),
        Intent::Condition { .. } => format!("a widened condition in {nm}"),
        Intent::Boundary { .. } => format!("a moved comparison boundary in {nm}"),
        Intent::Inverts { .. } => format!("an inverted condition in {nm}"),
        Intent::NamesLiteral { name, .. } => format!("a literal named {name} in {nm}"),
        Intent::DefaultParam { .. } => format!("an optional parameter in {nm}"),
        Intent::Async { added: true, .. } => format!("an async function in {nm}"),
        Intent::Async { .. } => format!("a function made synchronous in {nm}"),
        Intent::SwapsCall { new, .. } => format!("a call swapped for {new} in {nm}"),
        Intent::ReplacedByCall { call, .. } => format!("inline code replaced by {call} in {nm}"),
    }
}

fn paths_of<'a>(files: &[Facts<'a>], kind: Option<PathKind>) -> Vec<&'a str> {
    files
        .iter()
        .filter(|f| kind.is_none_or(|k| f.kind == k))
        .map(|f| f.path)
        .collect()
}

fn tests_clause(files: &[Facts]) -> String {
    match paths_of(files, Some(PathKind::Test)) {
        t if t.is_empty() => ", with no test touched".to_string(),
        t => format!(", with test changes in {}", list(&t)),
    }
}

/// The first sentence: what the label rests on.
fn what(
    label: Label,
    files: &[Facts],
    added: &[&str],
    ev: &Evidence,
    lines: &[[Vec<&str>; 2]],
) -> String {
    match label {
        Label::Docs => format!(
            "Edits documentation and comments in {}.",
            list(&paths_of(files, None))
        ),
        Label::Test => format!(
            "Edits tests only: {}.",
            list(&paths_of(files, Some(PathKind::Test)))
        ),
        Label::Chore => format!(
            "Edits build, configuration or generated files only: {}.",
            list(&paths_of(files, None))
        ),
        Label::Refactor => {
            let mut parts = vec![];
            let renames: Vec<String> = ev
                .rename_pairs
                .iter()
                .map(|(o, n)| format!("{o} → {n}"))
                .collect();
            if !renames.is_empty() {
                parts.push(format!("renames {}", list(&renames)));
            }
            let (mut reordered, mut constants) = (vec![], vec![]);
            let code = files.iter().filter(|f| f.kind == PathKind::Code);
            for (f, (k, s)) in code.flat_map(|f| f.hunks.iter().map(move |h| (f, h))) {
                match (k, &s.intent) {
                    (Kind::Reorder, _) => {
                        let [old, new] = &lines[f.fi];
                        let intact = ev.intact(f.fi, [old, new]);
                        reordered.extend(
                            s.defines
                                .iter()
                                .filter(|d| intact.contains(*d))
                                .map(String::as_str),
                        )
                    }
                    (Kind::Constant, Some(Intent::NamesLiteral { literal, name })) => {
                        constants.push(format!("{literal} as {name}"))
                    }
                    _ => {}
                }
            }
            constants.sort();
            constants.dedup();
            for (verb, mut names) in [
                ("moves", ev.moved.iter().copied().collect::<Vec<&str>>()),
                ("reorders", reordered),
                ("extracts", ev.extracted.keys().copied().collect()),
            ] {
                names.sort_unstable();
                names.dedup();
                if !names.is_empty() {
                    parts.push(format!("{verb} {}", list(&names)));
                }
            }
            if !constants.is_empty() {
                parts.push(format!("names {}", list(&constants)));
            }
            let formats = code_hunks(files)
                .filter(|(k, _)| *k == Kind::Format)
                .count();
            if formats > 0 {
                parts.push(format!("reformats {formats} hunk{}", plural(formats)));
            }
            if parts.is_empty() {
                parts.push("restructures code".to_string());
            }
            format!("Refactor: {}.", parts.join("; "))
        }
        Label::FixCandidate => {
            let mut nouns: Vec<String> = files
                .iter()
                .filter(|f| f.kind == PathKind::Code)
                .flat_map(|f| f.hunks.iter().map(move |(k, s)| (f.path, *k, *s)))
                .filter_map(|(p, k, s)| Some(intent_noun(fix_pattern(k, s)?, &place(s, p))))
                .collect();
            nouns.dedup();
            let what = match nouns.is_empty() {
                true => format!("edits {}", list(&edited(files))),
                false => list(&nouns),
            };
            format!("Fix candidate: {what}{}.", tests_clause(files))
        }
        Label::Feature => format!("Adds {}{}.", list(added), tests_clause(files)),
        Label::Change => format!("Edits {}{}.", list(&edited(files)), tests_clause(files)),
    }
}

/// Where code hunks did more than a preserving edit, each place once.
fn edited(files: &[Facts]) -> Vec<String> {
    let mut places: Vec<String> = vec![];
    for f in files.iter().filter(|f| f.kind == PathKind::Code) {
        for (_, s) in f.hunks.iter().filter(|(k, _)| !k.preserves()) {
            let p = place(s, f.path);
            if !places.contains(&p) {
                places.push(p);
            }
        }
    }
    places
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The second sentence: the verdict and what it rests on.
fn why(verdict: Verdict, files: &[Facts], exposed: &[String]) -> String {
    let hunks = || {
        files
            .iter()
            .filter(|f| f.kind != PathKind::Docs)
            .flat_map(|f| f.hunks.iter().map(move |(k, s)| (f, *k, *s)))
    };
    match verdict {
        Verdict::Preserving => {
            let mut kinds: Vec<&str> = vec![];
            for (_, k, _) in hunks() {
                let word = match k {
                    Kind::Format => "formatting",
                    Kind::Comment => "a comment",
                    Kind::Rename => "a rename",
                    Kind::Move => "a move",
                    Kind::Reorder => "a reordering",
                    Kind::Constant => "a literal named as its constant",
                    _ => "an extraction",
                };
                if !kinds.contains(&word) {
                    kinds.push(word);
                }
            }
            if kinds.is_empty() {
                return "Behaviour-preserving: only documentation changed.".to_string();
            }
            match exposed {
                [] => format!(
                    "Behaviour-preserving: every hunk is {}.",
                    kinds.join(" or ")
                ),
                _ => format!(
                    "Behaviour-preserving within this change: every hunk is {}, but the API changes for callers outside it: {}.",
                    kinds.join(" or "),
                    list(exposed)
                ),
            }
        }
        Verdict::Changes => {
            let (f, k, s) = hunks()
                .find(|(f, k, _)| {
                    matches!(k, Kind::Pattern | Kind::Value)
                        && matches!(f.kind, PathKind::Code | PathKind::Chore)
                })
                .expect("a changes verdict has a pattern or value hunk");
            let nm = place(s, f.path);
            let what = match (k, &s.intent) {
                (Kind::Pattern, Some(i)) => intent_noun(i, &nm),
                _ => format!("a value changed in {nm}"),
            };
            format!("Changes behaviour: {what}.")
        }
        Verdict::Unknown => {
            let n = hunks().filter(|(_, k, _)| !k.preserves()).count();
            format!(
                "Behaviour change not ruled out: {n} hunk{} edit{} beyond a rename, move, extraction or reformat.",
                plural(n),
                if n == 1 { "s" } else { "" }
            )
        }
    }
}
