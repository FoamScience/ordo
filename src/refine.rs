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
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refined {
    pub removed: Vec<Option<Vec<Span>>>,
    pub added: Vec<Option<Vec<Span>>>,
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

/// One grammar leaf, clipped to a single line. `id` is an interned form of the
/// leaf's text, unique per distinct string across both sides of a `Refiner`,
/// so the LCS inner loop compares `u32`s instead of `String`s.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    start: usize,
    end: usize,
    id: u32,
    role: Role,
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
            old_tokens: tokens_by_row(&old_tree, &old_src, old, &mut interned, None),
            new_tokens: tokens_by_row(&new_tree, &new_src, new, &mut interned, None),
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
        Some(Refiner {
            old_tokens: tokens_by_row(&old_tree, old_src, &old, &mut interned, Some(old_rows)),
            new_tokens: tokens_by_row(&new_tree, new_src, &new, &mut interned, Some(new_rows)),
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

    /// `old_range`/`new_range` are the hunk's 1-based inclusive line ranges, as
    /// the engine reports them (an empty side has `end < start`).
    pub fn refine(&self, old_range: [usize; 2], new_range: [usize; 2]) -> Refined {
        let old = rows_of(old_range, &self.old_tokens);
        let new = rows_of(new_range, &self.new_tokens);
        let mut out = Refined {
            removed: vec![None; old.len()],
            added: vec![None; new.len()],
        };
        if old.is_empty() || new.is_empty() || old.len() * new.len() > MAX_PAIRS {
            return out;
        }
        for (i, j) in pair_lines(&old, &new) {
            let (rm, add) = line_spans(&old[i], &new[j]);
            out.removed[i] = Some(rm);
            out.added[j] = Some(add);
        }
        out
    }
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
fn tokens_by_row(
    tree: &Tree,
    src: &str,
    lines: &[String],
    interned: &mut std::collections::HashMap<String, u32>,
    // only these rows (0-based, sorted), each leaf with its `Role`; `None` is
    // every row and no roles, which is all the renderer needs
    want: Option<&[usize]>,
) -> Vec<Vec<Token>> {
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
        for (row, bytes, line) in leaf_spans(node, text) {
            if row >= rows {
                break;
            }
            if !bytes.is_empty() {
                out[row].push(Token {
                    start: to_char(row, bytes.start),
                    end: to_char(row, bytes.end),
                    id: intern(interned, line),
                    role: if want.is_some() {
                        role_of(node)
                    } else {
                        Role::Other
                    },
                });
            }
        }
    }
    for row in &mut out {
        row.sort_by_key(|t| t.start);
    }
    out
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

/// The char spans of one line pair that differ: whatever the leaf LCS didn't
/// match. Adjacent spans separated by at most one column (the space between two
/// tokens) are merged, so `, label: Option<&str>` reads as one region rather
/// than eight.
fn line_spans(old: &[Token], new: &[Token]) -> (Vec<Span>, Vec<Span>) {
    let matched = lcs(old, new);
    let take = |toks: &[Token], keep: Vec<usize>| -> Vec<Span> {
        // `keep` comes from `lcs`'s backtrack, which pushes matched indices in
        // increasing order, so it's already sorted — binary search over it.
        let mut spans: Vec<Span> = vec![];
        for (k, t) in toks.iter().enumerate() {
            if keep.binary_search(&k).is_ok() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if t.start <= last.1 + 1 => last.1 = t.end,
                _ => spans.push((t.start, t.end)),
            }
        }
        spans
    };
    (
        take(old, matched.iter().map(|&(i, _)| i).collect()),
        take(new, matched.iter().map(|&(_, j)| j).collect()),
    )
}
