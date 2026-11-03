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
//!
//! `M2V_MODE=intent` asks instead whether a commit's mean hunk embedding
//! predicts its conventional-commit type. Only commits whose subject carries
//! one of `INTENTS` count. Each repo is held out in turn and scored by nearest
//! class centroid (raw and repo-centred), against the training repos' majority
//! type and a path-only rule (docs if every hunk is markdown, test if every
//! hunk is in a test file).
//! `M2V_DUMP` gets one line per held-out commit.
//!
//! `M2V_MODE=search` and `M2V_MODE=similar` test retrieval within one repo,
//! which takes the repo and language out of the comparison: a commit's subject
//! finding its own hunks, and a commit's diff finding related earlier commits,
//! against lexical baselines. `M2V_MODEL` takes several dirs, comma-separated,
//! scored side by side; `M2V_DUMP` gets each method's top three for a sample.
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use model2vec_rs::model::StaticModel;
use ordo::model::{Input, Output};

#[path = "../tests/common/mod.rs"]
mod common;
use common::{commit_input, corpus_dir, git, parse_manifest, range_input, Repo};

const THRESHOLDS: [f32; 6] = [0.95, 0.9, 0.85, 0.8, 0.75, 0.7];

/// One engine run, reduced to what the simulation needs: each non-noise hunk's
/// path, its baseline cluster, and its embedding.
struct Run {
    path: Vec<String>,
    label: Vec<String>,
    enclosing: Vec<Option<String>>,
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
    let (mut run, texts, out) = extract(input);
    let t0 = Instant::now();
    run.emb = model.encode(&texts);
    *embed_time += t0.elapsed();
    (run, out)
}

/// The engine's non-noise clustered hunks, each with the text the models embed:
/// path, enclosing definition, defined and used names, then both sides' lines.
fn extract(input: Input) -> (Run, Vec<String>, Output) {
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
    let (mut path, mut label, mut enclosing, mut cluster, mut texts) =
        (vec![], vec![], vec![], vec![], vec![]);
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
            enclosing.push(h.enclosing.clone());
            cluster.push(ci);
            texts.push(t);
        }
    }
    let run = Run {
        path,
        label,
        enclosing,
        cluster,
        emb: vec![],
    };
    (run, texts, out)
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

/// Conventional-commit types the intent mode predicts. Matched lower-case only,
/// so numpy-style `FIX:`/`DOC:` tags stay out; `doc` and `tests` fold in.
const INTENTS: [&str; 7] = ["feat", "fix", "refactor", "docs", "test", "perf", "chore"];

fn intent_named(name: &str) -> Option<usize> {
    let name = match name {
        "doc" => "docs",
        "tests" => "test",
        n => n,
    };
    INTENTS.iter().position(|&i| i == name)
}

fn intent_of(subject: &str) -> Option<usize> {
    let head = subject.split_once(':')?.0;
    intent_named(
        head.split_once('(')
            .map_or(head, |(t, _)| t)
            .trim_end_matches('!'),
    )
}

/// The path-only control's guess, when every hunk is in docs or in tests.
fn path_hint(paths: &[String]) -> Option<usize> {
    let docs = |p: &String| p.ends_with(".md") || p.ends_with(".markdown");
    let test = |p: &String| {
        p.to_lowercase()
            .split(['/', '.', '_', '-'])
            .any(|w| matches!(w, "test" | "tests" | "spec"))
    };
    if paths.iter().all(docs) {
        intent_named("docs")
    } else if paths.iter().all(test) {
        intent_named("test")
    } else {
        None
    }
}

fn mean<'a>(vs: impl Iterator<Item = &'a Vec<f32>>) -> Vec<f32> {
    let (mut sum, mut n) = (Vec::<f32>::new(), 0);
    for v in vs {
        sum.resize(v.len(), 0.0);
        sum.iter_mut().zip(v).for_each(|(s, x)| *s += x);
        n += 1;
    }
    sum.iter_mut().for_each(|s| *s /= n.max(1) as f32);
    sum
}

/// One labelled commit: its type, the mean of its hunk embeddings, and that
/// mean less its repo's mean commit vector.
struct Sample {
    repo: usize,
    label: usize,
    hint: Option<usize>,
    sha: String,
    subject: String,
    emb: Vec<f32>,
    centred: Vec<f32>,
}

/// Every commit in `repos` with a conventional type and at least one embedded
/// hunk, and per repo the typed commits dropped for having none.
fn intent_samples(
    model: &StaticModel,
    root: &Path,
    repos: &[Repo],
    embed_time: &mut Duration,
) -> (Vec<Sample>, Vec<usize>) {
    let mut samples = vec![];
    let mut skipped = vec![0; repos.len()];
    for (ri, repo) in repos.iter().enumerate() {
        let dir = root.join(&repo.name);
        let max = repo.max_commits.to_string();
        let log = git(
            &dir,
            &["log", "--format=%H %s", "--max-count", &max, &repo.rev],
        );
        for (sha, subject) in log.lines().filter_map(|l| l.split_once(' ')) {
            let Some(label) = intent_of(subject) else {
                continue;
            };
            let run = commit_input(&dir, sha).map(|(_, input)| reduce(model, input, embed_time).0);
            let Some(run) = run.filter(|r| !r.emb.is_empty()) else {
                skipped[ri] += 1;
                continue;
            };
            samples.push(Sample {
                repo: ri,
                label,
                hint: path_hint(&run.path),
                sha: sha.to_string(),
                subject: subject.to_string(),
                emb: mean(run.emb.iter()),
                centred: vec![],
            });
        }
    }
    let repo_means: Vec<Vec<f32>> = (0..repos.len())
        .map(|ri| mean(samples.iter().filter(|s| s.repo == ri).map(|s| &s.emb)))
        .collect();
    for s in &mut samples {
        s.centred = s
            .emb
            .iter()
            .zip(&repo_means[s.repo])
            .map(|(x, m)| x - m)
            .collect();
    }
    (samples, skipped)
}

/// Per-type F1 over (truth, prediction) pairs; None for a type absent from the
/// truth, so every method's macro-F1 averages over the same types.
fn f1s(pairs: &[(usize, usize)]) -> Vec<Option<f64>> {
    (0..INTENTS.len())
        .map(|c| {
            let tp = pairs.iter().filter(|&&(t, p)| t == c && p == c).count();
            let miss = pairs.iter().filter(|&&(t, p)| (t == c) != (p == c)).count();
            pairs
                .iter()
                .any(|&(t, _)| t == c)
                .then(|| 2.0 * tp as f64 / (2 * tp + miss) as f64)
        })
        .collect()
}

fn scores(pairs: &[(usize, usize)]) -> (f64, f64) {
    let acc = pairs.iter().filter(|(t, p)| t == p).count() as f64 / pairs.len().max(1) as f64;
    let f: Vec<f64> = f1s(pairs).into_iter().flatten().collect();
    (acc, f.iter().sum::<f64>() / f.len().max(1) as f64)
}

fn score_row(repo: &str, n: usize, methods: &[Vec<(usize, usize)>]) {
    print!("{repo:<14}{n:>5}");
    for m in methods {
        let (acc, f1) = scores(m);
        print!("{acc:>10.3}{f1:>7.3}");
    }
    println!();
}

/// The class whose centroid is most cosine-similar to `v`, and that similarity.
fn nearest(v: &[f32], centroids: &[(usize, Vec<f32>)]) -> (usize, f32) {
    centroids
        .iter()
        .map(|(c, m)| (*c, cos(v, m)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}

/// Methods scored per held-out repo: the training majority, the path rule, and
/// nearest centroid on raw and on repo-centred commit vectors. Centring on the
/// repo's own mean uses no labels and strips the language and project style
/// that otherwise decide which centroid is nearest.
const METHODS: [&str; 4] = ["maj", "path", "m2v", "m2v-c"];

fn intent_mode(
    model: &StaticModel,
    root: &Path,
    repos: &[Repo],
    mut dump: Option<&mut std::fs::File>,
) {
    use std::io::Write;
    let mut embed_time = Duration::ZERO;
    let (samples, skipped) = intent_samples(model, root, repos, &mut embed_time);

    print!("{:<14}{:>5}{:>6}", "repo", "n", "skip");
    INTENTS.iter().for_each(|i| print!("{i:>9}"));
    println!();
    for (ri, repo) in repos.iter().enumerate() {
        let mut count = [0; INTENTS.len()];
        samples
            .iter()
            .filter(|s| s.repo == ri)
            .for_each(|s| count[s.label] += 1);
        print!(
            "{:<14}{:>5}{:>6}",
            repo.name,
            count.iter().sum::<usize>(),
            skipped[ri]
        );
        count.iter().for_each(|c| print!("{c:>9}"));
        println!();
    }

    print!("\n{:<14}{:>5}", "held out", "n");
    METHODS
        .iter()
        .for_each(|m| print!("{:>10}{:>7}", format!("{m} acc"), "F1"));
    println!();
    let mut pooled = vec![vec![]; METHODS.len()];
    for (ri, repo) in repos.iter().enumerate() {
        let (test, train): (Vec<&Sample>, Vec<&Sample>) =
            samples.iter().partition(|s| s.repo == ri);
        if test.is_empty() {
            continue;
        }
        if train.is_empty() {
            eprintln!(
                "{}: no training commits in the other repos, not scored",
                repo.name
            );
            continue;
        }
        let mut count = [0; INTENTS.len()];
        train.iter().for_each(|s| count[s.label] += 1);
        let majority = (0..INTENTS.len()).max_by_key(|&c| count[c]).unwrap();
        let centroids = |vector: fn(&Sample) -> &Vec<f32>| -> Vec<(usize, Vec<f32>)> {
            (0..INTENTS.len())
                .filter(|&c| count[c] > 0)
                .map(|c| {
                    (
                        c,
                        mean(train.iter().filter(|s| s.label == c).map(|s| vector(s))),
                    )
                })
                .collect()
        };
        let (raw_centroids, centred_centroids) = (centroids(|s| &s.emb), centroids(|s| &s.centred));
        let mut fold = vec![vec![]; METHODS.len()];
        for s in &test {
            let (raw, raw_sim) = nearest(&s.emb, &raw_centroids);
            let (cen, cen_sim) = nearest(&s.centred, &centred_centroids);
            fold[0].push((s.label, majority));
            fold[1].push((s.label, s.hint.unwrap_or(majority)));
            fold[2].push((s.label, raw));
            fold[3].push((s.label, cen));
            if let Some(f) = dump.as_mut() {
                let _ = writeln!(
                    f,
                    "{}\t{}\t{}\t{}\t{raw_sim:.3}\t{}\t{cen_sim:.3}\t{}",
                    repo.name,
                    &s.sha[..8],
                    INTENTS[s.label],
                    INTENTS[raw],
                    INTENTS[cen],
                    s.subject
                );
            }
        }
        score_row(&repo.name, test.len(), &fold);
        pooled.iter_mut().zip(fold).for_each(|(p, f)| p.extend(f));
    }
    if pooled[0].is_empty() {
        eprintln!("no repo had both typed commits and training commits elsewhere");
        return;
    }
    score_row("POOLED", pooled[0].len(), &pooled);

    for (mi, pairs) in pooled.iter().enumerate().skip(2) {
        println!(
            "\n{} confusion, pooled (rows truth, columns prediction)",
            METHODS[mi]
        );
        print!("{:<10}", "");
        INTENTS.iter().for_each(|i| print!("{i:>9}"));
        println!("{:>9}", "F1");
        let f1 = f1s(pairs);
        for (t, name) in INTENTS.iter().enumerate() {
            print!("{name:<10}");
            for p in 0..INTENTS.len() {
                let n = pairs.iter().filter(|&&pair| pair == (t, p)).count();
                print!("{n:>9}");
            }
            println!("{:>9}", f1[t].map_or("-".into(), |f| format!("{f:.3}")));
        }
    }
    println!("embedding time {:.2}s", embed_time.as_secs_f64());
}

/// Lower-case word pieces of identifiers: split on anything not alphanumeric and
/// on camelCase humps, dropping single characters and bare numbers.
fn tokens(text: &str) -> Vec<String> {
    let mut out = vec![];
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = word.chars().collect();
        let mut start = 0;
        for i in 1..=chars.len() {
            let cut = i == chars.len()
                || (chars[i].is_uppercase()
                    && (chars[i - 1].is_lowercase()
                        || chars.get(i + 1).is_some_and(|c| c.is_lowercase())
                            && chars[i - 1].is_uppercase()));
            if cut {
                let piece: String = chars[start..i].iter().collect::<String>().to_lowercase();
                if piece.chars().count() > 1 && !piece.chars().all(|c| c.is_ascii_digit()) {
                    out.push(piece);
                }
                start = i;
            }
        }
    }
    out
}

/// Okapi BM25 (k1 1.2, b 0.75) over token lists.
struct Bm25 {
    tf: Vec<HashMap<String, f32>>,
    len: Vec<f32>,
    avg: f32,
    df: HashMap<String, f32>,
}

impl Bm25 {
    fn new(docs: &[Vec<String>]) -> Self {
        let mut df: HashMap<String, f32> = HashMap::new();
        let tf: Vec<HashMap<String, f32>> = docs
            .iter()
            .map(|d| {
                let mut m: HashMap<String, f32> = HashMap::new();
                d.iter()
                    .for_each(|t| *m.entry(t.clone()).or_default() += 1.0);
                m.keys()
                    .for_each(|t| *df.entry(t.clone()).or_default() += 1.0);
                m
            })
            .collect();
        let len: Vec<f32> = docs.iter().map(|d| d.len() as f32).collect();
        let avg = len.iter().sum::<f32>() / len.len().max(1) as f32;
        Bm25 { tf, len, avg, df }
    }

    /// Every document's score for the distinct terms of `query`.
    fn scores(&self, query: &[String]) -> Vec<f32> {
        let n = self.tf.len() as f32;
        let mut terms: Vec<&String> = query.iter().collect();
        terms.sort();
        terms.dedup();
        let idf: Vec<(&String, f32)> = terms
            .into_iter()
            .filter_map(|t| {
                let df = *self.df.get(t)?;
                Some((t, (1.0 + (n - df + 0.5) / (df + 0.5)).ln()))
            })
            .collect();
        self.tf
            .iter()
            .zip(&self.len)
            .map(|(tf, &len)| {
                let norm = 1.2 * (0.25 + 0.75 * len / self.avg.max(1.0));
                idf.iter()
                    .filter_map(|&(t, w)| tf.get(t).map(|&f| w * f * 2.2 / (f + norm)))
                    .sum()
            })
            .collect()
    }
}

/// Reciprocal-rank fusion (k = 60) of two rankings over the same items.
fn rrf(a: &[usize], b: &[usize]) -> Vec<f32> {
    let mut s = vec![0.0; a.len().max(b.iter().max().map_or(0, |m| m + 1))];
    for ranking in [a, b] {
        for (r, &i) in ranking.iter().enumerate() {
            s[i] += 1.0 / (61.0 + r as f32);
        }
    }
    s
}

/// `items` by descending score; ties go to the lower `tie` key.
fn ranked(items: &[usize], score: &[f32], tie: impl Fn(usize) -> usize) -> Vec<usize> {
    let mut v = items.to_vec();
    v.sort_by(|&a, &b| score[b].total_cmp(&score[a]).then(tie(a).cmp(&tie(b))));
    v
}

/// Short display name of a model dir: the hub repo name when it is a hub cache path.
fn model_name(dir: &str) -> String {
    dir.split('/')
        .find_map(|s| s.strip_prefix("models--"))
        .and_then(|s| s.rsplit("--").next())
        .unwrap_or(dir.trim_end_matches('/').rsplit('/').next().unwrap_or(dir))
        .to_string()
}

const CONVENTIONAL: [&str; 13] = [
    "feat", "fix", "refactor", "docs", "doc", "test", "tests", "perf", "chore", "build", "ci",
    "style", "revert",
];

/// A subject without its conventional `type(scope)!:`, `[Tag]` or `ENH:` head,
/// its trailing `(#123)` and its area head (`lib:`, `` `nvm ls`: ``), as
/// (area, rest). The area is the conventional scope or the area head.
fn split_subject(subject: &str) -> (Option<String>, &str) {
    let mut s = subject.trim();
    if let Some(r) = s.strip_suffix(')').and_then(|r| r.rsplit_once(" (#")) {
        if r.1.chars().all(|c| c.is_ascii_digit()) {
            s = r.0.trim_end();
        }
    }
    if let Some(rest) = s.strip_prefix('[').and_then(|r| r.split_once(']')) {
        s = rest.1.trim_start();
    }
    let Some((head, rest)) = s.split_once(": ") else {
        return (None, s);
    };
    let (ty, scope) = match head.trim_end_matches('!').split_once('(') {
        Some((t, sc)) => (t, Some(sc.trim_end_matches(')'))),
        None => (head.trim_end_matches('!'), None),
    };
    if CONVENTIONAL.contains(&ty) {
        return (scope.map(str::to_lowercase), rest);
    }
    let area = head.trim_matches('`');
    if head.contains(' ') && !head.starts_with('`') || area.len() > 40 {
        return (None, s);
    }
    if area.chars().all(|c| !c.is_lowercase()) {
        return (None, rest);
    }
    (Some(area.to_lowercase()), rest)
}

/// One repo's newest non-merge commits that have at least one embedded hunk,
/// newest first, with their hunks flattened across commits.
struct History {
    commits: Vec<Commit>,
    hunks: Vec<Hunk>,
}

struct Hunk {
    commit: usize,
    label: String,
    tokens: Vec<String>,
    /// One per model.
    emb: Vec<Vec<f32>>,
}

struct Commit {
    sha: String,
    subject: String,
    body: String,
    /// The merge that brought it in, so commits of one PR are never each other's answer.
    merge: Option<usize>,
    hunks: std::ops::Range<usize>,
    files: Vec<String>,
    defs: Vec<String>,
}

fn history(models: &[StaticModel], dir: &Path, repo: &Repo, embed_time: &mut Duration) -> History {
    let max = repo.max_commits.to_string();
    let log = git(
        dir,
        &[
            "log",
            "--no-merges",
            "--format=%H%x1f%s%x1f%b%x1e",
            "--max-count",
            &max,
            &repo.rev,
        ],
    );
    let mut merge_of: HashMap<String, usize> = HashMap::new();
    let oldest = log
        .rsplit('\x1e')
        .find_map(|r| r.trim().split('\x1f').next().filter(|s| !s.is_empty()));
    let not_older = oldest.map(|o| format!("^{o}"));
    let mut args = vec!["rev-list", "--merges", &repo.rev];
    args.extend(not_older.as_deref());
    let merges = git(dir, &args);
    for (mi, m) in merges.lines().enumerate() {
        let range = format!("{m}^1..{m}^2");
        for sha in git(dir, &["rev-list", "--no-merges", &range]).lines() {
            merge_of.entry(sha.to_string()).or_insert(mi);
        }
    }
    let mut h = History {
        commits: vec![],
        hunks: vec![],
    };
    for rec in log.split('\x1e') {
        let mut f = rec.trim_start_matches('\n').splitn(3, '\x1f');
        let (Some(sha), Some(subject), body) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let Some((_, input)) = commit_input(dir, sha) else {
            continue;
        };
        let (run, texts, _) = extract(input);
        if texts.is_empty() {
            continue;
        }
        let t0 = Instant::now();
        let embs: Vec<Vec<Vec<f32>>> = models.iter().map(|m| m.encode(&texts)).collect();
        *embed_time += t0.elapsed();
        let ci = h.commits.len();
        let start = h.hunks.len();
        for (i, t) in texts.iter().enumerate() {
            let emb = embs.iter().map(|e| e[i].clone()).collect();
            h.hunks.push(Hunk {
                commit: ci,
                label: run.label[i].clone(),
                tokens: tokens(t),
                emb,
            });
        }
        let mut files = run.path.clone();
        files.dedup();
        let mut defs: Vec<String> = run
            .path
            .iter()
            .zip(&run.enclosing)
            .filter_map(|(p, e)| e.as_ref().map(|e| format!("{p}::{e}")))
            .collect();
        defs.sort();
        defs.dedup();
        h.commits.push(Commit {
            sha: sha.to_string(),
            subject: subject.to_string(),
            body: body.unwrap_or("").to_string(),
            merge: merge_of.get(sha).copied(),
            hunks: start..h.hunks.len(),
            files,
            defs,
        });
    }
    h
}

/// Running sums of one method's per-query scores.
#[derive(Default, Clone, Copy)]
struct Score {
    n: usize,
    at_k: f64,
    rr: f64,
}

impl Score {
    /// Adds one query: `at_k` is the caller's recall or precision at its cutoff.
    fn add(&mut self, ranking: &[usize], relevant: impl Fn(usize) -> bool, at_k: f64) {
        self.n += 1;
        self.at_k += at_k;
        self.rr += ranking
            .iter()
            .position(|&i| relevant(i))
            .map_or(0.0, |r| 1.0 / (r + 1) as f64);
    }

    fn merge(&mut self, o: &Score) {
        self.n += o.n;
        self.at_k += o.at_k;
        self.rr += o.rr;
    }

    fn cell(&self) -> String {
        let n = self.n.max(1) as f64;
        format!("{:.3} {:.3}", self.at_k / n, self.rr / n)
    }
}

/// Column widths: each method's name, at least wide enough for its two numbers.
fn widths(methods: &[String]) -> Vec<usize> {
    methods.iter().map(|m| m.len().max(11) + 2).collect()
}

fn header(methods: &[String], metric: &str) {
    print!("{:<14}{:>6}{:>8}", "repo", "n", "pool");
    for (m, w) in methods.iter().zip(widths(methods)) {
        print!("{m:>w$}");
    }
    print!("\n{:<28}", "");
    for w in widths(methods) {
        print!("{:>w$}", format!("{metric} MRR"));
    }
    println!();
}

fn row(repo: &str, pool: usize, methods: &[String], scores: &[Score]) {
    print!(
        "{repo:<14}{:>6}{pool:>8}",
        scores.first().map_or(0, |s| s.n)
    );
    for (s, w) in scores.iter().zip(widths(methods)) {
        print!("{:>w$}", s.cell());
    }
    println!();
}

/// A deterministic sample of `k` queries: the lowest by sha, which is uniform.
fn sample(queries: &[usize], commits: &[Commit], k: usize) -> Vec<usize> {
    let mut v = queries.to_vec();
    v.sort_by_key(|&q| &commits[q].sha);
    v.truncate(k);
    v
}

/// Search: the query is a commit's subject without its conventional head, the
/// relevant hunks are that commit's own, and the pool is every hunk of the
/// repo's window. BM25 over identifier tokens of the hunk text vs cosine to the
/// embedded query, per model, plus their rank fusion.
fn search_mode(
    models: &[StaticModel],
    names: &[String],
    root: &Path,
    repos: &[Repo],
    mut dump: Option<&mut std::fs::File>,
) {
    use std::io::Write;
    let mut methods = vec!["bm25".to_string()];
    methods.extend(names.iter().cloned());
    methods.extend(names.iter().map(|n| format!("rrf-{n}")));
    header(&methods, " R@10");
    let mut total = vec![Score::default(); methods.len()];
    let (mut pool_total, mut embed_time) = (0, Duration::ZERO);
    for repo in repos {
        let h = history(models, &root.join(&repo.name), repo, &mut embed_time);
        let docs: Vec<Vec<String>> = h.hunks.iter().map(|x| x.tokens.clone()).collect();
        let bm25 = Bm25::new(&docs);
        let all: Vec<usize> = (0..h.hunks.len()).collect();
        let mut scores = vec![Score::default(); methods.len()];
        let mut queries = vec![];
        let mut rankings: HashMap<usize, Vec<Vec<usize>>> = HashMap::new();
        for (ci, c) in h.commits.iter().enumerate() {
            let query = split_subject(&c.subject).1;
            let qt = tokens(query);
            if qt.is_empty() {
                continue;
            }
            let own = |i: usize| h.hunks[i].commit == ci;
            // Ties go against the answer: a zero BM25 score is no evidence.
            let order = |s: &[f32]| ranked(&all, s, |i| own(i) as usize);
            let lexical = order(&bm25.scores(&qt));
            let t0 = Instant::now();
            let qe: Vec<Vec<f32>> = models
                .iter()
                .map(|m| m.encode(&[query.to_string()]).remove(0))
                .collect();
            embed_time += t0.elapsed();
            let dense: Vec<Vec<usize>> = (0..models.len())
                .map(|mi| {
                    let s: Vec<f32> = h.hunks.iter().map(|x| cos(&qe[mi], &x.emb[mi])).collect();
                    order(&s)
                })
                .collect();
            let mut rs = vec![lexical.clone()];
            rs.extend(dense.iter().cloned());
            rs.extend(dense.iter().map(|d| order(&rrf(&lexical, d))));
            let want = c.hunks.len().min(10) as f64;
            for (s, r) in scores.iter_mut().zip(&rs) {
                let hits = r.iter().take(10).filter(|&&i| own(i)).count() as f64;
                s.add(r, own, hits / want);
            }
            queries.push(ci);
            rankings.insert(ci, rs);
        }
        row(&repo.name, h.hunks.len(), &methods, &scores);
        total.iter_mut().zip(&scores).for_each(|(t, s)| t.merge(s));
        pool_total += h.hunks.len();
        if let Some(f) = dump.as_mut() {
            for q in sample(&queries, &h.commits, 2) {
                let c = &h.commits[q];
                let _ = writeln!(
                    f,
                    "## {} {} | query: {}\n   own hunks: {}",
                    repo.name,
                    &c.sha[..8],
                    split_subject(&c.subject).1,
                    h.hunks[c.hunks.clone()]
                        .iter()
                        .map(|x| x.label.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                for (m, r) in methods.iter().zip(&rankings[&q]).take(1 + models.len()) {
                    let _ = writeln!(f, "  [{m}]");
                    for &i in r.iter().take(3) {
                        let Hunk {
                            commit: oc, label, ..
                        } = &h.hunks[i];
                        let mark = if *oc == q { "*" } else { " " };
                        let _ = writeln!(f, "   {mark} {label}  <- {}", h.commits[*oc].subject);
                    }
                }
            }
        }
    }
    row("POOLED", pool_total, &methods, &total);
    println!("embedding time {:.2}s", embed_time.as_secs_f64());
}

/// Ground-truth proxies for "a similar earlier change".
const PROXIES: [&str; 3] = ["def", "ref", "any"];

/// Which proxies call `old` an answer for `new`: a shared enclosing definition;
/// a shared subject area or issue reference; any of those or a revert/backport.
fn truth(new: &Commit, old: &Commit) -> [bool; 3] {
    let def = new.defs.iter().any(|d| old.defs.contains(d));
    let refs = |s: &str| -> Vec<String> {
        s.split('#')
            .skip(1)
            .map(|r| {
                r.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .filter(|r| !r.is_empty())
            .collect()
    };
    let (na, oa) = (split_subject(&new.subject), split_subject(&old.subject));
    let area = na.0.is_some() && na.0 == oa.0;
    let shared_ref = refs(&new.subject)
        .iter()
        .any(|r| refs(&old.subject).contains(r));
    let bare = |s: &str| {
        let s = s.trim();
        s.rsplit_once(" (#")
            .filter(|(_, n)| n.trim_end_matches(')').chars().all(|c| c.is_ascii_digit()))
            .map_or(s, |(h, _)| h)
            .to_string()
    };
    let reverted = new
        .subject
        .strip_prefix("Revert \"")
        .and_then(|r| r.rsplit_once('"'))
        .is_some_and(|(r, _)| bare(r) == bare(&old.subject));
    let cites = new.body.contains(&format!("reverts commit {}", old.sha))
        || new
            .body
            .contains(&format!("cherry picked from commit {}", old.sha));
    [
        def,
        area || shared_ref,
        def || area || shared_ref || reverted || cites,
    ]
}

/// Similar change: each commit ranks the repo's earlier commits, not counting
/// any brought in by the same merge. Shared-file Jaccard and BM25 over the
/// whole diff's tokens vs cosine of mean hunk embeddings, per model. Ties go to
/// the more recent commit for every method.
fn similar_mode(
    models: &[StaticModel],
    names: &[String],
    root: &Path,
    repos: &[Repo],
    mut dump: Option<&mut std::fs::File>,
) {
    use std::io::Write;
    let mut methods = vec!["file".to_string(), "bm25".to_string()];
    methods.extend(names.iter().cloned());
    methods.extend(names.iter().map(|n| format!("rrf-{n}")));
    let mut out: Vec<Vec<(String, usize, Vec<Score>)>> = vec![vec![]; PROXIES.len()];
    let mut embed_time = Duration::ZERO;
    for repo in repos {
        let h = history(models, &root.join(&repo.name), repo, &mut embed_time);
        let n = h.commits.len();
        let docs: Vec<Vec<String>> = h
            .commits
            .iter()
            .map(|c| {
                h.hunks[c.hunks.clone()]
                    .iter()
                    .flat_map(|x| x.tokens.clone())
                    .collect()
            })
            .collect();
        let bm25 = Bm25::new(&docs);
        let vecs: Vec<Vec<Vec<f32>>> = (0..models.len())
            .map(|mi| {
                h.commits
                    .iter()
                    .map(|c| mean(h.hunks[c.hunks.clone()].iter().map(|x| &x.emb[mi])))
                    .collect()
            })
            .collect();
        let mut scores = vec![vec![Score::default(); methods.len()]; PROXIES.len()];
        let mut queries = vec![];
        let mut rankings: HashMap<usize, Vec<Vec<usize>>> = HashMap::new();
        let mut reverts = 0;
        for q in 0..n {
            let c = &h.commits[q];
            let cands: Vec<usize> = (q + 1..n)
                .filter(|&j| c.merge.is_none() || h.commits[j].merge != c.merge)
                .collect();
            let truths: HashMap<usize, [bool; 3]> = cands
                .iter()
                .map(|&j| (j, truth(c, &h.commits[j])))
                .collect();
            if !truths.values().any(|t| t[2]) {
                continue;
            }
            reverts += truths.values().any(|t| t[2] && !t[0] && !t[1]) as usize;
            let mut full = vec![0.0f32; n];
            let order = |s: &[f32]| ranked(&cands, s, |j| j);
            for &j in &cands {
                let o = &h.commits[j];
                let inter = c.files.iter().filter(|f| o.files.contains(f)).count();
                full[j] = inter as f32 / (c.files.len() + o.files.len() - inter) as f32;
            }
            let mut rs = vec![order(&full), order(&bm25.scores(&docs[q]))];
            for v in &vecs {
                let s: Vec<f32> = (0..n).map(|j| cos(&v[q], &v[j])).collect();
                rs.push(order(&s));
            }
            for mi in 0..models.len() {
                let fused = order(&rrf(&rs[1], &rs[2 + mi]));
                rs.push(fused);
            }
            for (p, ps) in scores.iter_mut().enumerate() {
                let rel = |j: usize| truths.get(&j).is_some_and(|t| t[p]);
                if !cands.iter().any(|&j| rel(j)) {
                    continue;
                }
                for (s, r) in ps.iter_mut().zip(&rs) {
                    let hits = r.iter().take(5).filter(|&&j| rel(j)).count() as f64;
                    s.add(r, rel, hits / 5.0);
                }
            }
            queries.push(q);
            rankings.insert(q, rs);
        }
        for (p, ps) in scores.into_iter().enumerate() {
            out[p].push((repo.name.clone(), n, ps));
        }
        eprintln!(
            "{}: {} commits, {reverts} queries answered only by a revert/backport",
            repo.name, n
        );
        if let Some(f) = dump.as_mut() {
            let files = |c: &Commit| {
                c.files
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            for q in sample(&queries, &h.commits, 2) {
                let c = &h.commits[q];
                let _ = writeln!(
                    f,
                    "## {} {} | {}\n   files: {}",
                    repo.name,
                    &c.sha[..8],
                    c.subject,
                    files(c)
                );
                for (m, r) in methods.iter().zip(&rankings[&q]).take(2 + models.len()) {
                    let _ = writeln!(f, "  [{m}]");
                    for &j in r.iter().take(3) {
                        let o = &h.commits[j];
                        let t = truth(c, o);
                        let mark: String = ["d", "r", "*"]
                            .iter()
                            .zip(t)
                            .map(|(m, b)| if b { *m } else { "." })
                            .collect();
                        let _ = writeln!(f, "   {mark} {} | {}", o.subject, files(o));
                    }
                }
            }
        }
    }
    for (p, rows) in out.iter().enumerate() {
        println!("\nproxy {}", PROXIES[p]);
        header(&methods, "  P@5");
        let mut total = vec![Score::default(); methods.len()];
        let mut pool = 0;
        for (name, n, ps) in rows {
            row(name, *n, &methods, ps);
            total.iter_mut().zip(ps).for_each(|(t, s)| t.merge(s));
            pool += n;
        }
        row("POOLED", pool, &methods, &total);
    }
    println!("embedding time {:.2}s", embed_time.as_secs_f64());
}

fn main() {
    let root = corpus_dir().expect("set ORDO_CORPUS to a populated corpus dir");
    let model_dirs = std::env::var("M2V_MODEL").expect("set M2V_MODEL to model2vec model dirs");
    let model_dirs: Vec<&str> = model_dirs.split(',').collect();
    let models: Vec<StaticModel> = model_dirs
        .iter()
        .map(|d| StaticModel::from_pretrained(d, None, None, None).expect("load model"))
        .collect();
    let (model_dir, model) = (model_dirs[0], &models[0]);
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
    let repos: Vec<Repo> = parse_manifest()
        .into_iter()
        .filter(|r| all || want.contains(&r.name.as_str()))
        .filter(|r| {
            let fetched = root.join(&r.name).is_dir();
            if !fetched {
                eprintln!("{}: not fetched, skipped", r.name);
            }
            fetched
        })
        .collect();
    let names: Vec<String> = model_dirs.iter().map(|d| model_name(d)).collect();
    match std::env::var("M2V_MODE").as_deref() {
        Ok("intent") => {
            println!("model {model_dir} mode intent");
            return intent_mode(model, &root, &repos, dump.out.as_mut());
        }
        Ok("search") => return search_mode(&models, &names, &root, &repos, dump.out.as_mut()),
        Ok("similar") => return similar_mode(&models, &names, &root, &repos, dump.out.as_mut()),
        _ => {}
    }
    let mode = match std::env::var("M2V_MODE").as_deref() {
        Ok("cross") => Mode::Cross,
        Ok("mutual") => Mode::Mutual,
        Ok("samefile") => Mode::SameFile,
        Ok("file") => Mode::File,
        _ => Mode::All,
    };
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
    for repo in &repos {
        let dir = root.join(&repo.name);
        let tallies = sweep(
            model,
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
