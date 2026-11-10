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
//! split `ordo` keeps.
use std::collections::BTreeMap;
use std::path::Path;

use ordo_diff::model::{Category, Output};

mod common;
use common::{commit_input, corpus_dir, git, parse_manifest, range_input, update_requested, Repo};

const MAX_RATIONALE: usize = 240;
const BASELINE: &str = "corpus/baseline.json";

/// Repeating one of these inside a single rationale means per-symbol fragments
/// were joined instead of grouped — the shape of two past regressions.
const PROVENANCE: &[&str] = &[
    ", extracted from ",
    ", no uses in ",
    ", defined in ",
    ", used in ",
];

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
    let spans = |file: &ordo_diff::model::FileOut, old: bool| -> Vec<[usize; 2]> {
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
    let by_path: BTreeMap<&str, &ordo_diff::model::FileOut> =
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
    /// commits whose subject says fix/revert — a proxy for "this change
    /// corrects a defect", the only defect ground truth git offers for free
    fix_commits: usize,
    /// ...of which at least one non-noise hunk carries a finding. Compare with
    /// `other_with_finding` over `hunks`: a catalog that fires no more often on
    /// fixes than elsewhere is not pointing at defects.
    fix_with_finding: usize,
    other_commits: usize,
    other_with_finding: usize,
    /// ...of which the first non-noise hunk in `order` is production code, not
    /// a test: the fix is read before the test that pins it
    fix_code_first: usize,
    /// consecutive parent/child commit pairs touching disjoint files, squashed
    /// into one input — each is a PR that should split into two clusters
    pairs: usize,
    /// ...where some cluster mixes hunks from both. Every such merge is a
    /// cross-file edge between two independent commits; an upper bound on
    /// false edges, since the child may legitimately use what the parent added.
    merged: usize,
    /// pairs that share a file, where every squashed hunk still belongs to
    /// exactly one of the two commits — the only witness of a wrong join inside
    /// one file
    shared_pairs: usize,
    /// ...where some cluster mixes hunks from both
    shared_merged: usize,
    /// pairs sharing a file that were left out because some squashed hunk
    /// belonged to both commits or neither
    shared_skipped: usize,
    /// clusters summed over every commit — the "independent parts" a reviewer
    /// is offered as a split
    clusters: usize,
    /// ...of which a single hunk
    singletons: usize,
    /// groups reading one edit repeated across files as one item
    same_edits: usize,
    /// ...and the hunks they hold
    same_edit_hunks: usize,
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
            ("fix_commits", self.fix_commits, Dir::Any),
            ("fix_with_finding", self.fix_with_finding, Dir::Any),
            ("other_commits", self.other_commits, Dir::Any),
            ("other_with_finding", self.other_with_finding, Dir::Any),
            ("fix_code_first", self.fix_code_first, Dir::Up),
            ("pairs", self.pairs, Dir::Any),
            ("merged", self.merged, Dir::Down),
            ("shared_pairs", self.shared_pairs, Dir::Any),
            ("shared_merged", self.shared_merged, Dir::Down),
            ("shared_skipped", self.shared_skipped, Dir::Any),
            ("clusters", self.clusters, Dir::Any),
            ("singletons", self.singletons, Dir::Any),
            ("same_edits", self.same_edits, Dir::Any),
            ("same_edit_hunks", self.same_edit_hunks, Dir::Up),
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
    // `order` is a permutation of every hunk (the README's claim, never before
    // checked at scale). Import hunks are part of it: they are noise — visible
    // but skippable — rather than dropped, since a dropped import made a new
    // dependency invisible and a moved one read as a bare deletion.
    let mut ordered: Vec<&str> = out.order.iter().map(|o| o.hunk.as_str()).collect();
    let mut all: Vec<&str> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
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
            assert!(
                h.category != Category::Import || h.noise,
                "{label}: {}:L{} is an import hunk but not noise",
                f.path,
                h.new_range[0]
            );
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

fn is_fix_subject(subject: &str) -> bool {
    subject.split(|c: char| !c.is_alphanumeric()).any(|w| {
        matches!(
            w.to_ascii_lowercase().as_str(),
            "fix" | "fixes" | "fixed" | "revert" | "reverts"
        )
    })
}

/// One-commit signals git can vouch for: does a finding land on this change,
/// and is production code the first thing a reader is shown. A commit with no
/// code hunk at all (docs, comments, pure noise) can satisfy neither and is
/// left out of both buckets.
fn fix_signals(out: &Output, m: &mut Metrics, fix: bool) {
    let by_id: BTreeMap<&str, &ordo_diff::model::HunkOut> = out
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .map(|h| (h.id.as_str(), h))
        .collect();
    let Some(first) = out.order.iter().find(|o| {
        by_id
            .get(o.hunk.as_str())
            .is_some_and(|h| !h.noise && !h.comment)
    }) else {
        return;
    };
    let has_finding = by_id.values().any(|h| !h.noise && !h.findings.is_empty());
    if fix {
        m.fix_commits += 1;
        m.fix_with_finding += has_finding as usize;
        m.fix_code_first += !ordo_diff::is_test_path(&first.path) as usize;
    } else {
        m.other_commits += 1;
        m.other_with_finding += has_finding as usize;
    }
}

/// Each file a commit touched, with the new-side range of every hunk the engine
/// made of it.
type NewRanges = Vec<(String, Vec<[usize; 2]>)>;

/// Whether two inclusive `[start, end]` ranges share a line. An empty range
/// (`end < start`) — the point a pure insertion or deletion sits at — still
/// occupies its start line.
fn overlaps([a0, a1]: [usize; 2], [b0, b1]: [usize; 2]) -> bool {
    a0 <= b1.max(b0) && b0 <= a1.max(a0)
}

/// Squash `child` onto its parent `sha` and ask whether ordo keeps them apart.
/// On disjoint file sets a hunk belongs to whichever commit touched its file.
/// A shared file can blend both commits' lines into one hunk, where no cluster
/// boundary can fall, so there each squashed hunk is attributed by its ranges —
/// its old side against the parent's hunks (both on the grandparent), its new
/// side against the child's (both on the child) — and a pair with any hunk
/// claimed by both or neither is skipped. The file sets are the engine's own —
/// what it turned into hunks for each commit alone — so a file it skipped
/// cannot make a pair look shared.
fn pair_signals(
    repo: &str,
    dir: &Path,
    (child, child_new): (&str, &NewRanges),
    sha: &str,
    sha_out: &Output,
    parent: &str,
    m: &mut Metrics,
) {
    let child_ranges = |path: &str| child_new.iter().find(|(p, _)| p == path).map(|(_, r)| r);
    let shared = sha_out
        .files
        .iter()
        .any(|f| child_ranges(&f.path).is_some());
    let Some(input) = range_input(dir, parent, child) else {
        return;
    };
    let out = ordo_diff::run(input);
    let mut from_child: BTreeMap<&str, bool> = BTreeMap::new();
    for f in &out.files {
        let mine = child_ranges(&f.path);
        let theirs = sha_out.files.iter().find(|pf| pf.path == f.path);
        for h in &f.hunks {
            let c = mine.is_some_and(|r| r.iter().any(|&r| overlaps(r, h.new_range)));
            let p = theirs.is_some_and(|pf| {
                pf.hunks
                    .iter()
                    .any(|ph| overlaps(ph.old_range, h.old_range))
            });
            let owner = if shared {
                match (c, p) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => {
                        m.shared_skipped += 1;
                        return;
                    }
                }
            } else {
                mine.is_some()
            };
            from_child.insert(h.id.as_str(), owner);
        }
    }
    let mixed = out.clusters.iter().any(|c| {
        let mut owners = c.iter().filter_map(|id| from_child.get(id.as_str()));
        owners.clone().any(|o| *o) && owners.any(|o| !*o)
    });
    if shared {
        m.shared_pairs += 1;
        m.shared_merged += mixed as usize;
    } else {
        m.pairs += 1;
        m.merged += mixed as usize;
    }
    if mixed {
        // named so a rise in `merged` can be chased to the pair that caused it
        let kind = if shared { "shared-file" } else { "merged" };
        eprintln!("{repo}: {kind} pair {}..{}", &sha[..8], &child[..8]);
    }
}

fn sweep(dir: &Path, repo: &Repo) -> Metrics {
    // `--format=%s` interleaves `commit <sha>` / subject lines
    let listing = git(
        dir,
        &[
            "rev-list",
            "--max-count",
            &repo.max_commits.to_string(),
            "--format=%s",
            &repo.rev,
        ],
    );
    let mut lines = listing.lines();
    let mut shas: Vec<(&str, &str)> = vec![];
    while let Some(l) = lines.next() {
        if let Some(sha) = l.strip_prefix("commit ") {
            shas.push((sha, lines.next().unwrap_or("")));
        }
    }
    let mut m = Metrics::default();
    let mut swept = 0usize;
    // the previously swept commit — its sha, its parent, the hunks the engine
    // made — kept for one iteration in case this commit turns out to be that parent
    let mut child: Option<(&str, String, NewRanges)> = None;
    for (sha, subject) in shas {
        let Some((parent, input)) = commit_input(dir, sha) else {
            child = None;
            continue;
        };
        let out = ordo_diff::run(input);
        let label = format!("{} {}", repo.name, &sha[..8]);
        check_invariants(&label, &out);
        check_coverage(&label, dir, &parent, sha, &out);
        swept += 1;
        fix_signals(&out, &mut m, is_fix_subject(subject));
        if let Some((c, _, c_files)) = child.as_ref().filter(|(_, c_parent, _)| c_parent == sha) {
            pair_signals(&repo.name, dir, (c, c_files), sha, &out, &parent, &mut m);
        }
        m.clusters += out.clusters.len();
        m.singletons += out.clusters.iter().filter(|c| c.len() == 1).count();
        for g in out.groups.iter().filter(|g| !g.files.is_empty()) {
            m.same_edits += 1;
            m.same_edit_hunks += g.members.len();
        }
        child = Some((
            sha,
            parent,
            out.files
                .iter()
                .map(|f| {
                    (
                        f.path.clone(),
                        f.hunks.iter().map(|h| h.new_range).collect(),
                    )
                })
                .collect(),
        ));
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

fn read_baseline() -> BTreeMap<String, BTreeMap<String, usize>> {
    std::fs::read_to_string(BASELINE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[test]
fn corpora_hold_their_invariants_and_do_not_regress() {
    let Some(root) = corpus_dir() else {
        // Skipping is right for a normal `cargo test` — the sweep needs real
        // repositories cloned first. It is wrong for a run that meant to
        // exercise the ratchet, where a silent pass is indistinguishable from
        // a green one, so that caller says so and gets a failure instead.
        assert!(
            !update_requested("ORDO_CORPUS_REQUIRED"),
            "ORDO_CORPUS_REQUIRED is set but $ORDO_CORPUS is unset or missing — \
             the quality ratchet did not run; populate it with scripts/corpus-fetch.sh"
        );
        eprintln!(
            "corpus: $ORDO_CORPUS unset or missing — skipping.\n\
             populate it with scripts/corpus-fetch.sh\n\
             set ORDO_CORPUS_REQUIRED=1 to make this a failure instead"
        );
        return;
    };
    let update = update_requested("UPDATE_CORPUS_BASELINE");
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
        let pct = |a: usize, b: usize| (a * 100).checked_div(b).unwrap_or(0);
        eprintln!(
            "corpus: {:10} fix commits={} finding on fix={}% on other={}% code first={}% \
             pairs={} merged={} shared pairs={} merged={} skipped={} clusters={} singletons={} \
             same edits={} ({} hunks)",
            "",
            m.fix_commits,
            pct(m.fix_with_finding, m.fix_commits),
            pct(m.other_with_finding, m.other_commits),
            pct(m.fix_code_first, m.fix_commits),
            m.pairs,
            m.merged,
            m.shared_pairs,
            m.shared_merged,
            m.shared_skipped,
            m.clusters,
            m.singletons,
            m.same_edits,
            m.same_edit_hunks
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
