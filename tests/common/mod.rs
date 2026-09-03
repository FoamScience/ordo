//! Corpus plumbing shared by `tests/corpus.rs` and `benches/corpus.rs`.
//!
//! Lives under `tests/common/` rather than `tests/` so cargo does not build it
//! as an integration-test target of its own; the bench pulls it in with an
//! explicit `#[path]`. The engine stays git-free — this is the consumer-side
//! git plumbing both harnesses need to turn a commit into engine input.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ordo::model::{Change, Input, Options};

/// Extensions the engine claims. Anything else is skipped: feeding it a `.png`
/// measures nothing about rationale quality.
pub fn is_supported(path: &str) -> bool {
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

pub struct Repo {
    pub name: String,
    pub lang: String,
    pub rev: String,
    pub max_commits: usize,
}

pub fn parse_manifest() -> Vec<Repo> {
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

pub fn git(dir: &Path, args: &[&str]) -> String {
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
pub fn commit_input(dir: &Path, sha: &str) -> Option<(String, Input)> {
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

pub fn corpus_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var("ORDO_CORPUS").ok()?);
    d.is_dir().then_some(d)
}
