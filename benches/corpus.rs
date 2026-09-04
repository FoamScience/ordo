//! CPU-time bench for `ordo::run` over the real corpora.
//!
//! Opt-in exactly like `tests/corpus.rs`: set `$ORDO_CORPUS` to a directory
//! populated by `scripts/corpus-fetch.sh`, or this returns early.
//!
//!   cargo bench --bench corpus
//!   ORDO_BENCH_REPOS=all cargo bench --bench corpus
//!   UPDATE_BENCH_BASELINE=1 cargo bench --bench corpus
//!
//! Deliberately not criterion. Each measured unit is tens to hundreds of
//! milliseconds of deterministic CPU work, so sampling machinery would buy
//! nothing a median of a few reps does not, and would cost a dependency plus
//! minutes per run.
//!
//! Three scenarios per repo, because a single "average commit" number hides the
//! shape that matters. Per-hunk work inside a file is what scales badly, so
//! `wide` (many files, few hunks each) and `deep` (≤3 files carrying the whole
//! change) are measured apart — a fix that helps one can leave the other flat.
//!
//! Timed with this process's own CPU time (user + system), not wall clock:
//! wall time swings tens of percent when other processes compete for cores
//! (this repo's own concurrent builds are enough to do it), which swamps the
//! 5-20% wins this harness exists to detect. CPU time is immune to that.
//!
//! The baseline lands in `target/` and is never committed: the `ms` figure is
//! CPU-milliseconds, meaningful only against another run on the same machine.
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ordo::model::{Change, Input, Options};

#[path = "../tests/common/mod.rs"]
mod common;
use common::{commit_input, corpus_dir, git, is_supported, parse_manifest};

const BASELINE: &str = "target/ordo-bench.json";
const DEFAULT_REPOS: &str = "click,ripgrep,vue";

/// A commit's blobs, held so reps can rebuild `Input` (not `Clone`, and `run`
/// consumes it) without paying git I/O inside the timed region.
type Blobs = Vec<(String, String, String)>;

struct Job {
    label: String,
    commits: Vec<Blobs>,
    files: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Row {
    ms: f64,
    hunks: usize,
    files: usize,
    commits: usize,
}

/// `(sha, supported files, changed lines)` per commit, newest first — one git
/// call for the whole window instead of one per commit.
fn commit_stats(dir: &Path, rev: &str, max: usize) -> Vec<(String, usize, usize)> {
    let log = git(
        dir,
        &[
            "log",
            "--no-merges",
            "--format=%H",
            "--numstat",
            "--max-count",
            &max.to_string(),
            rev,
        ],
    );
    let mut out: Vec<(String, usize, usize)> = vec![];
    for line in log.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((added, rest)) = line.split_once('\t') else {
            out.push((line.to_string(), 0, 0));
            continue;
        };
        let Some((deleted, path)) = rest.split_once('\t') else {
            continue;
        };
        if !is_supported(path) {
            continue;
        }
        // "-" in either column is a binary file; is_supported already excludes
        // the extensions those carry, so parse failures just count as zero.
        if let Some(c) = out.last_mut() {
            c.1 += 1;
            c.2 += added.parse().unwrap_or(0) + deleted.parse::<usize>().unwrap_or(0);
        }
    }
    out.retain(|c| c.1 > 0);
    out
}

fn build(dir: &Path, shas: &[String]) -> Vec<Blobs> {
    shas.iter()
        .filter_map(|sha| {
            let (_, input) = commit_input(dir, sha)?;
            Some(
                input
                    .changes
                    .into_iter()
                    .map(|c| (c.path, c.old.unwrap_or_default(), c.new.unwrap_or_default()))
                    .collect(),
            )
        })
        .filter(|b: &Blobs| !b.is_empty())
        .collect()
}

fn to_input(blobs: &Blobs) -> Input {
    Input {
        changes: blobs
            .iter()
            .map(|(path, old, new)| Change {
                path: path.clone(),
                old: Some(old.clone()),
                new: Some(new.clone()),
                diff: None,
            })
            .collect(),
        options: Options::default(),
    }
}

/// This process's total CPU time (user + system) consumed so far.
///
/// Wall clock is at the mercy of whatever else is running on the machine;
/// CPU time is not, which is the whole reason this bench uses it. On Linux,
/// read it straight from `/proc/self/stat` rather than pull in a dependency
/// for two integers. Anywhere else, fall back to wall-clock `Instant` — this
/// is a bench, not something worth failing the build over.
#[cfg(target_os = "linux")]
fn cpu_time() -> Duration {
    // Linux's clock tick rate (USER_HZ / sysconf(_SC_CLK_TCK)) is 100 on
    // essentially every real configuration; hardcoding it avoids a libc
    // dependency just to call sysconf.
    const TICKS_PER_SEC: f64 = 100.0;

    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Field 2 is `comm`, parenthesised and possibly containing spaces (or even
    // ')'), so find the LAST ')' rather than splitting the whole line on
    // whitespace. Everything after it is space-separated starting at field 3.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // fields[0] is field 3, so field N is at index N - 3: utime is field 14
    // (index 11), stime is field 15 (index 12).
    let utime: u64 = fields.get(11).and_then(|s| s.parse().ok()).unwrap_or(0);
    let stime: u64 = fields.get(12).and_then(|s| s.parse().ok()).unwrap_or(0);
    Duration::from_secs_f64((utime + stime) as f64 / TICKS_PER_SEC)
}

#[cfg(not(target_os = "linux"))]
fn cpu_time() -> Duration {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

/// Median CPU time, plus the hunk count the run produced. Inputs are rebuilt
/// outside the clock so only `ordo::run` is measured.
///
/// `reps` is a floor, not a count. `/proc/self/stat` only has 10ms
/// resolution (100 ticks/sec), so a scenario that finishes in tens of
/// milliseconds needs many reps or the quantisation itself is a large error
/// — a fast job keeps sampling until its samples cover `MIN_CPU`, which at
/// 2s keeps that error well under 1%.
fn measure(job: &Job, reps: usize) -> (Duration, usize) {
    const MIN_CPU: Duration = Duration::from_secs(2);
    const MAX_REPS: usize = 25;

    let mut times: Vec<Duration> = Vec::with_capacity(reps);
    let mut hunks = 0;
    while times.len() < reps || (times.len() < MAX_REPS && times.iter().sum::<Duration>() < MIN_CPU)
    {
        let inputs: Vec<Input> = job.commits.iter().map(to_input).collect();
        let start = cpu_time();
        let outs: Vec<_> = inputs.into_iter().map(ordo::run).collect();
        times.push(cpu_time() - start);
        hunks = outs
            .iter()
            .flat_map(|o| &o.files)
            .map(|f| f.hunks.len())
            .sum();
    }
    times.sort();
    (times[times.len() / 2], hunks)
}

fn main() {
    let Some(root) = corpus_dir() else {
        eprintln!(
            "bench: $ORDO_CORPUS unset or missing — skipping.\n\
             populate it with scripts/corpus-fetch.sh"
        );
        return;
    };
    let want = std::env::var("ORDO_BENCH_REPOS").unwrap_or_else(|_| DEFAULT_REPOS.to_string());
    let reps: usize = std::env::var("ORDO_BENCH_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let baseline: BTreeMap<String, Row> = std::fs::read_to_string(BASELINE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut jobs: Vec<Job> = vec![];
    for repo in parse_manifest() {
        if want != "all" && !want.split(',').any(|w| w.trim() == repo.name) {
            continue;
        }
        let dir = root.join(&repo.name);
        if !dir.is_dir() {
            eprintln!("bench: {} not fetched — skipping", repo.name);
            continue;
        }
        let stats = commit_stats(&dir, &repo.rev, repo.max_commits);
        if stats.is_empty() {
            eprintln!("bench: {} swept no commits — skipping", repo.name);
            continue;
        }

        let pick = |shas: Vec<String>, kind: &str| -> Option<Job> {
            let commits = build(&dir, &shas);
            let files = commits.iter().map(Vec::len).sum();
            (!commits.is_empty()).then_some(Job {
                label: format!("{}/{kind}", repo.name),
                commits,
                files,
            })
        };

        // history order — what an average review actually looks like
        let typical: Vec<String> = stats.iter().take(8).map(|c| c.0.clone()).collect();

        // most files touched: breadth, the shape the engine already handles well
        let mut by_files = stats.clone();
        by_files.sort_by_key(|c| std::cmp::Reverse(c.1));
        let wide: Vec<String> = by_files.iter().take(4).map(|c| c.0.clone()).collect();

        // whole change concentrated in ≤3 files: the per-hunk-inside-a-file
        // paths a breadth-only workload never exercises
        let mut by_density: Vec<_> = stats.iter().filter(|c| c.1 <= 3).cloned().collect();
        by_density.sort_by_key(|c| std::cmp::Reverse(c.2 / c.1));
        let deep: Vec<String> = by_density.iter().take(4).map(|c| c.0.clone()).collect();

        jobs.extend(pick(typical, "typical"));
        jobs.extend(pick(wide, "wide"));
        jobs.extend(pick(deep, "deep"));
    }

    if jobs.is_empty() {
        eprintln!("bench: nothing to run — check $ORDO_CORPUS and ORDO_BENCH_REPOS");
        return;
    }

    println!(
        "\n{:<20} {:>7} {:>6} {:>7} {:>9} {:>9} {:>9} {:>10}",
        "scenario", "commits", "files", "hunks", "ms", "ms/file", "ms/hunk", "vs base"
    );
    println!("{}", "-".repeat(83));

    let mut recorded: BTreeMap<String, Row> = BTreeMap::new();
    for job in &jobs {
        let (elapsed, hunks) = measure(job, reps);
        let ms = elapsed.as_secs_f64() * 1000.0;
        // The pair that says where the time actually goes: a fix aimed at
        // per-file work moves ms/file, one aimed at per-hunk work moves ms/hunk.
        let per_file = ms / job.files.max(1) as f64;
        let per_hunk = if hunks > 0 { ms / hunks as f64 } else { 0.0 };

        // A hunk-count change means the two runs did different work, so the
        // delta is not a like-for-like timing comparison — say so rather than
        // report a percentage that reads as a speedup.
        let delta = match baseline.get(&job.label) {
            Some(b) if b.hunks != hunks => format!("hunks {}→{}", b.hunks, hunks),
            Some(b) if b.ms > 0.0 => format!("{:+.1}%", (ms - b.ms) / b.ms * 100.0),
            _ => "—".to_string(),
        };

        println!(
            "{:<20} {:>7} {:>6} {:>7} {:>9.1} {:>9.1} {:>9.1} {:>10}",
            job.label,
            job.commits.len(),
            job.files,
            hunks,
            ms,
            per_file,
            per_hunk,
            delta
        );
        recorded.insert(
            job.label.clone(),
            Row {
                ms,
                hunks,
                files: job.files,
                commits: job.commits.len(),
            },
        );
    }

    // The whole point of splitting the scenarios: `deep` must actually be
    // narrower than `wide`, or both are measuring breadth and the per-hunk
    // paths stay unmeasured. Guaranteed by construction — assert it so a
    // future edit to the selection cannot silently undo it.
    for (label, row) in &recorded {
        let Some(repo) = label.strip_suffix("/deep") else {
            continue;
        };
        let Some(wide) = recorded.get(&format!("{repo}/wide")) else {
            continue;
        };
        let (deep_fpc, wide_fpc) = (
            row.files as f64 / row.commits as f64,
            wide.files as f64 / wide.commits as f64,
        );
        assert!(
            deep_fpc <= wide_fpc,
            "{repo}: deep scenario ({deep_fpc:.1} files/commit) is not narrower \
             than wide ({wide_fpc:.1}) — commit selection is broken and the \
             per-hunk paths are going unmeasured"
        );
        let ratio = |r: &Row| r.hunks as f64 / r.files.max(1) as f64;
        println!(
            "  {repo}: {:.1} hunks/file deep vs {:.1} wide",
            ratio(row),
            ratio(wide)
        );
    }

    if std::env::var("UPDATE_BENCH_BASELINE").is_ok() {
        std::fs::write(
            BASELINE,
            format!("{}\n", serde_json::to_string_pretty(&recorded).unwrap()),
        )
        .expect("write bench baseline");
        println!("\nbench: baseline recorded ({BASELINE})");
    } else if baseline.is_empty() {
        println!("\nbench: no baseline yet — record one with UPDATE_BENCH_BASELINE=1");
    }
}
