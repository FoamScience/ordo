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
    Extract,
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
            Kind::Format | Kind::Comment | Kind::Rename | Kind::Move | Kind::Extract
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
    /// old name → new name, every rename in the change
    renames: HashMap<&'a str, &'a str>,
    moved: HashSet<&'a str>,
    /// per file, the old-side rows a moved definition left from
    moved_rows: Vec<Vec<usize>>,
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
    /// per file, the 1-based old and new rows inside a string that spans
    /// lines: whitespace there is text. Parsed only for a file one of whose
    /// hunks looks like a reformat.
    strings: Vec<OnceCell<[HashSet<usize>; 2]>>,
}

struct Facts<'a> {
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
    let mut ev = Evidence {
        renames: code_changes()
            .flat_map(|c| c.rename.iter().map(|(n, o)| (o.as_str(), n.as_str())))
            .collect(),
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
        moved_rows: changed
            .iter()
            .zip(&code)
            .map(|(c, code)| {
                c.removals
                    .iter()
                    .filter(|r| *code && matches!(r.kind, RemovalKind::MovedTo(_)))
                    .map(|r| r.row)
                    .collect()
            })
            .collect(),
        extracted: code_changes()
            .flat_map(|c| c.relocated.iter().map(|(n, f)| (n.as_str(), f.as_str())))
            .collect(),
        removed: vec![],
        added: vec![],
        extracted_lines: HashMap::new(),
        source_lines: HashMap::new(),
        changes,
        strings: changes.iter().map(|_| OnceCell::new()).collect(),
    };
    for (fi, f) in hunks.iter().enumerate() {
        let [old, new] = &lines[fi];
        let (mut rm, mut ad) = (HashSet::new(), HashSet::new());
        for (h, s) in f.raw.iter().zip(&f.sem).filter(|_| code[fi]) {
            let (o, n): (Vec<String>, Vec<String>) = (
                side(old, h.old_range)
                    .into_iter()
                    .map(|l| ev.renamed(l))
                    .collect(),
                side(new, h.new_range)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            );
            for (nm, from) in &ev.extracted {
                if s.defines.iter().any(|d| d == nm) {
                    ev.extracted_lines
                        .entry((fi, nm))
                        .or_default()
                        .extend(n.iter().cloned());
                }
                if s.enclosing.as_deref().is_some_and(|e| inside(e, from)) {
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
    let sentences = vec![what(label, &files, &added, &ev), why(verdict, &files)];
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
            && !self.renames.is_empty()
            && o.len() == n.len()
            && o.iter().zip(&n).all(|(a, b)| self.renamed(a) == *b)
        {
            return Kind::Rename;
        }
        if code && s.category == Category::Import {
            return self.import_kind(s);
        }
        if code && self.moves(fi, s, &o, &n) {
            return Kind::Move;
        }
        if code && self.extracts(fi, s, &o, &n) {
            return Kind::Extract;
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

    /// Either side of the hunk lies in a string that spans lines.
    fn in_string(&self, fi: usize, h: &RawHunk) -> bool {
        let [old, new] = self.strings[fi].get_or_init(|| {
            let c = &self.changes[fi];
            let rows = |text: Option<&str>| {
                lang::for_path(&c.path)
                    .zip(text)
                    .and_then(|(spec, t)| lang::parse(spec, t))
                    .map(|t| string_rows(t.root_node()))
                    .unwrap_or_default()
            };
            [rows(c.old.as_deref()), rows(c.new.as_deref())]
        });
        let within = |rows: &HashSet<usize>, [a, b]: [usize; 2]| (a..=b).any(|r| rows.contains(&r));
        within(old, h.old_range) || within(new, h.new_range)
    }

    /// An import that follows a rename, move or extraction elsewhere in the
    /// change brings in nothing new.
    fn import_kind(&self, s: &HunkSem) -> Kind {
        let known = |n: &String| {
            let n = n.as_str();
            self.moved.contains(n)
                || self.extracted.contains_key(n)
                || self.renames.contains_key(n)
                || self.renames.values().any(|v| *v == n)
        };
        if !s.imports.is_empty() && s.imports.iter().all(known) {
            Kind::Move
        } else {
            Kind::Edit
        }
    }

    /// The line with every renamed identifier spelled its new way.
    fn renamed(&self, line: &str) -> String {
        if self.renames.is_empty() {
            return line.to_string();
        }
        let mut out = String::with_capacity(line.len());
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            out.push_str(self.renames.get(word.as_str()).copied().unwrap_or(word));
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

    /// A hunk carrying a moved definition, whose every line left (or arrived
    /// in) another file of the change.
    fn moves(&self, fi: usize, s: &HunkSem, o: &[&str], n: &[&str]) -> bool {
        let [a, b] = s.old_range;
        let touches = s.defines.iter().any(|d| self.moved.contains(d.as_str()))
            || self.moved_rows[fi].iter().any(|r| (a..=b).contains(r));
        let elsewhere = |sets: &[HashSet<String>], l: String| {
            sets.iter()
                .enumerate()
                .any(|(fj, set)| fj != fi && set.contains(&l))
        };
        touches
            && n.iter().all(|l| elsewhere(&self.removed, l.to_string()))
            && o.iter().all(|l| elsewhere(&self.added, self.renamed(l)))
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
            && o.iter().all(|l| holds(self.renamed(l)))
            && n.iter().all(|l| into.iter().any(|nm| named(l, nm)))
    }
}

/// 1-based rows of every string literal, template or heredoc that spans more
/// than one line.
fn string_rows(node: tree_sitter::Node) -> HashSet<usize> {
    let mut rows = HashSet::new();
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        let (a, b) = (n.start_position().row, n.end_position().row);
        if a == b {
            continue;
        }
        let k = n.kind();
        if ["string", "template", "heredoc", "block_scalar"]
            .iter()
            .any(|w| k.contains(w))
        {
            rows.extend(a + 1..=b + 1);
            continue;
        }
        stack.extend(n.children(&mut cursor));
    }
    rows
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
fn what(label: Label, files: &[Facts], added: &[&str], ev: &Evidence) -> String {
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
            let mut renames: Vec<String> = ev
                .renames
                .iter()
                .map(|(o, n)| format!("{o} → {n}"))
                .collect();
            renames.sort();
            if !renames.is_empty() {
                parts.push(format!("renames {}", list(&renames)));
            }
            for (verb, mut names) in [
                ("moves", ev.moved.iter().copied().collect::<Vec<&str>>()),
                ("extracts", ev.extracted.keys().copied().collect()),
            ] {
                names.sort_unstable();
                if !names.is_empty() {
                    parts.push(format!("{verb} {}", list(&names)));
                }
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
            let nouns: Vec<String> = files
                .iter()
                .filter(|f| f.kind == PathKind::Code)
                .flat_map(|f| f.hunks.iter().map(move |(k, s)| (f.path, *k, *s)))
                .filter_map(|(p, k, s)| Some(intent_noun(fix_pattern(k, s)?, &place(s, p))))
                .collect();
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
fn why(verdict: Verdict, files: &[Facts]) -> String {
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
                    _ => "an extraction",
                };
                if !kinds.contains(&word) {
                    kinds.push(word);
                }
            }
            if kinds.is_empty() {
                return "Behaviour-preserving: only documentation changed.".to_string();
            }
            format!(
                "Behaviour-preserving: every hunk is {}.",
                kinds.join(" or ")
            )
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
