// ------------------------------------------------------- consumer repositories
//! Other repositories that import this one. A change to `script/run.py` whose
//! only caller lives in a sibling repository is invisible to the engine unless
//! that caller is handed over; this finds such files and passes them along as
//! consumers, so the contract checks can name the call that has to follow.

use crate::git::git;
use ordo::model::{Change, Consumer};
use std::path::{Path, PathBuf};

/// A monorepo next door must not turn one review into reading all of it.
const MAX_FILES: usize = 200;
const MAX_BYTES: u64 = 512 * 1024;
/// all consumers together; the engine holds each one twice
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

const SOURCE_GLOBS: &[&str] = &["*.py", "*.js", "*.mjs", "*.ts", "*.tsx"];

/// The files in `declared` repositories, and in sibling repositories whose
/// manifest points back at this one, that mention a module the change touches.
pub(super) fn gather(root: &Path, changes: &[Change], declared: &[PathBuf]) -> Vec<Consumer> {
    let needles = needles(changes);
    if needles.is_empty() {
        return vec![];
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root = root.as_path();
    let mut repos: Vec<PathBuf> = vec![];
    for r in declared.iter().cloned().chain(dependents(root)) {
        let r = r.canonicalize().unwrap_or(r);
        if !repos.contains(&r) && r != root {
            repos.push(r);
        }
    }
    let mut out = vec![];
    let mut total = 0;
    for repo in repos {
        let shown = shown_as(root, &repo);
        let dir = repo.to_string_lossy().into_owned();
        let mut args = vec!["-C", dir.as_str(), "ls-files", "--"];
        args.extend(SOURCE_GLOBS);
        for file in git(&args).lines() {
            if out.len() >= MAX_FILES || total >= MAX_TOTAL_BYTES {
                return out;
            }
            let path = repo.join(file);
            if path.metadata().is_ok_and(|m| m.len() > MAX_BYTES) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if needles.iter().any(|n| content.contains(n.as_str())) {
                total += content.len();
                out.push(Consumer {
                    path: format!("{shown}/{file}"),
                    content,
                });
            }
        }
    }
    out
}

/// What an importer of a changed file has to write: `script.run` for
/// `script/run.py` (`from script.run import main`), `script/run` for a js
/// module. One segment alone (`run`) would match half of any repository.
fn needles(changes: &[Change]) -> Vec<String> {
    let mut out = vec![];
    for c in changes {
        let Some((stem, ext)) = c.path.rsplit_once('.') else {
            continue;
        };
        let mut parts: Vec<&str> = stem.split('/').collect();
        if parts.last() == Some(&"__init__") {
            parts.pop();
        }
        let tail = &parts[parts.len().saturating_sub(2)..];
        let needle = match (ext, tail) {
            (_, []) => continue,
            ("py", [one]) => format!("import {one}"),
            ("py", _) => tail.join("."),
            ("js" | "mjs" | "ts" | "tsx", [one]) => format!("/{one}"),
            ("js" | "mjs" | "ts" | "tsx", _) => tail.join("/"),
            _ => continue,
        };
        if !out.contains(&needle) {
            out.push(needle);
        }
    }
    out
}

/// Sibling repositories whose manifest names this one by relative path:
/// `{ path = "../pump", editable = true }` in a pyproject, uv.lock's
/// `editable = "../pump"`, `-e ../pump` in a requirements file, `file:../pump`
/// in a package.json.
fn dependents(root: &Path) -> Vec<PathBuf> {
    let (Some(name), Some(parent)) = (root.file_name(), root.parent()) else {
        return vec![];
    };
    let needle = format!("../{}", name.to_string_lossy());
    let Ok(dirs) = std::fs::read_dir(parent) else {
        return vec![];
    };
    dirs.filter_map(Result::ok)
        .map(|d| d.path())
        .filter(|d| d.is_dir() && d != root)
        .filter(|d| {
            [
                "pyproject.toml",
                "uv.lock",
                "requirements.txt",
                "package.json",
            ]
            .iter()
            .filter_map(|m| std::fs::read_to_string(d.join(m)).ok())
            .any(|t| t.contains(&needle))
        })
        .collect()
}

/// `../pipeline` for a sibling, the full path otherwise
fn shown_as(root: &Path, repo: &Path) -> String {
    match (repo.parent(), root.parent(), repo.file_name()) {
        (Some(a), Some(b), Some(name)) if a == b => format!("../{}", name.to_string_lossy()),
        _ => repo.to_string_lossy().into_owned(),
    }
}
