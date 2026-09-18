//! Minimal unified/git diff parsing for `change.diff` input and `ordo review`.
//!
//! ponytail: a context-limited patch does NOT contain full new-file content, so
//! tree-sitter semantics can't be recovered for *modified* files — those yield
//! positional hunks (ranges from the `@@` headers). Added/deleted files ARE
//! fully reconstructable. For full semantics on modified files, use the JSON
//! old/new API (which runs `similar` over complete contents). Documented ceiling.
use crate::extract::RawHunk;

pub struct ParsedFile {
    // old/new are Some only when the full content is known: an added file, or a
    // caller-asserted full-context diff. Otherwise the change is positional.
    pub old: Option<String>,
    pub new: Option<String>,
    pub hunks: Vec<RawHunk>,
}

/// Apply a unified diff to `old`, returning full new content, or None on any
/// parse/apply failure (caller then degrades to positional ordering). Git-free.
pub fn apply(old: &str, diff: &str) -> Option<String> {
    let text = with_headers(diff);
    let patch = diffy::Patch::from_str(&text).ok()?;
    diffy::apply(old, &patch).ok()
}

// diffy needs `---`/`+++` file headers; a bare-hunk `diff` gets synthetic ones.
fn with_headers(diff: &str) -> String {
    if diff.lines().any(|l| l.starts_with("--- ")) {
        diff.to_string()
    } else {
        format!("--- a\n+++ b\n{diff}")
    }
}

/// Split a multi-file git/unified patch into `(path, single-file-diff)` chunks.
///
/// **Ceiling:** files are split on `diff --git` lines. A plain `diff -u` over
/// several files carries no such line and comes back as one chunk named after
/// its first `+++`; `git diff` (what `ordo review` is written for) always emits
/// them.
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

/// Parse a single-file unified diff. Reconstructs full `old`/`new` content when
/// safe: an added file (all `+` lines), or — only when `full_context` is set by
/// the caller — a single whole-file hunk (see L2 in docs/diff-input-design.md).
/// Otherwise old/new are None and the change is ordered positionally.
pub fn parse_file_diff(diff: &str, full_context: bool) -> ParsedFile {
    let mut is_new_file = false;
    let mut hunks = vec![];
    let mut old_side: Vec<&str> = vec![]; // context + removed
    let mut new_side: Vec<&str> = vec![]; // context + added
    let mut in_hunk = false;
    for line in diff.lines() {
        // Only *outside* a hunk body is `--- `/`+++ ` a file header. Inside
        // one it is an ordinary changed line whose own text begins with `--`
        // or `++`: a lua or sql comment being removed (`-` + `-- note`), TOML
        // front matter being added to a markdown file (`+` + `+++`). Treating
        // those as headers dropped them from the reconstructed side, and the
        // hunk then reported an edit as a pure addition.
        if !in_hunk && line.starts_with("+++ ") {
            // new path marker — nothing to record here
        } else if let Some(p) = (!in_hunk).then(|| line.strip_prefix("--- ")).flatten() {
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
                new_side.push(rest);
            } else if let Some(rest) = line.strip_prefix('-') {
                old_side.push(rest);
            } else if let Some(rest) = line.strip_prefix(' ') {
                old_side.push(rest);
                new_side.push(rest);
            }
        }
    }
    let (old, new) = if is_new_file {
        (Some(String::new()), Some(join_nl(&new_side)))
    } else if full_context && hunks.len() == 1 && hunks[0].old_range[0] == 1 {
        // caller-asserted complete patch + structurally whole-file → reconstruct
        (Some(join_nl(&old_side)), Some(join_nl(&new_side)))
    } else {
        (None, None)
    };
    ParsedFile { old, new, hunks }
}

fn join_nl(lines: &[&str]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    }
}

fn parse_hunk_header(line: &str) -> Option<RawHunk> {
    // @@ -os,ol +ns,nl @@ [section heading]
    let core = line.trim_start_matches('@');
    let mut parts = core.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (os, ol) = parse_range(old)?;
    let (ns, nl) = parse_range(new)?;
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

/// One side of an `@@` header: `start[,count]`, both 1-based line numbers, with
/// `count` defaulting to 1. `None` when it does not parse, or when a non-empty
/// side claims to start at line 0 — the arithmetic below reads `start - 1`, and
/// a header this malformed is not one to guess at.
fn parse_range(s: &str) -> Option<(usize, usize)> {
    let mut it = s.split(',');
    let start: usize = it.next()?.parse().ok()?;
    let count: usize = match it.next() {
        Some(v) => v.parse().ok()?,
        None => 1,
    };
    (start > 0 || count == 0).then_some((start, count))
}

fn clean_path(p: &str) -> String {
    let p = p.split('\t').next().unwrap_or(p).trim();
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::apply;

    use super::{parse_file_diff, parse_hunk_header};

    #[test]
    fn a_malformed_hunk_header_is_skipped_not_fatal() {
        // `ns - 1` on a header whose start did not parse used to panic
        assert!(parse_hunk_header("@@ -1,3 +x,3 @@").is_none());
        assert!(parse_hunk_header("@@ -1,3 +0,3 @@").is_none());
        // an empty new side legitimately starts at 0
        assert!(parse_hunk_header("@@ -1,3 +0,0 @@").is_some());
    }

    #[test]
    fn a_changed_line_starting_with_dashes_is_not_a_file_header() {
        // removing a lua comment writes `-` + `-- note`; that used to be read
        // as a `--- ` file header and dropped from the reconstructed old side
        let diff = "--- a/f.lua\n+++ b/f.lua\n@@ -1,2 +1,2 @@\n local x = 1\n--- old\n+-- new\n";
        let pf = parse_file_diff(diff, true);
        assert_eq!(pf.old.as_deref(), Some("local x = 1\n-- old\n"));
        assert_eq!(pf.new.as_deref(), Some("local x = 1\n-- new\n"));
    }

    const OLD: &str = "a\nb\nc\n";
    const NEW: &str = "a\nB\nc\n";

    #[test]
    fn apply_with_headers() {
        let diff = "--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        assert_eq!(apply(OLD, diff).as_deref(), Some(NEW));
    }

    #[test]
    fn apply_headerless() {
        let diff = "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        assert_eq!(apply(OLD, diff).as_deref(), Some(NEW));
    }

    #[test]
    fn apply_bad_context_is_none() {
        // context lines don't match OLD → apply must fail, not misapply
        let diff = "@@ -1,3 +1,3 @@\n x\n-y\n+Y\n z\n";
        assert_eq!(apply(OLD, diff), None);
    }
}
