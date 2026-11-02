//! Experiment: would static-embedding similarity between hunks (model2vec)
//! improve ordo's clusters?
//!
//! Clusters are connected components over hunk links, so adding a similarity
//! edge is exactly a union of the two clusters it joins. This harness runs the
//! engine unchanged and simulates the extra edges on its output, so it needs no
//! engine edits.
//!
//! Two signals per threshold, over the same corpora as `tests/corpus.rs`:
//! - `split`: single commits ordo cuts into more than one cluster. The corpora
//!   are picked for single-concern commits, so a drop is a missed link found.
//! - `merged`: squashed parent/child pairs on disjoint files that end up sharing
//!   a cluster. A rise is a false link added.
//!
//!   ORDO_CORPUS=~/.cache/ordo-corpus M2V_MODEL=<model dir> \
//!     cargo run --release --example m2v --features m2v [-- click,ripgrep,vue]
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use model2vec_rs::model::StaticModel;
use ordo::model::{Input, Output};

#[path = "../tests/common/mod.rs"]
mod common;
use common::{commit_input, corpus_dir, git, parse_manifest, range_input};

const THRESHOLDS: [f32; 6] = [0.95, 0.9, 0.85, 0.8, 0.75, 0.7];

/// One engine run, reduced to what the simulation needs: each non-noise hunk's
/// path, its baseline cluster, and its embedding.
struct Run {
    path: Vec<String>,
    label: Vec<String>,
    cluster: Vec<usize>,
    emb: Vec<Vec<f32>>,
}

/// Which similar pairs become edges: every pair over the threshold, only pairs
/// in different files, only mutual nearest neighbours, only pairs in the same
/// file; or, as a model-free control, every pair in the same file.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    All,
    Cross,
    Mutual,
    SameFile,
    File,
}

fn lines(text: &str, [start, count]: [usize; 2]) -> impl Iterator<Item = &str> {
    text.lines().skip(start.saturating_sub(1)).take(count)
}

fn reduce(model: &StaticModel, input: Input, embed_time: &mut Duration) -> (Run, Output) {
    let sides: HashMap<String, (String, String)> = input
        .changes
        .iter()
        .map(|c| {
            let side = |s: &Option<String>| s.clone().unwrap_or_default();
            (c.path.clone(), (side(&c.old), side(&c.new)))
        })
        .collect();
    let out = ordo::run(input);
    let cluster_of: HashMap<&str, usize> = out
        .clusters
        .iter()
        .enumerate()
        .flat_map(|(ci, c)| c.iter().map(move |id| (id.as_str(), ci)))
        .collect();
    let (mut path, mut label, mut cluster, mut texts) = (vec![], vec![], vec![], vec![]);
    for f in &out.files {
        let (old, new) = &sides[&f.path];
        for h in f.hunks.iter().filter(|h| !h.noise) {
            let Some(&ci) = cluster_of.get(h.id.as_str()) else {
                continue;
            };
            let mut t = format!(
                "{}\n{}\n{}\n{}\n",
                f.path,
                h.enclosing.as_deref().unwrap_or(""),
                h.defines.join(" "),
                h.uses.join(" ")
            );
            for l in lines(old, h.old_range).chain(lines(new, h.new_range)) {
                t.push_str(l);
                t.push('\n');
            }
            path.push(f.path.clone());
            label.push(format!(
                "{}:{} {}",
                f.path,
                h.new_range[0],
                h.enclosing.as_deref().unwrap_or("-")
            ));
            cluster.push(ci);
            texts.push(t);
        }
    }
    let t0 = Instant::now();
    let emb = model.encode(&texts);
    *embed_time += t0.elapsed();
    (
        Run {
            path,
            label,
            cluster,
            emb,
        },
        out,
    )
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

/// The similarity edges `mode` keeps at threshold `t`, between hunks the
/// engine left in different clusters.
fn edges(run: &Run, t: f32, mode: Mode) -> Vec<(usize, usize, f32)> {
    let n = run.emb.len();
    let sim = |i: usize, j: usize| cos(&run.emb[i], &run.emb[j]);
    let apart = |i: usize, j: usize| run.cluster[i] != run.cluster[j];
    let best: Vec<Option<usize>> = (0..n)
        .map(|i| {
            (0..n)
                .filter(|&j| j != i && apart(i, j))
                .max_by(|&a, &b| sim(i, a).total_cmp(&sim(i, b)))
        })
        .collect();
    let mut out = vec![];
    for i in 0..n {
        for j in i + 1..n {
            if !apart(i, j) {
                continue;
            }
            let keep = match mode {
                Mode::All => true,
                Mode::Cross => run.path[i] != run.path[j],
                Mode::Mutual => best[i] == Some(j) && best[j] == Some(i),
                Mode::SameFile | Mode::File => run.path[i] == run.path[j],
            };
            if keep && mode == Mode::File {
                out.push((i, j, 1.0));
            } else if keep {
                let c = sim(i, j);
                if c >= t {
                    out.push((i, j, c));
                }
            }
        }
    }
    out
}

/// Each non-noise hunk's cluster root once `links` are added.
fn merge(run: &Run, links: &[(usize, usize, f32)]) -> Vec<usize> {
    let n = run.cluster.iter().max().map_or(0, |m| m + 1);
    let mut p: Vec<usize> = (0..n).collect();
    for &(i, j, _) in links {
        let (a, b) = (find(&mut p, run.cluster[i]), find(&mut p, run.cluster[j]));
        p[a] = b;
    }
    run.cluster.iter().map(|&c| find(&mut p, c)).collect()
}

/// (clusters, hunks in the largest one, singleton clusters)
fn shape(roots: &[usize]) -> (usize, usize, usize) {
    let mut size: HashMap<usize, usize> = HashMap::new();
    for &r in roots {
        *size.entry(r).or_default() += 1;
    }
    let top = size.values().copied().max().unwrap_or(0);
    let single = size.values().filter(|&&s| s == 1).count();
    (size.len(), top, single)
}

#[derive(Default, Clone)]
struct Tally {
    commits: usize,
    split: usize,
    clusters: usize,
    singletons: usize,
    hunks: usize,
    top: usize,
    pairs: usize,
    merged: usize,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.commits += o.commits;
        self.split += o.split;
        self.clusters += o.clusters;
        self.singletons += o.singletons;
        self.hunks += o.hunks;
        self.top += o.top;
        self.pairs += o.pairs;
        self.merged += o.merged;
    }

    fn row(&self, repo: &str, label: &str) {
        println!(
            "{:<14}{:>6}{:>8}{:>8}{:>10}{:>8}{:>7.1}%{:>7}{:>8}",
            repo,
            label,
            self.commits,
            self.split,
            self.clusters,
            self.singletons,
            100.0 * self.top as f64 / self.hunks.max(1) as f64,
            self.pairs,
            self.merged
        );
    }
}

/// Joins worth a human look, written when `M2V_DUMP` names a file.
struct Dump {
    out: Option<std::fs::File>,
    t: f32,
    budget: usize,
}

impl Dump {
    fn write(&mut self, repo: &str, kind: &str, run: &Run, links: &[(usize, usize, f32)]) {
        use std::io::Write;
        let Some(f) = self.out.as_mut() else { return };
        for &(i, j, c) in links {
            if self.budget == 0 {
                return;
            }
            self.budget -= 1;
            let _ = writeln!(
                f,
                "{repo}\t{kind}\t{c:.3}\t{}\t{}",
                run.label[i], run.label[j]
            );
        }
    }
}

fn files_of(out: &Output) -> Vec<String> {
    out.files.iter().map(|f| f.path.clone()).collect()
}

fn sweep(
    model: &StaticModel,
    repo: &str,
    dir: &Path,
    rev: &str,
    max: usize,
    mode: Mode,
    dump: &mut Dump,
    embed_time: &mut Duration,
) -> Vec<Tally> {
    let settings: Vec<Option<f32>> = std::iter::once(None).chain(THRESHOLDS.map(Some)).collect();
    let mut tallies = vec![Tally::default(); settings.len()];
    let links_at = |run: &Run, s: Option<f32>| s.map_or(vec![], |t| edges(run, t, mode));
    let listing = git(dir, &["rev-list", "--max-count", &max.to_string(), rev]);
    let mut child: Option<(String, String, Vec<String>)> = None;
    dump.budget = 40;
    for sha in listing.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let Some((parent, input)) = commit_input(dir, sha) else {
            child = None;
            continue;
        };
        let (run, out) = reduce(model, input, embed_time);
        let files = files_of(&out);
        if !run.cluster.is_empty() {
            for (s, tally) in settings.iter().zip(&mut tallies) {
                let links = links_at(&run, *s);
                if *s == Some(dump.t) {
                    let kind = format!("join {}", &sha[..8]);
                    dump.write(repo, &kind, &run, &links);
                }
                let (k, top, single) = shape(&merge(&run, &links));
                tally.commits += 1;
                tally.split += (k > 1) as usize;
                tally.clusters += k;
                tally.singletons += single;
                tally.hunks += run.cluster.len();
                tally.top += top;
            }
        }
        if let Some((c, _, c_files)) = child.as_ref().filter(|(_, cp, _)| cp == sha) {
            if !files.iter().any(|f| c_files.contains(f)) {
                if let Some(input) = range_input(dir, &parent, c) {
                    let (prun, _) = reduce(model, input, embed_time);
                    let owner: Vec<bool> = prun.path.iter().map(|p| c_files.contains(p)).collect();
                    for (s, tally) in settings.iter().zip(&mut tallies) {
                        let links = links_at(&prun, *s);
                        if *s == Some(dump.t) {
                            let false_links: Vec<_> = links
                                .iter()
                                .copied()
                                .filter(|&(i, j, _)| owner[i] != owner[j])
                                .collect();
                            dump.budget += false_links.len();
                            dump.write(repo, "FALSE", &prun, &false_links);
                        }
                        let roots = merge(&prun, &links);
                        let mut by_root: HashMap<usize, (bool, bool)> = HashMap::new();
                        for (r, &o) in roots.iter().zip(&owner) {
                            let e = by_root.entry(*r).or_default();
                            if o {
                                e.0 = true
                            } else {
                                e.1 = true
                            }
                        }
                        tally.pairs += 1;
                        tally.merged += by_root.values().any(|&(a, b)| a && b) as usize;
                    }
                }
            }
        }
        child = Some((sha.to_string(), parent, files));
    }
    tallies
}

fn main() {
    let root = corpus_dir().expect("set ORDO_CORPUS to a populated corpus dir");
    let model_dir = std::env::var("M2V_MODEL").expect("set M2V_MODEL to a model2vec model dir");
    let model = StaticModel::from_pretrained(&model_dir, None, None, None).expect("load model");
    let mode = match std::env::var("M2V_MODE").as_deref() {
        Ok("cross") => Mode::Cross,
        Ok("mutual") => Mode::Mutual,
        Ok("samefile") => Mode::SameFile,
        Ok("file") => Mode::File,
        _ => Mode::All,
    };
    let mut dump = Dump {
        out: std::env::var("M2V_DUMP")
            .ok()
            .map(|p| std::fs::File::create(p).expect("create M2V_DUMP")),
        t: std::env::var("M2V_DUMP_T")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.9),
        budget: 0,
    };
    let want = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "click,ripgrep,vue".into());
    let all = want == "all";
    let want: Vec<&str> = want.split(',').collect();
    println!(
        "model {model_dir} mode {}",
        std::env::var("M2V_MODE").unwrap_or_else(|_| "all".into())
    );
    println!(
        "{:<14}{:>6}{:>8}{:>8}{:>10}{:>8}{:>8}{:>7}{:>8}",
        "repo", "t", "commits", "split", "clusters", "single", "top", "pairs", "merged"
    );
    let label = |i: usize| {
        if i == 0 {
            "base".to_string()
        } else {
            format!("{:.2}", THRESHOLDS[i - 1])
        }
    };
    let mut total = vec![Tally::default(); THRESHOLDS.len() + 1];
    let mut embed_time = Duration::ZERO;
    for repo in parse_manifest()
        .iter()
        .filter(|r| all || want.contains(&r.name.as_str()))
    {
        let dir = root.join(&repo.name);
        if !dir.is_dir() {
            eprintln!("{}: not fetched, skipped", repo.name);
            continue;
        }
        let tallies = sweep(
            &model,
            &repo.name,
            &dir,
            &repo.rev,
            repo.max_commits,
            mode,
            &mut dump,
            &mut embed_time,
        );
        for (i, t) in tallies.iter().enumerate() {
            t.row(&repo.name, &label(i));
            total[i].add(t);
        }
    }
    for (i, t) in total.iter().enumerate() {
        t.row("TOTAL", &label(i));
    }
    println!("embedding time {:.2}s", embed_time.as_secs_f64());
}
