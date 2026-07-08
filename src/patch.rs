//! Minimal unified/git diff parsing for `change.diff` input and `ordo review`.
//!
//! ponytail: a context-limited patch does NOT contain full new-file content, so
//! tree-sitter semantics can't be recovered for *modified* files — those yield
//! positional hunks (ranges from the `@@` headers). Added/deleted files ARE
//! fully reconstructable. For full semantics on modified files, use the JSON
//! old/new API (which runs `similar` over complete contents). Documented ceiling.
use crate::extract::RawHunk;

pub struct ParsedFile {
    pub new: Option<String>, // Some only when the new side is fully known (added file)
    pub hunks: Vec<RawHunk>,
}

/// Split a multi-file git/unified patch into `(path, single-file-diff)` chunks.
pub fn split_patch(patch: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = vec![];
    let mut cur: Option<String> = None;
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            if let Some(seg) = cur.take() {
                push_file(&mut files, seg);
            }
            cur = Some(String::new());
        }
        match cur.as_mut() {
            Some(seg) => {
                seg.push_str(line);
                seg.push('\n');
            }
            None => cur = Some(format!("{line}\n")),
        }
    }
    if let Some(seg) = cur.take() {
        push_file(&mut files, seg);
    }
    files
}

fn push_file(files: &mut Vec<(String, String)>, seg: String) {
    let path = seg
        .lines()
        .find_map(|l| l.strip_prefix("+++ ").map(clean_path))
        .filter(|p| p != "/dev/null")
        .or_else(|| {
            seg.lines()
                .find_map(|l| l.strip_prefix("--- ").map(clean_path))
        })
        .unwrap_or_default();
    if !path.is_empty() {
        files.push((path, seg));
    }
}

pub fn parse_file_diff(diff: &str) -> ParsedFile {
    let mut is_new_file = false;
    let mut hunks = vec![];
    let mut added: Vec<&str> = vec![];
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("+++ ") {
            // new path marker — nothing to record here
        } else if let Some(p) = line.strip_prefix("--- ") {
            if clean_path(p) == "/dev/null" {
                is_new_file = true;
            }
        } else if line.starts_with("@@") {
            in_hunk = true;
            if let Some(h) = parse_hunk_header(line) {
                hunks.push(h);
            }
        } else if in_hunk {
            if let Some(rest) = line.strip_prefix('+') {
                added.push(rest);
            }
        }
    }
    let new = if is_new_file {
        Some(added.join("\n") + "\n")
    } else {
        None
    };
    ParsedFile { new, hunks }
}

fn parse_hunk_header(line: &str) -> Option<RawHunk> {
    // @@ -os,ol +ns,nl @@ [section heading]
    let core = line.trim_start_matches('@');
    let mut parts = core.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (os, ol) = parse_range(old);
    let (ns, nl) = parse_range(new);
    let old_range = if ol > 0 {
        [os, os + ol - 1]
    } else {
        [os + 1, os]
    };
    let new_range = if nl > 0 {
        [ns, ns + nl - 1]
    } else {
        [ns + 1, ns]
    };
    let (new_r0, new_r1) = if nl > 0 {
        (Some(ns - 1), ns + nl - 2)
    } else {
        (None, 0)
    };
    Some(RawHunk {
        old_range,
        new_range,
        new_r0,
        new_r1,
    })
}

fn parse_range(s: &str) -> (usize, usize) {
    let mut it = s.split(',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (start, count)
}

fn clean_path(p: &str) -> String {
    let p = p.split('\t').next().unwrap_or(p).trim();
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p)
        .to_string()
}
