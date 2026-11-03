//! Intra-line refinement: given a hunk's removed and added lines, which *parts*
//! of a changed line actually changed.
//!
//! A line-level diff says "this line went away, that one arrived". When the two
//! are the same statement with one argument added, saying so is more useful than
//! showing both lines whole — the reviewer's eye should land on the argument.
//!
//! The unit of comparison is the tree-sitter **leaf node**, never a character or
//! a whitespace-split word: the grammar decides what a token is, so `Option<&str>`
//! is `Option` `<` `&` `str` `>` and never splits mid-identifier. Matching is a
//! longest-common-subsequence over those leaves; what the LCS leaves unmatched is
//! what changed.
//!
//! Deliberately not a tree *alignment* (difftastic's Dijkstra-over-graphs, or
//! syndiff): those buy accuracy on moved and restructured code, at a cost this
//! does not need to pay to say "an argument was added". A pair of lines whose
//! leaves have little in common is simply left unrefined and renders whole, so a
//! genuine rewrite is never dressed up as a small edit.
use crate::lang;
use tree_sitter::{Node, Parser, Tree};

/// A half-open char-column span within one line.
pub type Span = (usize, usize);

/// What changed in one hunk. Each vector is parallel to the hunk's lines on that
/// side: `Some(spans)` means the line was paired with its counterpart and only
/// `spans` differ; `None` means it stands alone and changed as a whole.
///
/// `moved_removed`/`moved_added` lie over those: the parts of the changed text
/// that reappear, unmatched, on the other side of the hunk — an argument two
/// places over, a statement a few lines down. Empty when nothing moved.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refined {
    pub removed: Vec<Option<Vec<Span>>>,
    pub added: Vec<Option<Vec<Span>>>,
    pub moved_removed: Vec<Vec<Span>>,
    pub moved_added: Vec<Vec<Span>>,
}

/// Below this leaf-overlap ratio two lines are not the same line edited, they
/// are a removal and an unrelated addition — refining them would invent a
/// relationship the code doesn't have.
const PAIR_THRESHOLD: f32 = 0.4;

/// Refinement is O(removed × added) in line pairing. A hunk far past this is a
/// rewrite, where per-token highlighting would be noise anyway, so it renders
/// whole rather than costing a frame.
const MAX_PAIRS: usize = 4096;

/// Cells the leaf LCS table may hold for one line pair. The table is
/// `(n+1)×(m+1)` `u32`s, and nothing bounds the leaves on a single line: one
/// changed line of a minified bundle carries six figures of them, and the
/// allocation that follows is measured in gigabytes. Past this the pair renders
/// whole, which is what a machine-generated line deserves anyway.
const MAX_LCS_CELLS: usize = 1 << 22;

/// Unmatched leaves a side may hold for move detection to run: each round
/// compares every unmatched run against every one on the other side, so past
/// this the hunk is a rewrite and nothing in it is reported moved.
const MAX_MOVE_TOKENS: usize = 256;

/// One grammar leaf, clipped to a single line. `id` is an interned form of the
/// leaf's text, unique per distinct string across both sides of a `Refiner`,
/// so the LCS inner loop compares `u32`s instead of `String`s.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    start: usize,
    end: usize,
    id: u32,
    role: Role,
    /// part of a comment, a string or prose, where the leaf is a run of words
    /// rather than one name
    texty: bool,
}

/// What a leaf is part of, as far as a one-line headline can say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// inside a string literal
    Str,
    /// a number literal
    Num,
    /// the function a call calls
    Callee,
    /// inside a call's argument list
    Arg,
    /// a name
    Ident,
    Other,
}

/// The one thing a hunk changed, when that is all it changed: a single run of
/// leaves on a single line, all of one role. `old`/`new` are the run's text on
/// each side (empty for a pure insertion or removal).
#[derive(Debug, PartialEq, Eq)]
pub struct SingleChange {
    pub role: Role,
    pub old: String,
    pub new: String,
}

/// A file parsed once, refined per hunk — parsing per hunk would re-walk the
/// whole file for every hunk in it.
pub struct Refiner {
    old_tokens: Vec<Vec<Token>>,
    new_tokens: Vec<Vec<Token>>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

impl Refiner {
    /// `None` when the path has no grammar or either side fails to parse: the
    /// caller then renders whole lines, which is the honest fallback.
    pub fn new(path: &str, old: &[String], new: &[String]) -> Option<Refiner> {
        let spec = lang::for_path(path)?;
        let lang = (spec.language)();
        let leaves = Leaves {
            want: None,
            words: true,
            prose: spec.prose,
        };
        let parse = |lines: &[String]| -> Option<(Tree, String)> {
            let src = lines.join("\n");
            let mut p = Parser::new();
            p.set_language(&lang).ok()?;
            Some((p.parse(&src, None)?, src))
        };
        let (old_tree, old_src) = parse(old)?;
        let (new_tree, new_src) = parse(new)?;
        // one interner shared by both sides: a token's id must mean the same
        // text whichever side it came from, or cross-side comparisons in
        // `lcs` are comparing unrelated numbers.
        let mut interned: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        Some(Refiner {
            old_tokens: tokens_by_row(&old_tree, &old_src, old, &mut interned, &leaves),
            new_tokens: tokens_by_row(&new_tree, &new_src, new, &mut interned, &leaves),
            old_lines: old.to_vec(),
            new_lines: new.to_vec(),
        })
    }

    /// The engine's refiner: both sides parsed through `lang::parse`, whose
    /// cache already holds them, and only `old_rows`/`new_rows` (0-based,
    /// sorted) tokenized — a headline needs a few lines, not the whole file.
    pub(crate) fn for_rows(
        spec: &lang::LangSpec,
        (old_src, new_src): (&str, &str),
        (old_rows, new_rows): (&[usize], &[usize]),
    ) -> Option<Refiner> {
        let old_tree = lang::parse(spec, old_src)?;
        let new_tree = lang::parse(spec, new_src)?;
        let owned = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        let (old, new) = (owned(old_src), owned(new_src));
        let mut interned: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        // whole leaves: a headline names the string that changed, not a word in it
        let leaves = |want| Leaves {
            want: Some(want),
            words: false,
            prose: spec.prose,
        };
        Some(Refiner {
            old_tokens: tokens_by_row(&old_tree, old_src, &old, &mut interned, &leaves(old_rows)),
            new_tokens: tokens_by_row(&new_tree, new_src, &new, &mut interned, &leaves(new_rows)),
            old_lines: old,
            new_lines: new,
        })
    }

    /// The one run of leaves this hunk changed, if that is all it did: every
    /// line of each side pairs with one on the other, a single pair differs,
    /// and it differs in one contiguous run of leaves sharing a role.
    pub fn single_change(
        &self,
        old_range: [usize; 2],
        new_range: [usize; 2],
    ) -> Option<SingleChange> {
        let (old, new) = (
            rows_of(old_range, &self.old_tokens),
            rows_of(new_range, &self.new_tokens),
        );
        if old.is_empty() || old.len() != new.len() || old.len() * new.len() > MAX_PAIRS {
            return None;
        }
        let pairs = pair_lines(&old, &new);
        if pairs.len() != old.len() {
            return None;
        }
        let mut changed = pairs
            .iter()
            .filter(|&&(i, j)| old[i].iter().map(|t| t.id).ne(new[j].iter().map(|t| t.id)));
        let &(i, j) = changed.next()?;
        if changed.next().is_some() {
            return None;
        }
        let matched = lcs(&old[i], &new[j]);
        let rest = |toks: &[Token], kept: Vec<usize>| -> Option<Vec<Token>> {
            // `kept` comes out of `lcs` in increasing order
            let gone: Vec<usize> = (0..toks.len())
                .filter(|k| kept.binary_search(k).is_err())
                .collect();
            // one contiguous run, or nothing on this side
            let contiguous = gone.windows(2).all(|w| w[1] == w[0] + 1);
            contiguous.then(|| gone.iter().map(|&k| toks[k].clone()).collect())
        };
        let o = rest(&old[i], matched.iter().map(|&(a, _)| a).collect())?;
        let n = rest(&new[j], matched.iter().map(|&(_, b)| b).collect())?;
        let role =
            o.iter()
                .chain(&n)
                .map(|t| t.role)
                .reduce(|a, b| if a == b { a } else { Role::Other })?;
        let text = |toks: &[Token], line: &str| -> String {
            match (toks.first(), toks.last()) {
                (Some(f), Some(l)) => line.chars().skip(f.start).take(l.end - f.start).collect(),
                _ => String::new(),
            }
        };
        Some(SingleChange {
            role,
            old: text(&o, &self.old_lines[old_range[0] - 1 + i]),
            new: text(&n, &self.new_lines[new_range[0] - 1 + j]),
        })
    }

    /// How much of a hunk's old leaves survive into its new side, 0.0–1.0, both
    /// sides read as one stream each with line breaks set aside: a call
    /// re-wrapped over three lines keeps all of them. A comment or a string
    /// counts by its words, so a year bumped in a copyright line keeps the rest
    /// of the line.
    pub(crate) fn kept_share(&self, old_range: [usize; 2], new_range: [usize; 2]) -> f32 {
        #[derive(Hash, PartialEq, Eq)]
        enum Key {
            Leaf(u32),
            Word(String),
        }
        let keys = |r: [usize; 2], all: &[Vec<Token>], lines: &[String]| -> Vec<Key> {
            let mut out = vec![];
            for (k, row) in rows_of(r, all).iter().enumerate() {
                let line: Vec<char> = lines
                    .get(r[0] - 1 + k)
                    .map_or(vec![], |l| l.chars().collect());
                for t in row {
                    if !t.texty {
                        out.push(Key::Leaf(t.id));
                        continue;
                    }
                    let text: String = line
                        .get(t.start..t.end.min(line.len()))
                        .unwrap_or(&[])
                        .iter()
                        .collect();
                    out.extend(words_of(&text).map(|(a, b)| Key::Word(text[a..b].to_string())));
                }
            }
            out
        };
        kept_of(
            &keys(old_range, &self.old_tokens, &self.old_lines),
            &keys(new_range, &self.new_tokens, &self.new_lines),
        )
    }

    /// `old_range`/`new_range` are the hunk's 1-based inclusive line ranges, as
    /// the engine reports them (an empty side has `end < start`).
    pub fn refine(&self, old_range: [usize; 2], new_range: [usize; 2]) -> Refined {
        let old = rows_of(old_range, &self.old_tokens);
        let new = rows_of(new_range, &self.new_tokens);
        let mut out = Refined {
            removed: vec![None; old.len()],
            added: vec![None; new.len()],
            ..Refined::default()
        };
        if old.is_empty() || new.is_empty() || old.len() * new.len() > MAX_PAIRS {
            return out;
        }
        let mut old_free: Vec<Vec<bool>> = old.iter().map(|r| vec![true; r.len()]).collect();
        let mut new_free: Vec<Vec<bool>> = new.iter().map(|r| vec![true; r.len()]).collect();
        let mut anchors: Vec<Anchor> = vec![];
        for (i, j) in pair_lines(&old, &new) {
            for (a, b) in lcs(&old[i], &new[j]) {
                old_free[i][a] = false;
                new_free[j][b] = false;
                anchors.push(((i, a), (j, b)));
            }
            out.removed[i] = Some(spans_of(&old[i], &old_free[i]));
            out.added[j] = Some(spans_of(&new[j], &new_free[j]));
        }
        let words = |rows: &[Vec<Token>], lines: &[String], first: usize| -> Vec<Vec<bool>> {
            rows.iter()
                .enumerate()
                .map(|(k, r)| {
                    let line: Vec<char> =
                        lines.get(first + k).map_or(vec![], |l| l.chars().collect());
                    r.iter()
                        .map(|t| {
                            line.get(t.start..t.end.min(line.len()))
                                .is_some_and(|cs| cs.iter().any(|c| c.is_alphanumeric()))
                        })
                        .collect()
                })
                .collect()
        };
        let old_side = Side {
            rows: &old,
            free: &old_free,
            word: &words(&old, &self.old_lines, old_range[0] - 1),
        };
        let new_side = Side {
            rows: &new,
            free: &new_free,
            word: &words(&new, &self.new_lines, new_range[0] - 1),
        };
        (out.moved_removed, out.moved_added) = moves(&old_side, &new_side, &anchors);
        out
    }
}

/// One side of a hunk as move detection reads it: its token rows, which
/// tokens the line pairing left unmatched, and which carry a letter or digit.
struct Side<'a> {
    rows: &'a [Vec<Token>],
    free: &'a [Vec<bool>],
    word: &'a [Vec<bool>],
}

/// The share of `old` a diff finds again in `new`; all of it when `old` is empty.
pub(crate) fn kept_of<T: std::hash::Hash + Eq>(old: &[T], new: &[T]) -> f32 {
    if old.is_empty() {
        return 1.0;
    }
    let kept: usize = similar::capture_diff_slices(similar::Algorithm::Myers, old, new)
        .iter()
        .filter(|op| op.tag() == similar::DiffTag::Equal)
        .map(|op| op.old_range().len())
        .sum();
    kept as f32 / old.len() as f32
}

/// A run of consecutive unmatched tokens on one row: `(row, start, end)`.
type Run = (usize, usize, usize);

/// A token the line pairing matched, as `(row, index)` on each side.
type Anchor = ((usize, usize), (usize, usize));

impl Side<'_> {
    fn runs(&self) -> Vec<Run> {
        let mut out = vec![];
        for (row, free) in self.free.iter().enumerate() {
            let mut k = 0;
            while k < free.len() {
                if !free[k] {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < free.len() && free[k] {
                    k += 1;
                }
                out.push((row, start, k));
            }
        }
        out
    }

    /// The run without the punctuation at its ends: `, b` is the run `b`.
    fn core(&self, (row, a, b): Run) -> Option<(usize, usize)> {
        let w = &self.word[row];
        let s = (a..b).find(|&k| w[k])?;
        let e = (a..b).rev().find(|&k| w[k])? + 1;
        Some((s, e))
    }
}

/// Text left unmatched on both sides that is the same text: what moved rather
/// than changed. Greedy, longest first, over runs of unmatched tokens. A match
/// counts when it holds two words, or is the only word of its run on both
/// sides — a lone `self` shared by two rewritten lines is a coincidence, a lone
/// argument that left one place and arrived at another is a move.
///
/// Text that kept its place relative to everything the pairing matched — a
/// call split over three lines, the rest of a line wrapped onto the next — is
/// re-wrapped, not moved, and stays as the pairing left it.
fn moves(old: &Side, new: &Side, anchors: &[Anchor]) -> (Vec<Vec<Span>>, Vec<Vec<Span>>) {
    let mut old_moved: Vec<Vec<bool>> = old.rows.iter().map(|r| vec![false; r.len()]).collect();
    let mut new_moved: Vec<Vec<bool>> = new.rows.iter().map(|r| vec![false; r.len()]).collect();
    let free = |s: &Side| s.free.iter().flatten().filter(|&&f| f).count();
    if free(old) <= MAX_MOVE_TOKENS && free(new) <= MAX_MOVE_TOKENS {
        let (mut old_runs, mut new_runs) = (old.runs(), new.runs());
        while let Some((oi, ni, oa, na, len)) = best_move(old, new, &old_runs, &new_runs, anchors) {
            let (orow, os, oe) = old_runs[oi];
            let (nrow, ns, ne) = new_runs[ni];
            old_moved[orow][oa..oa + len].fill(true);
            new_moved[nrow][na..na + len].fill(true);
            let split = |runs: &mut Vec<Run>, at: usize, (row, s, e): Run, a: usize| {
                runs.remove(at);
                runs.extend(
                    [(row, s, a), (row, a + len, e)]
                        .into_iter()
                        .filter(|r| r.1 < r.2),
                );
            };
            split(&mut old_runs, oi, (orow, os, oe), oa);
            split(&mut new_runs, ni, (nrow, ns, ne), na);
        }
    }
    let spans = |s: &Side, moved: &[Vec<bool>]| -> Vec<Vec<Span>> {
        s.rows
            .iter()
            .zip(moved)
            .map(|(toks, m)| spans_of(toks, m))
            .collect()
    };
    (spans(old, &old_moved), spans(new, &new_moved))
}

/// The longest acceptable common token run between any old and any new run:
/// `(old run, new run, old start, new start, length)`.
fn best_move(
    old: &Side,
    new: &Side,
    old_runs: &[Run],
    new_runs: &[Run],
    anchors: &[Anchor],
) -> Option<(usize, usize, usize, usize, usize)> {
    let crosses = |at: Anchor| anchors.iter().any(|&(o, n)| (o < at.0) != (n < at.1));
    let mut best: Option<(usize, usize, usize, usize, usize)> = None;
    for (oi, &(orow, os, oe)) in old_runs.iter().enumerate() {
        let a = &old.rows[orow][os..oe];
        for (ni, &(nrow, ns, ne)) in new_runs.iter().enumerate() {
            let b = &new.rows[nrow][ns..ne];
            // longest common substring ending at each (i, j), one row at a time
            let mut prev = vec![0usize; b.len() + 1];
            for (i, at) in a.iter().enumerate() {
                let mut cur = vec![0usize; b.len() + 1];
                for (j, bt) in b.iter().enumerate() {
                    if at.id != bt.id {
                        continue;
                    }
                    cur[j + 1] = prev[j] + 1;
                    let len = cur[j + 1];
                    if best.is_some_and(|m| m.4 >= len) {
                        continue;
                    }
                    let (oa, na) = (os + i + 1 - len, ns + j + 1 - len);
                    let words = old.word[orow][oa..oa + len].iter().filter(|&&w| w).count();
                    let covers = |s: &Side, run: Run, at: usize| {
                        s.core(run)
                            .is_some_and(|(cs, ce)| at <= cs && ce <= at + len)
                    };
                    let enough = words >= 2
                        || words == 1
                            && covers(old, (orow, os, oe), oa)
                            && covers(new, (nrow, ns, ne), na);
                    if enough && crosses(((orow, oa), (nrow, na))) {
                        best = Some((oi, ni, oa, na, len));
                    }
                }
                prev = cur;
            }
        }
    }
    best
}

/// The token rows of a 1-based inclusive line range; empty when the side is.
fn rows_of(r: [usize; 2], all: &[Vec<Token>]) -> Vec<Vec<Token>> {
    if r[0] < 1 || r[1] < r[0] {
        return vec![];
    }
    (r[0]..=r[1].min(all.len()))
        .map(|ln| all.get(ln - 1).cloned().unwrap_or_default())
        .collect()
}

/// What a leaf is part of: the nearest string or number literal, call callee
/// or argument list above it, else a bare name or anything else.
fn role_of(leaf: Node) -> Role {
    let mut at = leaf;
    for depth in 0..6 {
        let kind = at.kind();
        if kind.contains("string")
            || kind == "template_substitution"
            || kind.contains("char_literal")
        {
            return Role::Str;
        }
        if kind.contains("number")
            || kind.contains("integer")
            || kind.contains("float")
            || kind.ends_with("int_literal")
        {
            return Role::Num;
        }
        let Some(parent) = at.parent() else { break };
        let is_callee = parent
            .child_by_field_name("function")
            .or_else(|| {
                matches!(parent.kind(), "method_invocation" | "function_call")
                    .then(|| parent.child_by_field_name("name"))
                    .flatten()
            })
            .is_some_and(|f| f.id() == at.id());
        // the called name itself: the whole function position, or the name at
        // the end of `obj.method` / `ns::f` — not the receiver in front of it
        let names_it = at.id() == leaf.id()
            || ["property", "field", "attribute", "name"].iter().any(|f| {
                at.child_by_field_name(f)
                    .is_some_and(|n| n.id() == leaf.id())
            });
        if is_callee && (parent.kind().contains("call") || parent.kind() == "method_invocation") {
            return if names_it { Role::Callee } else { Role::Ident };
        }
        // an argument the call itself holds — `f(a, b)`, not an element deep
        // inside one (`useEffect(fn, [a, b])` adds to an array, not a call)
        let whole_arg = depth == 0 || matches!(at.kind(), "keyword_argument" | "named_argument");
        if whole_arg && matches!(parent.kind(), "arguments" | "argument_list") {
            return Role::Arg;
        }
        at = parent;
    }
    if lang::is_ident(leaf.kind()) {
        Role::Ident
    } else {
        Role::Other
    }
}

/// Every leaf of the tree, clipped to the row it sits on — a multi-row leaf (a
/// block comment, a raw string) contributes one token per row it covers, so a
/// row's tokens always describe that row alone.
/// Which leaves `tokens_by_row` reads, and how.
struct Leaves<'a> {
    /// only these rows (0-based, sorted), each leaf with its `Role`; `None` is
    /// every row and no roles, which is all the renderer needs
    want: Option<&'a [usize]>,
    /// a comment, string or prose leaf as one token per word, so an edited
    /// word marks the word rather than the whole comment
    words: bool,
    /// every leaf of the file is prose
    prose: bool,
}

fn tokens_by_row(
    tree: &Tree,
    src: &str,
    lines: &[String],
    interned: &mut std::collections::HashMap<String, u32>,
    leaves: &Leaves,
) -> Vec<Vec<Token>> {
    let want = leaves.want;
    let rows = lines.len();
    let wanted = |a: usize, b: usize| {
        want.is_none_or(|w| {
            w.get(w.partition_point(|&r| r < a))
                .is_some_and(|&r| r <= b)
        })
    };
    let mut out: Vec<Vec<Token>> = vec![vec![]; rows];
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];
    // tree-sitter columns are byte offsets; the renderer works in chars
    let to_char = |row: usize, byte: usize| -> usize {
        lines
            .get(row)
            .map_or(byte, |l| l[..byte.min(l.len())].chars().count())
    };
    while let Some(node) = stack.pop() {
        if !wanted(node.start_position().row, node.end_position().row) {
            continue;
        }
        if node.child_count() > 0 && !leaves_text_uncovered(node, src) {
            stack.extend(node.children(&mut cursor));
            continue;
        }
        let text = src.get(node.start_byte()..node.end_byte()).unwrap_or("");
        let texty = leaves.prose || is_texty(node);
        let role = if want.is_some() {
            role_of(node)
        } else {
            Role::Other
        };
        for (row, bytes, line) in leaf_spans(node, text) {
            if row >= rows {
                break;
            }
            if bytes.is_empty() {
                continue;
            }
            let mut push = |a: usize, b: usize, piece: &str| {
                out[row].push(Token {
                    start: to_char(row, a),
                    end: to_char(row, b),
                    id: intern(interned, piece),
                    role,
                    texty,
                })
            };
            if texty && leaves.words {
                for (a, b) in words_of(line) {
                    push(bytes.start + a, bytes.start + b, &line[a..b]);
                }
            } else {
                push(bytes.start, bytes.end, line);
            }
        }
    }
    for row in &mut out {
        row.sort_by_key(|t| t.start);
    }
    out
}

/// A leaf whose text is words rather than one token: a comment, a string
/// literal or its content, or what a c directive holds unparsed (`#error`'s
/// message).
fn is_texty(leaf: Node) -> bool {
    let k = leaf.kind();
    k.contains("comment") || k.contains("string") || k.contains("heredoc") || k == "preproc_arg"
}

/// The byte ranges of a text's words and punctuation: a run of letters, digits
/// and `_` is one, any other non-blank char is one on its own.
fn words_of(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut it = text.char_indices().peekable();
    std::iter::from_fn(move || {
        let (start, c) = it.find(|&(_, c)| !c.is_whitespace())?;
        let mut end = start + c.len_utf8();
        if word(c) {
            while let Some(&(i, n)) = it.peek() {
                if !word(n) {
                    break;
                }
                end = i + n.len_utf8();
                it.next();
            }
        }
        Some((start, end))
    })
}

/// Does the node hold text none of its children do? css's `1px` is an
/// `integer_value` whose one child is the `px`: split into its children, the
/// `1` would be no token at all, and `0` → `1px` would read as `0` → `px`.
fn leaves_text_uncovered(node: Node, src: &str) -> bool {
    let mut cursor = node.walk();
    let mut at = node.start_byte();
    let gap = |a: usize, b: usize| src.get(a..b).is_some_and(|t| !t.trim().is_empty());
    for ch in node.children(&mut cursor) {
        if gap(at, ch.start_byte()) {
            return true;
        }
        at = ch.end_byte();
    }
    gap(at, node.end_byte())
}

/// A leaf's text row by row: (row, the byte columns it covers there, the
/// line's text). The columns are tree-sitter's own on the first and last row
/// and the whole line in between.
fn leaf_spans<'s>(
    node: Node,
    text: &'s str,
) -> impl Iterator<Item = (usize, std::ops::Range<usize>, &'s str)> {
    let (s, e) = (node.start_position(), node.end_position());
    let (sr, sc, er, ec) = (s.row, s.column, e.row, e.column);
    text.split('\n').enumerate().map(move |(k, line)| {
        let row = sr + k;
        let bytes = if sr == er {
            sc..ec
        } else if row == sr {
            sc..sc + line.len()
        } else if row == er {
            0..ec
        } else {
            0..line.len()
        };
        (row, bytes, line)
    })
}

fn intern(interned: &mut std::collections::HashMap<String, u32>, text: &str) -> u32 {
    if let Some(&id) = interned.get(text) {
        return id;
    }
    let id = interned.len() as u32;
    interned.insert(text.to_string(), id);
    id
}

/// Fills the LCS DP table (flat, row-major, `(n+1) x (m+1)`) and returns it
/// alongside `n`/`m`, for callers that need to backtrack it.
fn lcs_table(a: &[Token], b: &[Token]) -> Option<(Vec<u32>, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    if (n + 1).saturating_mul(m + 1) > MAX_LCS_CELLS {
        return None;
    }
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if a[i].id == b[j].id {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    Some((dp, n, m))
}

/// Longest common subsequence of two token runs, as index pairs.
fn lcs(a: &[Token], b: &[Token]) -> Vec<(usize, usize)> {
    let Some((dp, n, m)) = lcs_table(a, b) else {
        return vec![];
    };
    let w = m + 1;
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n && j < m {
        if a[i].id == b[j].id {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Length of the LCS of two token runs, without backtracking a match out of
/// the table — the fill is reverse (`i`/`j` count down to 0), so the full
/// subsequence length ends up at `dp[0][0]`.
fn lcs_len(a: &[Token], b: &[Token]) -> Option<usize> {
    let (dp, _, _) = lcs_table(a, b)?;
    Some(dp[0] as usize)
}

/// How much two lines have in common, 0.0–1.0, by matched leaves. A pair too
/// large to build a table for scores 0 and so never pairs.
fn similarity(a: &[Token], b: &[Token]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    match lcs_len(a, b) {
        Some(common) => 2.0 * common as f32 / (a.len() + b.len()) as f32,
        None => 0.0,
    }
}

/// Which removed line is which added line, order-preserving: a monotone
/// alignment maximizing total similarity, keeping only pairs that clear
/// `PAIR_THRESHOLD`. Lines left unpaired changed as a whole.
fn pair_lines(old: &[Vec<Token>], new: &[Vec<Token>]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    let sim: Vec<Vec<f32>> = (0..n)
        .map(|i| (0..m).map(|j| similarity(&old[i], &new[j])).collect())
        .collect();
    let mut dp = vec![vec![0f32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let pair = if sim[i][j] >= PAIR_THRESHOLD {
                sim[i][j] + dp[i + 1][j + 1]
            } else {
                0.0
            };
            dp[i][j] = pair.max(dp[i + 1][j]).max(dp[i][j + 1]);
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < n && j < m {
        // Re-take the branch the fill took, by recomputing its `max` rather
        // than testing the accumulated sums for equality: `dp` holds sums of
        // many similarities, whose representable gap is far wider than
        // `f32::EPSILON`, so an absolute-epsilon comparison silently dropped
        // pairs on a long hunk. Ties prefer the pair, exactly as the fill does.
        let pair = sim[i][j] + dp[i + 1][j + 1];
        let paired = sim[i][j] >= PAIR_THRESHOLD && pair >= dp[i + 1][j] && pair >= dp[i][j + 1];
        if paired {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// The char spans of the tokens `pick` selects. Adjacent spans separated by at
/// most one column (the space between two tokens) are merged, so `, label:
/// Option<&str>` reads as one region rather than eight.
fn spans_of(toks: &[Token], pick: &[bool]) -> Vec<Span> {
    let mut spans: Vec<Span> = vec![];
    for (t, _) in toks.iter().zip(pick).filter(|(_, &p)| p) {
        match spans.last_mut() {
            Some(last) if t.start <= last.1 + 1 => last.1 = t.end,
            _ => spans.push((t.start, t.end)),
        }
    }
    spans
}
