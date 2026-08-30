//! Real-world corpora: hard invariants, plus a quality ratchet.
//!
//! Opt-in. Set `$ORDO_CORPUS` to a directory populated by
//! `scripts/corpus-fetch.sh`; without it these tests return early, so the normal
//! run stays hermetic, offline and fast.
//!
//! Deliberately NOT snapshots. Recording thousands of rationales would be
//! unmaintainable and would fail on every wording improvement, including the
//! wanted ones. Instead:
//!
//!   * hard invariants that must never break, whatever the wording
//!   * a metrics ratchet (`corpus/baseline.json`) that fails when quality moves
//!     the wrong way — re-record with `UPDATE_CORPUS_BASELINE=1`, mirroring the
//!     `UPDATE_GOLDEN=1` convention the golden tests already use
//!
//! The engine stays git-free; this test owns its own git plumbing, the same
//! split `ordo-tui` keeps.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ordo::model::{Category, Change, Input, Options, Output};

const MAX_RATIONALE: usize = 240;
const BASELINE: &str = "corpus/baseline.json";

/// Repeating one of these inside a single rationale means per-symbol fragments
/// were joined instead of grouped — the shape of two past regressions.
const PROVENANCE: &[&str] = &[
    ", extracted from ",
    ", no uses in ",
    ", used at L",
    ", defined in ",
    ", used in ",
];

/// Extensions the engine claims. Anything else is skipped: feeding it a `.png`
/// measures nothing about rationale quality.
fn is_supported(path: &str) -> bool {
    let ext = match path.rsplit_once('.') {
        Some((_, e)) => e,
        None => return false,
    };
    matches!(
        ext,
        "py" | "pyi"
            | "js"
            | "mjs"
            | "cjs"
            | "jsx"
            | "ts"
            | "mts"
            | "cts"
            | "tsx"
            | "rs"
            | "go"
            | "java"
            | "lua"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hpp"
            | "hh"
            | "hxx"
            | "C"
            | "H"
            | "md"
            | "markdown"
    )
}

struct Repo {
    name: String,
    lang: String,
    rev: String,
    max_commits: usize,
}

fn parse_manifest() -> Vec<Repo> {
    let text = std::fs::read_to_string("corpus/manifest.toml").expect("corpus/manifest.toml");
    let field = |block: &str, key: &str| -> Option<String> {
        block.lines().find_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
            Some(rest.trim_matches('"').to_string())
        })
    };
    text.split("[[repo]]")
        .skip(1)
        .filter_map(|b| {
            Some(Repo {
                name: field(b, "name")?,
                lang: field(b, "lang")?,
                rev: field(b, "rev")?,
                max_commits: field(b, "max_commits")?.parse().ok()?,
            })
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// One commit's changed files as engine input: both sides' full blobs, so hunks
/// get full semantics rather than degrading to a context-limited patch.
fn commit_input(dir: &Path, sha: &str) -> Option<(String, Input)> {
    let parent = git(dir, &["rev-parse", "--verify", "-q", &format!("{sha}^")]);
    let parent = parent.trim();
    // A commit whose parent is missing sits on the shallow clone's boundary, not
    // at the start of history. Diffing it against the empty tree would sweep the
    // whole file as one synthetic "everything added" hunk and quietly skew every
    // metric, so skip it — `max_commits` may legitimately exceed clone depth.
    if parent.is_empty() {
        return None;
    }
    let names = git(dir, &["diff", "--name-only", parent, sha]);
    let changes: Vec<Change> = names
        .lines()
        .map(str::trim)
        .filter(|p| !p.is_empty() && is_supported(p))
        .map(|path| Change {
            path: path.to_string(),
            old: Some(git(dir, &["show", &format!("{parent}:{path}")])),
            new: Some(git(dir, &["show", &format!("{sha}:{path}")])),
            diff: None,
        })
        .collect();
    (!changes.is_empty()).then(|| {
        (
            parent.to_string(),
            Input {
                changes,
                options: Options::default(),
            },
        )
    })
}

/// Every line git considers changed must fall inside a hunk the engine emitted
/// or explicitly recorded as dropped. This is the only check here that does not
/// trust ordo's own diff: git's Myers implementation is the independent witness,
/// so a line the engine never turned into a hunk at all shows up as a gap.
///
/// Hunk *counts* are deliberately not compared — git and ordo split and merge
/// adjacent changes differently, and both are right. Coverage is the invariant.
fn check_coverage(label: &str, dir: &Path, parent: &str, sha: &str, out: &Output) {
    // `-U0` so each header's range is exactly the changed lines, nothing else
    let diff = git(dir, &["diff", "-U0", parent, sha]);
    let spans = |file: &ordo::model::FileOut, old: bool| -> Vec<[usize; 2]> {
        let kept = file
            .hunks
            .iter()
            .map(|h| if old { h.old_range } else { h.new_range });
        let gone = file
            .dropped
            .iter()
            .map(|d| if old { d.old_range } else { d.new_range });
        // an empty span (end < start) is a pure insertion/deletion on this side
        kept.chain(gone).filter(|r| r[1] >= r[0]).collect()
    };
    let covered =
        |spans: &[[usize; 2]], line: usize| spans.iter().any(|r| line >= r[0] && line <= r[1]);

    // `+++ b/<path>` / `--- a/<path>`, with /dev/null for an add or delete
    let strip = |s: &str| -> Option<String> {
        let p = s.split_once(' ').map(|(_, p)| p).unwrap_or("").trim();
        (p != "/dev/null").then(|| {
            p.split_once('/')
                .map_or(p.to_string(), |(_, r)| r.to_string())
        })
    };
    let by_path: BTreeMap<&str, &ordo::model::FileOut> =
        out.files.iter().map(|f| (f.path.as_str(), f)).collect();

    let (mut a_path, mut b_path) = (None, None);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            a_path = strip(rest);
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ ") {
            b_path = strip(rest);
            continue;
        }
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some((ranges, _)) = rest.split_once(" @@") else {
            continue;
        };
        let mut sides = ranges.split_whitespace();
        let (Some(minus), Some(plus)) = (sides.next(), sides.next()) else {
            continue;
        };
        let parse = |s: &str| -> (usize, usize) {
            let s = &s[1..];
            match s.split_once(',') {
                Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
                None => (s.parse().unwrap_or(0), 1),
            }
        };
        // the old side is only comparable when both sides name the same path —
        // a rename means the engine saw the new path against an empty old one
        let same_path = a_path.is_none() || b_path.is_none() || a_path == b_path;
        for (side_is_old, header) in [(true, minus), (false, plus)] {
            if side_is_old && !same_path {
                continue;
            }
            let Some(path) = (if side_is_old {
                a_path.as_deref()
            } else {
                b_path.as_deref()
            }) else {
                continue;
            };
            let Some(file) = by_path.get(path) else {
                continue;
            };
            let (start, count) = parse(header);
            let s = spans(file, side_is_old);
            for line_no in start..start + count {
                assert!(
                    covered(&s, line_no),
                    "{label}: {path}: {} line {line_no} is changed but lies in no hunk \
                     (kept or dropped) — the engine lost it",
                    if side_is_old { "old" } else { "new" }
                );
            }
        }
    }
}

#[derive(Default, PartialEq)]
struct Metrics {
    hunks: usize,
    bare_change: usize,
    with_details: usize,
    with_symbols: usize,
    anonymous: usize,
    longest_rationale: usize,
    degraded_files: usize,
    unsupported_files: usize,
}

impl Metrics {
    /// Directions a metric may move without failing. `Up` means more is better.
    fn fields(&self) -> Vec<(&'static str, usize, Dir)> {
        vec![
            ("hunks", self.hunks, Dir::Any),
            ("bare_change", self.bare_change, Dir::Down),
            ("with_details", self.with_details, Dir::Up),
            ("with_symbols", self.with_symbols, Dir::Up),
            ("anonymous", self.anonymous, Dir::Down),
            ("longest_rationale", self.longest_rationale, Dir::Down),
            ("degraded_files", self.degraded_files, Dir::Down),
            ("unsupported_files", self.unsupported_files, Dir::Down),
        ]
    }
}

#[derive(PartialEq)]
enum Dir {
    Up,
    Down,
    Any,
}

/// Hard invariants — these must hold whatever the wording is.
fn check_invariants(label: &str, out: &Output) {
    // `order` is a permutation of the non-import hunks (the README's claim,
    // never before checked at scale)
    let mut ordered: Vec<&str> = out.order.iter().map(|o| o.hunk.as_str()).collect();
    let mut all: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .filter(|h| h.category != Category::Import)
        .map(|h| h.id.as_str())
        .collect();
    ordered.sort_unstable();
    all.sort_unstable();
    assert_eq!(
        ordered, all,
        "{label}: `order` is not a permutation of the non-import hunks"
    );

    for f in &out.files {
        for h in &f.hunks {
            let r = &h.rationale;
            assert!(
                r.chars().count() <= MAX_RATIONALE,
                "{label}: {}:L{} rationale is {} chars, over {MAX_RATIONALE}:\n  {r}",
                f.path,
                h.new_range[0],
                r.chars().count()
            );
            assert!(
                !r.contains('\n'),
                "{label}: {}:L{} rationale spans lines:\n  {r}",
                f.path,
                h.new_range[0]
            );
            for p in PROVENANCE {
                assert!(
                    r.matches(p).count() <= 1,
                    "{label}: {}:L{} repeats {p:?} — fragments joined, not grouped:\n  {r}",
                    f.path,
                    h.new_range[0]
                );
            }
            // `<anonymous>` is always a placeholder; `_` is NOT — it is a real
            // template-parameter name in C++ (`template <typename T, typename _>`),
            // so only the former can be asserted against here.
            assert!(
                !r.contains("<anonymous>"),
                "{label}: {}:L{} leaks a placeholder name:\n  {r}",
                f.path,
                h.new_range[0]
            );
            assert!(
                !r.contains(" , ") && !r.ends_with(','),
                "{label}: {}:L{} has an empty name in a list:\n  {r}",
                f.path,
                h.new_range[0]
            );
        }
    }
}

fn sweep(dir: &Path, repo: &Repo) -> Metrics {
    let shas = git(
        dir,
        &[
            "rev-list",
            "--max-count",
            &repo.max_commits.to_string(),
            &repo.rev,
        ],
    );
    let mut m = Metrics::default();
    let mut swept = 0usize;
    for sha in shas.lines() {
        let Some((parent, input)) = commit_input(dir, sha) else {
            continue;
        };
        let out = ordo::run(input);
        let label = format!("{} {}", repo.name, &sha[..8]);
        check_invariants(&label, &out);
        check_coverage(&label, dir, &parent, sha, &out);
        swept += 1;
        for f in &out.files {
            m.degraded_files += f.degraded as usize;
            m.unsupported_files += f.unsupported as usize;
            for h in &f.hunks {
                m.hunks += 1;
                m.bare_change += (h.rationale == "change") as usize;
                m.with_details += (!h.details.is_empty()) as usize;
                m.with_symbols += (!h.symbols.is_empty()) as usize;
                m.anonymous += h.rationale.contains("<anonymous>") as usize;
                m.longest_rationale = m.longest_rationale.max(h.rationale.chars().count());
            }
        }
    }
    assert!(
        swept > 0,
        "{}: swept no commits — clone shallower than its pinned rev, or every \
         candidate sat on the shallow boundary (deepen with scripts/corpus-fetch.sh)",
        repo.name
    );
    m
}

fn corpus_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var("ORDO_CORPUS").ok()?);
    d.is_dir().then_some(d)
}

fn read_baseline() -> BTreeMap<String, BTreeMap<String, usize>> {
    std::fs::read_to_string(BASELINE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[test]
fn corpora_hold_their_invariants_and_do_not_regress() {
    let Some(root) = corpus_dir() else {
        eprintln!(
            "corpus: $ORDO_CORPUS unset or missing — skipping.\n\
             populate it with scripts/corpus-fetch.sh"
        );
        return;
    };
    let update = std::env::var("UPDATE_CORPUS_BASELINE").is_ok();
    let baseline = read_baseline();
    let mut recorded: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut failures: Vec<String> = vec![];
    let mut seen = 0usize;

    for repo in parse_manifest() {
        let dir = root.join(&repo.name);
        if !dir.is_dir() {
            eprintln!("corpus: {} not fetched — skipping", repo.name);
            continue;
        }
        seen += 1;
        let m = sweep(&dir, &repo);
        let mut row = BTreeMap::new();
        for (k, v, _) in m.fields() {
            row.insert(k.to_string(), v);
        }
        eprintln!(
            "corpus: {:10} [{:>10}] hunks={} bare_change={} details={} symbols={} longest={}",
            repo.name,
            repo.lang,
            m.hunks,
            m.bare_change,
            m.with_details,
            m.with_symbols,
            m.longest_rationale
        );

        if let Some(before) = baseline.get(&repo.name) {
            for (k, now, dir_) in m.fields() {
                let Some(&was) = before.get(k) else { continue };
                let worse = match dir_ {
                    Dir::Down => now > was,
                    Dir::Up => now < was,
                    Dir::Any => false,
                };
                if worse {
                    failures.push(format!(
                        "{}: {k} moved the wrong way: {was} -> {now}",
                        repo.name
                    ));
                }
            }
        }
        recorded.insert(repo.name.clone(), row);
    }

    if seen == 0 {
        eprintln!("corpus: nothing fetched — skipping");
        return;
    }
    if update {
        std::fs::write(
            BASELINE,
            format!("{}\n", serde_json::to_string_pretty(&recorded).unwrap()),
        )
        .expect("write baseline");
        eprintln!("corpus: baseline re-recorded ({BASELINE})");
        return;
    }
    assert!(
        failures.is_empty(),
        "corpus quality regressed:\n  {}\n\nIf a change is a deliberate improvement, \
         re-record with UPDATE_CORPUS_BASELINE=1",
        failures.join("\n  ")
    );
}
