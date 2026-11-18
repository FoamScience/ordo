//! hunch's features: one vector of numbers per hunk, read off facts the engine
//! already has, for a small model trained offline on the corpus. The same
//! code path serves training (`Options.features`) and inference, so the two
//! cannot disagree on what a position means. Every column is named, and the
//! names hash to the value a trained model records: a column added, removed
//! or reordered is a different schema, and the model no longer fits. A
//! column whose name stays but whose values move is caught by the vectors of
//! a few fixed sample diffs, which hash to the fingerprint the model records.

use crate::model::{
    Category, Edge, Extent, FeatureSchema, FileOut, FindingSource, History, Hunch, HunchChange,
    HunkOut, Level, Options, Output,
};
use crate::readability::DefDelta;
use crate::{lang, readability, PerFileHunks};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Bump when a column's meaning changes without its name changing.
pub const VERSION: u32 = 1;

/// How far back a file's history counts, from the change's own commit time.
pub const HISTORY_DAYS: i64 = 90;

/// Past this many commits a file is simply busy; the count stops there so one
/// hot file cannot dominate the scale.
const HISTORY_CAP: usize = 50;

/// The rationale's first word, for the ones the engine writes often enough to
/// learn from (a corpus sweep, 2026-11); any other word is `other`.
const TEMPLATES: &[&str] = &[
    "adds",
    "calls",
    "changes",
    "comments",
    "copyright",
    "deletes",
    "edits",
    "formatting",
    "generated",
    "import",
    "moves",
    "removes",
    "renames",
    "replaces",
    "tests",
    "uses",
];

/// What one hunk's columns read.
pub(crate) struct Facts<'a> {
    pub hunk: &'a HunkOut,
    pub path: &'a str,
    pub deltas: &'a [DefDelta],
    pub edges_out: usize,
    pub edges_in: usize,
    /// (commits, fix commits) touching the file in the window; `None` with no
    /// history at all
    pub history: Option<(usize, usize)>,
}

type Value = Box<dyn Fn(&Facts) -> f64 + Send + Sync>;

struct Column {
    name: String,
    value: Value,
}

fn column(
    name: impl Into<String>,
    value: impl Fn(&Facts) -> f64 + Send + Sync + 'static,
) -> Column {
    Column {
        name: name.into(),
        value: Box::new(value),
    }
}

/// One column per word, 1 on the hunk whose `key` is that word.
fn one_hot(
    prefix: &str,
    words: impl IntoIterator<Item = String>,
    key: fn(&Facts) -> String,
) -> impl Iterator<Item = Column> {
    let prefix = prefix.to_string();
    words.into_iter().map(move |w| {
        let name = format!("{prefix}_{}", w.replace([' ', '-'], "_"));
        column(name, move |f| flag(key(f) == w))
    })
}

fn flag(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// a unit enum as the word its serde form spells
fn word(v: impl Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// rows in a `[start, end]` range; an empty side has `end < start`
fn span([a, b]: [usize; 2]) -> f64 {
    (b + 1).saturating_sub(a) as f64
}

fn findings(f: &Facts, source: FindingSource, level: Level) -> f64 {
    f.hunk
        .findings
        .iter()
        .filter(|x| x.source == source && x.level == level)
        .count() as f64
}

fn columns() -> &'static [Column] {
    static COLUMNS: OnceLock<Vec<Column>> = OnceLock::new();
    COLUMNS.get_or_init(|| {
        let mut cols = vec![
            column("noise", |f| flag(f.hunk.noise)),
            column("comment", |f| flag(f.hunk.comment)),
            column("test_path", |f| flag(lang::is_test_path(f.path))),
            column("lines_added", |f| span(f.hunk.new_range)),
            column("lines_removed", |f| span(f.hunk.old_range)),
            column("defines", |f| f.hunk.defines.len() as f64),
            column("uses", |f| f.hunk.uses.len() as f64),
            column("edges_out", |f| f.edges_out as f64),
            column("edges_in", |f| f.edges_in as f64),
            column("has_history", |f| flag(f.history.is_some())),
            column("churn", |f| f.history.map_or(0.0, |(c, _)| c as f64)),
            column("fixes", |f| f.history.map_or(0.0, |(_, x)| x as f64)),
            column("moved_defs", |f| f.deltas.len() as f64),
        ];
        for &(name, _, m) in readability::METRICS {
            cols.push(column(
                format!("delta_{}", name.replace(' ', "_")),
                move |f| {
                    f.deltas
                        .iter()
                        .map(|d| m(&d.now) as f64 - m(&d.was) as f64)
                        .sum()
                },
            ));
        }
        for source in [FindingSource::Catalog, FindingSource::Rule] {
            for level in [Level::Note, Level::Warn, Level::Verdict] {
                cols.push(column(
                    format!("{}_{}", source.as_str(), level.as_str()),
                    move |f| findings(f, source, level),
                ));
            }
        }
        cols.extend(one_hot("category", Category::ALL.iter().map(word), |f| {
            word(f.hunk.category)
        }));
        cols.extend(one_hot(
            "extent",
            Extent::ALL.iter().map(word).chain(["none".to_string()]),
            |f| f.hunk.extent.map_or("none".to_string(), word),
        ));
        cols.extend(one_hot(
            "enclosing",
            crate::model::ContainerKind::ALL
                .iter()
                .map(word)
                .chain(["none".to_string()]),
            |f| match (&f.hunk.enclosing, f.hunk.enclosing_kind) {
                (None, _) => "none".to_string(),
                (Some(_), None) => word(crate::model::ContainerKind::Definition),
                (Some(_), Some(k)) => word(k),
            },
        ));
        cols.extend(one_hot(
            "template",
            TEMPLATES
                .iter()
                .map(|t| t.to_string())
                .chain(["other".to_string()]),
            |f| {
                let first = f.hunk.rationale.split(' ').next().unwrap_or_default();
                if TEMPLATES.contains(&first) {
                    first.to_string()
                } else {
                    "other".to_string()
                }
            },
        ));
        cols.extend(one_hot(
            "lang",
            lang::all()
                .iter()
                .map(|s| s.name.to_string())
                .chain(["none".to_string()]),
            |f| {
                lang::for_path(f.path)
                    .map_or("none", |s| s.name)
                    .to_string()
            },
        ));
        cols
    })
}

/// The column names, in vector order.
pub fn names() -> Vec<String> {
    columns().iter().map(|c| c.name.clone()).collect()
}

/// FNV-1a: stable across builds and platforms, which `std`'s hasher does not
/// promise.
fn fnv1a(text: &str) -> String {
    let h = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{h:016x}")
}

/// The hash of the version and the names.
pub fn schema_hash() -> String {
    fnv1a(&format!("v{VERSION}\n{}", names().join("\n")))
}

/// The hash of the feature vectors in engine outputs, each named: per output
/// its name on a line, then a line per hunk of its values to six decimals.
/// scripts/hunch-train.py's `fingerprint` writes the same text for the
/// fixtures in tests/fixtures/hunch.
pub fn fingerprint<'a>(outputs: impl IntoIterator<Item = (&'a str, &'a Output)>) -> String {
    let mut text = String::new();
    for (name, out) in outputs {
        text.push_str(name);
        text.push('\n');
        for h in out.files.iter().flat_map(|f| &f.hunks) {
            let v = h.features.as_ref().expect("an output with features");
            let row: Vec<String> = v.iter().map(|x| format!("{x:.6}")).collect();
            text.push_str(&row.join(","));
            text.push('\n');
        }
    }
    fnv1a(&text)
}

pub fn schema() -> FeatureSchema {
    FeatureSchema {
        version: VERSION,
        hash: schema_hash(),
        names: names(),
    }
}

pub(crate) fn vector(f: &Facts) -> Vec<f64> {
    columns().iter().map(|c| (c.value)(f)).collect()
}

/// Per path, the commits that touched it in the `HISTORY_DAYS` before the
/// change, and how many of those a subject calls a fix, each capped. Which
/// commits came before is topology, settled by the caller (see `History`);
/// time only bounds the window's start, so a same-second ancestor counts.
pub(crate) fn file_history(history: &History) -> HashMap<&str, (usize, usize)> {
    static FIX: OnceLock<Regex> = OnceLock::new();
    let fix = FIX.get_or_init(|| Regex::new(r"(?i)\b(fix|bug|regress|revert)").expect("valid"));
    let since = history.at - HISTORY_DAYS * 86_400;
    let mut per: HashMap<&str, (usize, usize)> = HashMap::new();
    for c in history.commits.iter().filter(|c| c.time >= since) {
        let is_fix = fix.is_match(&c.subject);
        for p in &c.paths {
            let e = per.entry(p.as_str()).or_default();
            e.0 += 1;
            e.1 += usize::from(is_fix);
        }
    }
    for e in per.values_mut() {
        *e = (e.0.min(HISTORY_CAP), e.1.min(HISTORY_CAP));
    }
    per
}

/// Every hunk's vector, on the hunk when `options.features` asks for it, and
/// the model's guess from it when `options.hunch` does, with the guess moving
/// the hunk's `noise` flag as `decide` says. The reading order is settled
/// before this runs, so a moved flag hides or shows a hunk where it already
/// stands.
pub(crate) fn annotate(
    files: &mut [FileOut],
    hunks: &[PerFileHunks],
    edges: &[Edge],
    history: Option<&History>,
    options: &Options,
) {
    let mut out_deg: HashMap<&str, usize> = HashMap::new();
    let mut in_deg: HashMap<&str, usize> = HashMap::new();
    for e in edges {
        *out_deg.entry(e.from.as_str()).or_default() += 1;
        *in_deg.entry(e.to.as_str()).or_default() += 1;
    }
    let per_file = history.map(file_history);
    let vectors: Vec<Vec<Vec<f64>>> = files
        .iter()
        .enumerate()
        .map(|(fi, file)| {
            let hist = per_file
                .as_ref()
                .map(|h| h.get(file.path.as_str()).copied().unwrap_or_default());
            file.hunks
                .iter()
                .enumerate()
                .map(|(li, hunk)| {
                    vector(&Facts {
                        hunk,
                        path: &file.path,
                        deltas: &hunks[fi].sem[li].deltas,
                        edges_out: out_deg.get(hunk.id.as_str()).copied().unwrap_or(0),
                        edges_in: in_deg.get(hunk.id.as_str()).copied().unwrap_or(0),
                        history: hist,
                    })
                })
                .collect()
        })
        .collect();
    let guess_too = options.hunch && model_fits();
    debug_assert!(
        guess_too || !options.hunch,
        "{}",
        misfit(None).unwrap_or_default()
    );
    for (file, vs) in files.iter_mut().zip(vectors) {
        for (hunk, v) in file.hunks.iter_mut().zip(vs) {
            if guess_too {
                let g = guess(&v, hunk.noise);
                if let Some(c) = g.changed {
                    hunk.noise = c == HunchChange::Hid;
                }
                hunk.hunch = Some(g);
            }
            if options.features {
                hunk.features = Some(v);
            }
        }
    }
}

/// One node of a trained tree: a split sends `x[feature] <= threshold` left;
/// `value` is what the tree would output had it stopped here, which is what
/// path contributions are measured against.
enum Node {
    Split {
        feature: u16,
        threshold: f32,
        left: u32,
        right: u32,
        value: f32,
    },
    Leaf(f32),
}

/// The trained model, as build.rs generates it from `src/hunch.json`: a
/// LightGBM forest whose leaf values sum to the log-odds, or a logistic
/// regression over standardized features. Weights are f32, as the trainer
/// exported and checked them; sums are f64, as LightGBM's are.
#[allow(dead_code)] // build.rs builds whichever one the trainer shipped
enum Model {
    Trees {
        nodes: &'static [Node],
        roots: &'static [u32],
    },
    Logistic {
        mean: &'static [f32],
        scale: &'static [f32],
        coef: &'static [f32],
        intercept: f32,
    },
}

include!(concat!(env!("OUT_DIR"), "/hunch_model.rs"));

/// Whether the shipped model was trained on the columns this engine builds;
/// a guess from any other vector would be wrong without looking wrong. Only
/// the schema: the fingerprint needs the fixtures run through the engine,
/// which is a test's job, not every run's (see `misfit`).
pub fn model_fits() -> bool {
    misfit(None).is_none()
}

/// How the shipped model and this engine disagree, and the command that
/// retrains it; `None` when they agree. The feature schema is always
/// checked; the fixture fingerprint too when the caller computed it, as
/// tests/hunch.rs does on every CI run.
pub fn misfit(fingerprint: Option<&str>) -> Option<String> {
    let mut off = vec![];
    let schema = schema_hash();
    if SCHEMA_HASH != schema {
        off.push(format!(
            "feature schema {SCHEMA_HASH}, this engine builds {schema}"
        ));
    }
    if let Some(fp) = fingerprint.filter(|&fp| FINGERPRINT != Some(fp)) {
        off.push(format!(
            "fixture fingerprint {}, this engine computes {fp}",
            FINGERPRINT.unwrap_or("none")
        ));
    }
    (!off.is_empty()).then(|| {
        format!(
            "hunch: src/hunch.json was trained on {}; retrain it: cargo build --release && \
             ORDO_CORPUS=~/.cache/ordo-corpus scripts/hunch-train.py \
             --extra ~/.cache/ordo-corpus/judged.should_skip.jsonl",
            off.join(" and ")
        )
    })
}

/// How many contributions a guess reports.
const TOP: usize = 3;

/// P(skip) for one feature vector, and each feature's share of the log-odds:
/// a tree's split credits its feature with how far the step moved the
/// tree's value (Saabas), a regression's term is its own share. What is
/// left over is the model's bias, credited to nothing.
pub fn predict(x: &[f64]) -> (f64, Vec<f64>) {
    let mut contrib = vec![0.0; NAMES.len()];
    let raw = match &MODEL {
        Model::Trees { nodes, roots } => roots
            .iter()
            .map(|&root| {
                let mut node = &nodes[root as usize];
                while let Node::Split {
                    feature,
                    threshold,
                    left,
                    right,
                    value,
                } = node
                {
                    let f = usize::from(*feature);
                    let next = if x[f] as f32 <= *threshold {
                        left
                    } else {
                        right
                    };
                    node = &nodes[*next as usize];
                    let to = match node {
                        Node::Split { value, .. } | Node::Leaf(value) => *value,
                    };
                    contrib[f] += f64::from(to) - f64::from(*value);
                }
                match node {
                    Node::Leaf(v) => f64::from(*v),
                    Node::Split { .. } => unreachable!("the walk stops at a leaf"),
                }
            })
            .sum::<f64>(),
        Model::Logistic {
            mean,
            scale,
            coef,
            intercept,
        } => {
            for (i, c) in contrib.iter_mut().enumerate() {
                *c = f64::from(coef[i]) * (x[i] - f64::from(mean[i])) / f64::from(scale[i]);
            }
            f64::from(*intercept) + contrib.iter().sum::<f64>()
        }
    };
    (1.0 / (1.0 + (-raw).exp()), contrib)
}

fn round4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// The thresholds at which `--hunch` moves a hunk's `noise` flag, from the
/// trainer's promotion gate (scripts/hunch-train.py `promotion`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    /// show a hunk the engine hid when P(skip) is under this
    pub unhide: f64,
    /// hide a hunk the engine showed when P(skip) is at least this
    pub hide: f64,
}

/// The shipped model's policy; `None` when it failed the promotion gate,
/// and the guess then stays a shadow.
pub fn policy() -> Option<Policy> {
    POLICY
}

/// How a guess of `p` moves a hunk whose `noise` flag is `noise`, under
/// `policy`; `None` leaves the flag alone.
pub fn decide(noise: bool, p: f64, policy: Option<Policy>) -> Option<HunchChange> {
    let policy = policy?;
    if noise && p < policy.unhide {
        Some(HunchChange::Unhid)
    } else if !noise && p >= policy.hide {
        Some(HunchChange::Hid)
    } else {
        None
    }
}

/// The guess a hunk carries: P(skip), the `TOP` features that moved it most,
/// either way, in four decimals, and what the policy does to its `noise`
/// flag, decided on the unrounded P(skip).
fn guess(x: &[f64], noise: bool) -> Hunch {
    let (p, contrib) = predict(x);
    let mut by_size: Vec<usize> = (0..contrib.len()).filter(|&i| contrib[i] != 0.0).collect();
    by_size.sort_by(|&a, &b| contrib[b].abs().total_cmp(&contrib[a].abs()));
    Hunch {
        noise: round4(p),
        top: by_size
            .into_iter()
            .take(TOP)
            .map(|i| (NAMES[i].to_string(), round4(contrib[i])))
            .collect(),
        changed: decide(noise, p, POLICY),
    }
}
