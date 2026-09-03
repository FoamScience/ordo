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

/// Refinement is O(removed × added) in line pairing and O(n × m) in leaves per
/// pair. A hunk far past this is a rewrite, where per-token highlighting would
/// be noise anyway, so it renders whole rather than costing a frame.
const MAX_PAIRS: usize = 4096;

/// One grammar leaf, clipped to a single line. `id` is an interned form of the
/// leaf's text, unique per distinct string across both sides of a `Refiner`,
/// so the LCS inner loop compares `u32`s instead of `String`s.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    start: usize,
    end: usize,
    id: u32,
}

/// A file parsed once, refined per hunk — parsing per hunk would re-walk the
/// whole file for every hunk in it.
pub struct Refiner {
    old_tokens: Vec<Vec<Token>>,
    new_tokens: Vec<Vec<Token>>,
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
            old_tokens: tokens_by_row(&old_tree, &old_src, old, &mut interned),
            new_tokens: tokens_by_row(&new_tree, &new_src, new, &mut interned),
        })
    }

    /// `old_range`/`new_range` are the hunk's 1-based inclusive line ranges, as
    /// the engine reports them (an empty side has `end < start`).
    pub fn refine(&self, old_range: [usize; 2], new_range: [usize; 2]) -> Refined {
        let rows = |r: [usize; 2], all: &Vec<Vec<Token>>| -> Vec<Vec<Token>> {
            if r[0] < 1 || r[1] < r[0] {
                return vec![];
            }
            (r[0]..=r[1].min(all.len()))
                .map(|ln| all.get(ln - 1).cloned().unwrap_or_default())
                .collect()
        };
        let old = rows(old_range, &self.old_tokens);
        let new = rows(new_range, &self.new_tokens);
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

/// Every leaf of the tree, clipped to the row it sits on — a multi-row leaf (a
/// block comment, a raw string) contributes one token per row it covers, so a
/// row's tokens always describe that row alone.
fn tokens_by_row(
    tree: &Tree,
    src: &str,
    lines: &[String],
    interned: &mut std::collections::HashMap<String, u32>,
) -> Vec<Vec<Token>> {
    let rows = lines.len();
    let mut out: Vec<Vec<Token>> = vec![vec![]; rows];
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];
    // tree-sitter columns are byte offsets; the renderer works in chars
    let to_char = |row: usize, byte: usize| -> usize {
        lines
            .get(row)
            .map_or(byte, |l| l[..byte.min(l.len())].chars().count())
    };
    let node_rows = |node: Node| -> (usize, usize, usize, usize) {
        let (s, e) = (node.start_position(), node.end_position());
        (s.row, s.column, e.row, e.column)
    };
    while let Some(node) = stack.pop() {
        if node.child_count() > 0 {
            stack.extend(node.children(&mut cursor));
            continue;
        }
        let (sr, sc, er, ec) = node_rows(node);
        let text = src.get(node.start_byte()..node.end_byte()).unwrap_or("");
        for (k, line) in text.split('\n').enumerate() {
            let row = sr + k;
            if row >= rows {
                break;
            }
            // columns are byte offsets from tree-sitter; the renderer counts
            // chars, so convert against the row's own text
            let (bs, be) = if sr == er {
                (sc, ec)
            } else if row == sr {
                (sc, sc + line.len())
            } else if row == er {
                (0, ec)
            } else {
                (0, line.len())
            };
            if be > bs {
                let id = match interned.get(line) {
                    Some(&id) => id,
                    None => {
                        let id = interned.len() as u32;
                        interned.insert(line.to_string(), id);
                        id
                    }
                };
                out[row].push(Token {
                    start: to_char(row, bs),
                    end: to_char(row, be),
                    id,
                });
            }
        }
    }
    for row in &mut out {
        row.sort_by_key(|t| t.start);
    }
    out
}

/// Fills the LCS DP table (flat, row-major, `(n+1) x (m+1)`) and returns it
/// alongside `n`/`m`, for callers that need to backtrack it.
fn lcs_table(a: &[Token], b: &[Token]) -> (Vec<u32>, usize, usize) {
    let (n, m) = (a.len(), b.len());
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
    (dp, n, m)
}

/// Longest common subsequence of two token runs, as index pairs.
fn lcs(a: &[Token], b: &[Token]) -> Vec<(usize, usize)> {
    let (dp, n, m) = lcs_table(a, b);
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
fn lcs_len(a: &[Token], b: &[Token]) -> usize {
    let (dp, _, _) = lcs_table(a, b);
    dp[0] as usize
}

/// How much two lines have in common, 0.0–1.0, by matched leaves.
fn similarity(a: &[Token], b: &[Token]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let common = lcs_len(a, b) as f32;
    2.0 * common / (a.len() + b.len()) as f32
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
        let paired = sim[i][j] >= PAIR_THRESHOLD
            && (sim[i][j] + dp[i + 1][j + 1] - dp[i][j]).abs() < f32::EPSILON;
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
