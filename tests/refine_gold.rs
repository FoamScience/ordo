//! Intra-line refinement against hand-marked truth: `corpus/refine_gold.jsonl`.
//!
//! Each fixture is one corpus hunk whose removed and added lines a person
//! marked: `⟦…⟧` around what changed, `⟪…⟫` around what moved, the rest
//! unchanged. Text that only re-wraps across lines is unchanged, and the unit
//! is the word a reader sees — `0.6.0` → `0.7.0` marks the `6`, not the string.
//! Every non-whitespace char of the hunk is one of the three, and so is every
//! char the refiner reports: inside a span it is changed, on a line it left
//! unpaired it is changed, otherwise unchanged. A fixture's score is the count
//! of chars the two disagree on.
//!
//! Opt-in, like `tests/corpus.rs`: the refiner parses the whole file, which
//! lives in `$ORDO_CORPUS`. The ratchet is per fixture
//! (`corpus/refine_baseline.json`): no fixture may get worse. Re-record with
//! `UPDATE_REFINE_BASELINE=1`. Draw more with `scripts/refine-sample.py`.
use std::collections::BTreeMap;
use std::path::Path;

use ordo_diff::refine::{Refined, Refiner};
use serde::Deserialize;

mod common;
use common::{corpus_dir, git, update_requested};

const GOLD: &str = "corpus/refine_gold.jsonl";
const BASELINE: &str = "corpus/refine_baseline.json";

#[derive(Deserialize)]
struct Fixture {
    key: String,
    repo: String,
    sha: String,
    path: String,
    old_range: [usize; 2],
    new_range: [usize; 2],
    old: Vec<String>,
    new: Vec<String>,
    marked: Option<Marked>,
}

#[derive(Deserialize)]
struct Marked {
    old: Vec<String>,
    new: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Unchanged,
    Changed,
    Moved,
}

/// One marked line as its plain text and a class per char.
fn parse_marks(line: &str) -> (String, Vec<Class>) {
    let (mut text, mut classes, mut now) = (String::new(), vec![], Class::Unchanged);
    for c in line.chars() {
        match c {
            '⟦' => now = Class::Changed,
            '⟪' => now = Class::Moved,
            '⟧' | '⟫' => now = Class::Unchanged,
            _ => {
                text.push(c);
                classes.push(now);
            }
        }
    }
    (text, classes)
}

/// The refiner's verdict on one line as a class per char: moved where a
/// moved span says so, else changed inside a span or on an unpaired line.
fn predicted(
    line: &str,
    spans: Option<&Vec<(usize, usize)>>,
    moved: &[(usize, usize)],
) -> Vec<Class> {
    let inside = |v: &[(usize, usize)], k: usize| v.iter().any(|&(a, b)| k >= a && k < b);
    (0..line.chars().count())
        .map(|k| match spans {
            _ if inside(moved, k) => Class::Moved,
            None => Class::Changed,
            Some(v) if inside(v, k) => Class::Changed,
            Some(_) => Class::Unchanged,
        })
        .collect()
}

/// (mismatched, scored) non-whitespace chars over one side of a hunk.
fn score_side(
    lines: &[String],
    marked: &[String],
    refined: &[Option<Vec<(usize, usize)>>],
    moved: &[Vec<(usize, usize)>],
) -> (usize, usize) {
    let (mut miss, mut total) = (0, 0);
    for (k, (line, marks)) in lines.iter().zip(marked).enumerate() {
        let (text, truth) = parse_marks(marks);
        assert_eq!(&text, line, "marks do not spell the line");
        let pred = predicted(
            line,
            refined.get(k).and_then(Option::as_ref),
            moved.get(k).map_or(&[], Vec::as_slice),
        );
        for ((c, t), p) in line.chars().zip(truth).zip(pred) {
            if !c.is_whitespace() {
                total += 1;
                miss += (t != p) as usize;
            }
        }
    }
    (miss, total)
}

fn refine(dir: &Path, f: &Fixture) -> Refined {
    let show = |rev: String| {
        git(dir, &["show", &format!("{rev}:{}", f.path)])
            .lines()
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let (old, new) = (show(format!("{}^", f.sha)), show(f.sha.clone()));
    let slice = |all: &[String], [a, b]: [usize; 2]| all[a - 1..b].to_vec();
    assert_eq!(
        slice(&old, f.old_range),
        f.old,
        "{}: old side drifted",
        f.key
    );
    assert_eq!(
        slice(&new, f.new_range),
        f.new,
        "{}: new side drifted",
        f.key
    );
    Refiner::new(&f.path, &old, &new)
        .map(|r| r.refine(f.old_range, f.new_range))
        .unwrap_or_default()
}

#[test]
fn refinement_matches_the_marked_truth_and_does_not_regress() {
    let Some(root) = corpus_dir() else {
        assert!(
            !update_requested("ORDO_CORPUS_REQUIRED"),
            "ORDO_CORPUS_REQUIRED is set but $ORDO_CORPUS is unset or missing"
        );
        eprintln!("refine_gold: $ORDO_CORPUS unset or missing — skipping.");
        return;
    };
    let fixtures: Vec<Fixture> = std::fs::read_to_string(GOLD)
        .expect("read refine gold")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("parse refine gold row"))
        .collect();
    let mut scores: BTreeMap<String, usize> = BTreeMap::new();
    let (mut zero, mut within, mut labelled, mut miss_all, mut total_all) = (0, 0, 0, 0, 0);
    for f in &fixtures {
        let Some(m) = &f.marked else { continue };
        let r = refine(&root.join(&f.repo), f);
        // a side the refiner did not size to the hunk is a side it left whole
        let side = |v: &Vec<Option<Vec<(usize, usize)>>>, n: usize| {
            if v.len() == n {
                v.clone()
            } else {
                vec![None; n]
            }
        };
        let (mo, to) = score_side(
            &f.old,
            &m.old,
            &side(&r.removed, f.old.len()),
            &r.moved_removed,
        );
        let (mn, tn) = score_side(&f.new, &m.new, &side(&r.added, f.new.len()), &r.moved_added);
        let (miss, total) = (mo + mn, to + tn);
        labelled += 1;
        zero += (miss == 0) as usize;
        within += (miss * 100 <= total) as usize;
        miss_all += miss;
        total_all += total;
        eprintln!("{miss:5}/{total:<5} {}", f.key);
        scores.insert(f.key.clone(), miss);
    }
    eprintln!(
        "refine_gold: {labelled} fixtures, {zero} with zero mismatched chars, \
         {within} within 1%, {miss_all} of {total_all} chars mismatched"
    );
    if update_requested("UPDATE_REFINE_BASELINE") {
        let json = serde_json::to_string_pretty(&scores).unwrap();
        std::fs::write(BASELINE, format!("{json}\n")).unwrap();
        return;
    }
    let base: BTreeMap<String, usize> = std::fs::read_to_string(BASELINE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .expect("missing corpus/refine_baseline.json; run UPDATE_REFINE_BASELINE=1");
    let worse: Vec<String> = scores
        .iter()
        .filter_map(|(k, &now)| match base.get(k) {
            Some(&was) if now <= was => None,
            Some(&was) => Some(format!("{k}: {was} -> {now}")),
            None => Some(format!("{k}: not in the baseline")),
        })
        .collect();
    assert!(
        worse.is_empty(),
        "refinement got worse:\n{}",
        worse.join("\n")
    );
}
