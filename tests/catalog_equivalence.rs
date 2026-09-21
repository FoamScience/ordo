//! Catalog equivalence over the corpus.
//!
//! The construct catalog is moving off hand-written walkers and onto data
//! (tasks-9sj.33). A migrated construct must fire on exactly the rows it fired
//! on before — same name, same level, same file, same row — or the migration
//! quietly changed what reviewers are told, and no other test would notice:
//! the unit fixtures only assert that a construct fires *somewhere*.
//!
//! This records every `source = catalog` finding the corpus produces into
//! `corpus/catalog.txt` and fails on any difference. Re-record with
//! `UPDATE_CATALOG=1`, and only when the catalog is *meant* to change — the
//! gate writes the record and asserts nothing, so re-recording a run that was
//! not meant to be the truth (a timing experiment, a build with something
//! switched off) silently replaces the baseline with its output.
//!
//! Opt-in on `$ORDO_CORPUS`, like `tests/corpus.rs`.
use ordo::model::FindingSource;
use std::path::Path;

mod common;
use common::{commit_input, corpus_dir, git, parse_manifest, update_requested, Repo};

const RECORD: &str = "corpus/catalog.txt";

fn sweep(dir: &Path, repo: &Repo, out: &mut Vec<String>) {
    let shas = git(
        dir,
        &[
            "rev-list",
            "--max-count",
            &repo.max_commits.to_string(),
            &repo.rev,
        ],
    );
    for sha in shas.lines() {
        let Some((_, input)) = commit_input(dir, sha) else {
            continue;
        };
        for f in &ordo::run(input).files {
            for h in &f.hunks {
                for hit in h
                    .findings
                    .iter()
                    .filter(|x| x.source == FindingSource::Catalog)
                {
                    out.push(format!(
                        "{} {} {}:{} {} {}",
                        repo.name,
                        &sha[..8],
                        f.path,
                        h.new_range[0],
                        hit.name,
                        hit.level.as_str()
                    ));
                }
            }
        }
    }
}

#[test]
fn the_catalog_fires_on_exactly_the_rows_it_fired_on_before() {
    let Some(root) = corpus_dir() else {
        return;
    };
    let mut lines = vec![];
    for repo in parse_manifest() {
        let dir = root.join(&repo.name);
        if !dir.is_dir() {
            if update_requested("ORDO_CORPUS_REQUIRED") {
                panic!("corpus repo missing: {}", dir.display());
            }
            continue;
        }
        sweep(&dir, &repo, &mut lines);
    }
    // the sweep order is stable, but sorting makes a diff of the record read as
    // added/removed lines rather than a re-flow
    lines.sort();
    let got = lines.join("\n");
    if update_requested("UPDATE_CATALOG") {
        std::fs::write(RECORD, format!("{got}\n")).expect("write record");
        return;
    }
    let want = std::fs::read_to_string(RECORD)
        .unwrap_or_else(|_| panic!("missing {RECORD}; run UPDATE_CATALOG=1"));
    if got.trim() == want.trim() {
        return;
    }
    let got_lines: Vec<&str> = got.lines().collect();
    let want_lines: Vec<&str> = want.trim().lines().collect();
    let show = |v: &[&str]| v.iter().take(20).copied().collect::<Vec<_>>().join("\n");
    let gone: Vec<&str> = want_lines
        .iter()
        .filter(|l| !got_lines.contains(l))
        .copied()
        .collect();
    let new: Vec<&str> = got_lines
        .iter()
        .filter(|l| !want_lines.contains(l))
        .copied()
        .collect();
    panic!(
        "catalog output moved: {} finding(s) stopped firing, {} are new\n\
         stopped firing:\n{}\nnew:\n{}\n\
         If the catalog was meant to change, re-record with UPDATE_CATALOG=1",
        gone.len(),
        new.len(),
        show(&gone),
        show(&new),
    );
}
