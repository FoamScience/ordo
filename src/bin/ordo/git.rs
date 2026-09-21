// ------------------------------------------------------------------- git layer
use crate::read_progress;
use crate::Filter;
use crate::EMPTY_TREE;
use ordo::model::Change;
use ordo::model::Input;
use ordo::model::Options;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

// Empty string when git fails, so a missing blob reads as empty content. The
// status check matters: `rev-parse --verify -q` still prints on failure (a range
// echoes both endpoints), and taking that output would be read as a sha.
pub(super) fn git(args: &[&str]) -> String {
    run_cmd("git", args)
}

// Runs a binary, returning its stdout as a string or empty on any failure to
// launch or a nonzero exit — the shared body behind `git` and `but`.
//
// A failure still reads as empty output to the caller — no call site would
// branch differently on it — but it is no longer silent: the command and the
// first line of its stderr are recorded (see `COMMAND_FAILURES`) and shown in
// the empty-review message and in `:audit`.
pub(super) fn run_cmd(bin: &str, args: &[&str]) -> String {
    let Ok(out) = Command::new(bin).args(args).output() else {
        note_command_failure(bin, args, "could not be run");
        return String::new();
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        note_command_failure(bin, args, first);
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Commands that failed, so a review that came back empty can say why.
///
/// `run_cmd` returns an empty string on failure and always has: the load runs
/// on a worker thread with the TUI already holding the terminal, so it cannot
/// print, and threading a Result through twenty-odd call sites buys nothing
/// the caller would act on differently. What was missing is that the failure
/// left no trace at all, so a bad revision or an unreadable object arrived as
/// "nothing to review". Collected here and drained into the empty-review
/// message and `:audit`.
pub(super) static COMMAND_FAILURES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(super) fn note_command_failure(bin: &str, args: &[&str], why: &str) {
    // `but` is optional by design — its absence is the normal case on a repo
    // that is not GitButler-managed and says nothing about the review
    if bin == "but" {
        return;
    }
    let cmd = format!("{bin} {}", args.join(" "));
    let line = if why.is_empty() {
        cmd
    } else {
        format!("{cmd}: {why}")
    };
    if let Ok(mut v) = COMMAND_FAILURES.lock() {
        if !v.contains(&line) {
            v.push(line);
        }
    }
}

/// Every distinct command failure so far, in the order they happened.
pub(super) fn command_failures() -> Vec<String> {
    COMMAND_FAILURES
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}

// Same, with the arg list fed on stdin — for `check-attr --stdin`, where the
// path list can outgrow what a command line takes.
pub(super) fn git_stdin(args: &[&str], input: &str) -> String {
    use std::io::Write;
    let mut child = match Command::new("git")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    if let Some(mut w) = child.stdin.take() {
        let _ = w.write_all(input.as_bytes());
    }
    child
        .wait_with_output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// Fetches many `<rev>:<path>` blobs in one `git cat-file --batch` process
// instead of one `git show` per spec — the same fork/exec is paid once for the
// whole file list instead of once per file per side. Missing objects (a path
// added or deleted on one side) come back absent from the map, same as `git`
// returning empty on failure; look them up with `.unwrap_or_default()` to
// match. Lossy UTF-8, same as `run_cmd`, so binary-ish content behaves the
// same as the old per-file `git show` path. Stdin is written and dropped
// (closing it) before stdout is read, to avoid deadlocking on a full pipe
// buffer with a large spec list.
pub(super) fn git_cat_file_batch(specs: &[String]) -> HashMap<String, String> {
    use std::io::{Read, Write};
    let mut result = HashMap::with_capacity(specs.len());
    if specs.is_empty() {
        return result;
    }
    let mut child = match Command::new("git")
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return result,
    };
    {
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => return result,
        };
        for spec in specs {
            if writeln!(stdin, "{spec}").is_err() {
                break;
            }
        }
        // `stdin` drops here, closing the pipe so the child's stdout can flush
        // fully instead of the two of us deadlocking on a full pipe buffer.
    }
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => return result,
    };
    let mut buf = Vec::new();
    if stdout.read_to_end(&mut buf).is_err() {
        let _ = child.wait();
        return result;
    }
    let _ = child.wait();

    let mut i = 0;
    for spec in specs {
        let Some(nl) = buf[i..].iter().position(|&b| b == b'\n').map(|p| i + p) else {
            break;
        };
        let header = String::from_utf8_lossy(&buf[i..nl]).into_owned();
        i = nl + 1;
        if header.ends_with("missing") {
            continue;
        }
        // header: "<oid> <type> <size>" — take size by splitting from the
        // right so an oid or type never gets mistaken for it.
        let Some(size) = header
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
        else {
            break;
        };
        let Some(content) = buf.get(i..i + size) else {
            break;
        };
        result.insert(spec.clone(), String::from_utf8_lossy(content).into_owned());
        i += size + 1; // skip the trailing newline after the content block
    }
    result
}

// GitButler CLI: empty string if `but` isn't installed or the call fails, so the
// plain-git path is unaffected on non-GitButler repos.
pub(super) fn but(args: &[&str]) -> String {
    run_cmd("but", args)
}

// Paths the repo's own `.gitattributes` marks as generated: `linguist-generated`
// (the marker GitHub collapses a file by) or an explicit `-diff`. One batched
// call — `-z` makes both the path list and the output NUL-separated, so paths
// with colons or newlines survive the round trip.
pub(super) fn declared_generated(paths: &[String]) -> std::collections::HashSet<String> {
    let mut input = paths.join("\0");
    input.push('\0');
    let out = git_stdin(
        &["check-attr", "-z", "--stdin", "linguist-generated", "diff"],
        &input,
    );
    let mut fields = out.split('\0');
    let mut generated = std::collections::HashSet::new();
    while let (Some(path), Some(attr), Some(value)) = (fields.next(), fields.next(), fields.next())
    {
        if path.is_empty() {
            break;
        }
        let declared = match attr {
            // a bare `linguist-generated` reads as "set", `=true` as "true"
            "linguist-generated" => value == "set" || value == "true",
            "diff" => value == "unset",
            _ => false,
        };
        if declared {
            generated.insert(path.to_string());
        }
    }
    generated
}

// -------------------------------------------------------------- gitbutler layer

/// `but --json status` — the whole workspace in one call: the uncommitted
/// changes, each stack's branches and their commits, and the merge base. None
/// when `but` is missing or the repo isn't GitButler-managed.
pub(super) fn workspace() -> Option<serde_json::Value> {
    serde_json::from_str(&but(&["--json", "status"])).ok()
}

pub(super) fn field<'a>(v: &'a serde_json::Value, key: &str) -> &'a str {
    v.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

pub(super) fn branches(ws: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    ws.get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("branches")?.as_array())
        .flatten()
}

/// A workspace branch — by the CLI ID `but status` prints, or by its name —
/// reviews as the range spanning its own commits, which is what reviewing one
/// branch of a stack means. Takes precedence over git's reading of the same
/// name, where a branch is only its tip commit.
pub(super) fn branch_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let b = branches(ws).find(|b| field(b, "cliId") == arg || field(b, "name") == arg)?;
    let commits = b.get("commits")?.as_array()?;
    pub(super) fn commit_id(c: &serde_json::Value) -> Option<&str> {
        Some(field(c, "commitId")).filter(|s| !s.is_empty())
    }
    // `but` lists a branch newest-first, so the range runs from below the last
    // commit up to the first
    let tip = commit_id(commits.first()?)?.to_string();
    let oldest = commit_id(commits.last()?)?;
    let base = git(&["rev-parse", "--verify", "-q", &format!("{oldest}^")]);
    let base = base.trim();
    let base = if base.is_empty() { EMPTY_TREE } else { base };
    Some(Target::Range(base.to_string(), tip))
}

/// A workspace commit by its CLI ID, or by a prefix of its change ID or commit
/// ID — the same identifiers `but status` accepts.
pub(super) fn commit_target(ws: &serde_json::Value, arg: &str) -> Option<Target> {
    let c = branches(ws)
        .filter_map(|b| b.get("commits")?.as_array())
        .flatten()
        .find(|c| {
            field(c, "cliId") == arg
                || field(c, "changeId").starts_with(arg)
                || field(c, "commitId").starts_with(arg)
        })?;
    Some(Target::Commit(field(c, "commitId").to_string()))
}

pub(super) enum Target {
    Commit(String),
    /// A `base..tip` / `base...tip` range, already resolved to two commit shas.
    Range(String, String),
    Uncommitted,
    /// `base..zz` / `base...zz` — a resolved base commit versus the worktree
    /// (not HEAD). `...` has already been reduced to the merge base of `base`
    /// and HEAD, same as `Range`'s symmetric case.
    WorktreeRange(String),
}

// `base..tip` is a plain two-endpoint diff; `base...tip` diffs from the merge
// base, which is what you want for a branch that trails its target. An omitted
// side means HEAD, as in git. `tip == "zz"` diffs against the worktree instead
// of a commit — see `WorktreeRange`; `zz` on the left is meaningless (nothing
// precedes the uncommitted area) and rejected.
pub(super) fn resolve_range(arg: &str) -> Option<Target> {
    let (base, tip, symmetric) = match arg.split_once("...") {
        Some((b, t)) => (b, t, true),
        None => {
            let (b, t) = arg.split_once("..")?;
            (b, t, false)
        }
    };
    // a second `..` is not a range we understand (e.g. `a..b..c`)
    if tip.contains("..") {
        return None;
    }
    if base == "zz" {
        return None;
    }
    let rev = |s: &str| {
        let s = if s.is_empty() { "HEAD" } else { s };
        let sha = git(&["rev-parse", "--verify", "-q", &format!("{s}^{{commit}}")]);
        let sha = sha.trim().to_string();
        (!sha.is_empty()).then_some(sha)
    };
    if tip == "zz" {
        let base = rev(base)?;
        if !symmetric {
            return Some(Target::WorktreeRange(base));
        }
        let merge_base = git(&["merge-base", &base, "HEAD"]);
        let merge_base = merge_base.trim();
        return (!merge_base.is_empty()).then(|| Target::WorktreeRange(merge_base.to_string()));
    }
    let (base, tip) = (rev(base)?, rev(tip)?);
    if !symmetric {
        return Some(Target::Range(base, tip));
    }
    let merge_base = git(&["merge-base", &base, &tip]);
    let merge_base = merge_base.trim();
    (!merge_base.is_empty()).then(|| Target::Range(merge_base.to_string(), tip))
}

// Resolve an arg to a commit, a range, or the uncommitted area. `zz` is
// GitButler's constant for the uncommitted area and never a commit. Ranges come
// next, then a GitButler workspace *branch* — ahead of git so a branch label
// reviews the branch's commits rather than just its tip. Plain git commit-ishes
// (sha, HEAD~2, tag) follow, and finally a GitButler commit CLI ID. The
// GitButler steps are skipped entirely on a repo `but` doesn't manage.
pub(super) fn resolve(arg: &str) -> Option<Target> {
    if arg == "zz" {
        return Some(Target::Uncommitted);
    }
    // `..` is illegal in a refname, so it can only mean a range here
    if arg.contains("..") {
        return resolve_range(arg);
    }
    let ws = workspace();
    if let Some(t) = ws.as_ref().and_then(|ws| branch_target(ws, arg)) {
        return Some(t);
    }
    let sha = git(&["rev-parse", "--verify", "-q", &format!("{arg}^{{commit}}")]);
    let sha = sha.trim();
    if !sha.is_empty() {
        return Some(Target::Commit(sha.to_string()));
    }
    ws.as_ref().and_then(|ws| commit_target(ws, arg))
}

// The single commit the hover popup's history section is relative to: a
// range reviews as its tip, and `zz` (the uncommitted area) has no commit of
// its own, so HEAD stands in for it. None only if HEAD itself can't resolve
// (an empty repo) — history is then just unavailable.
pub(super) fn review_commit_sha(target: &Target) -> Option<String> {
    match target {
        Target::Commit(sha) => Some(sha.clone()),
        Target::Range(_, tip) => Some(tip.clone()),
        Target::Uncommitted | Target::WorktreeRange(_) => {
            let sha = git(&["rev-parse", "--verify", "-q", "HEAD"]);
            let sha = sha.trim();
            (!sha.is_empty()).then(|| sha.to_string())
        }
    }
}

pub(super) fn gather(rev: &str, filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{rev}^")]);
    let parent = parent.trim();
    let parent = if parent.is_empty() {
        EMPTY_TREE
    } else {
        parent
    };
    gather_range(parent, rev, filter, progress)
}

// Both sides' full blobs (empty when a file is added or deleted), so hunks get
// full semantics instead of degrading to a context-limited patch.
pub(super) fn gather_range(
    base: &str,
    tip: &str,
    filter: &Filter,
    progress: &dyn Fn(String),
) -> Input {
    let names = git(&["diff", "--name-only", base, tip]);
    let paths: Vec<String> = names
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths
        .iter()
        .flat_map(|p| [format!("{base}:{p}"), format!("{tip}:{p}")])
        .collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let old = blobs.remove(&format!("{base}:{path}")).unwrap_or_default();
            let new = blobs.remove(&format!("{tip}:{path}")).unwrap_or_default();
            Change {
                old: Some(old),
                new: Some(new),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// The uncommitted area (`zz`): GitButler owns what counts as uncommitted (it
// includes untracked adds git-diff wouldn't show), so take the file list from
// the workspace status — which lists it per file, unlike `but diff`'s per-hunk
// entries. Changes assigned to a stack are picked up the same way. Full old/new
// content — old from HEAD, new from the worktree — so hunks get full semantics
// rather than degrading to a context-limited patch.
// Every path GitButler's workspace status lists as changed: `stacks[].assignedChanges`
// plus `uncommittedChanges`, each entry's `filePath`. Shared by `gather_uncommitted`
// and `gather_worktree_range`'s untracked-file branch — neither sorts nor dedups
// here, that's each caller's own business.
pub(super) fn workspace_change_paths(ws: &serde_json::Value) -> Vec<String> {
    let assigned = ws
        .get("stacks")
        .and_then(|s| s.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("assignedChanges")?.as_array())
        .flatten();
    ws.get("uncommittedChanges")
        .and_then(|c| c.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default()
        .iter()
        .chain(assigned)
        .map(|c| field(c, "filePath").to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

pub(super) fn gather_uncommitted(filter: &Filter, progress: &dyn Fn(String)) -> Input {
    let ws = workspace().unwrap_or(serde_json::Value::Null);
    let mut paths = workspace_change_paths(&ws);
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths.iter().map(|p| format!("HEAD:{p}")).collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let old = blobs.remove(&format!("HEAD:{path}")).unwrap_or_default();
            Change {
                old: Some(old),
                new: Some(std::fs::read_to_string(&path).unwrap_or_default()),
                diff: None,
                path,
            }
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}

// `base..zz` / `base...zz`: everything done on a branch including what's not
// yet committed — `base`'s tree versus the worktree. `git diff --name-only
// <base>` (one revision, no second) already compares a tree to the working
// directory, but it never lists untracked files, so the changed-file set is
// that plus untracked paths — from `but --json status`, same source
// `gather_uncommitted` uses, on a GitButler-managed repo, else `git ls-files
// --others --exclude-standard`. `old` comes from `base`'s tree (empty when
// the path didn't exist there); `new` is read straight off disk. A path
// absent from the worktree (deleted since `base`) gets an empty `new`, not a
// read error; a path that exists but can't be read as UTF-8 is dropped
// rather than folded into an empty string, which would misreport it as a
// deletion.
pub(super) fn gather_worktree_range(
    base: &str,
    filter: &Filter,
    progress: &dyn Fn(String),
) -> Input {
    let mut paths: Vec<String> = git(&["diff", "--name-only", base])
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let untracked: Vec<String> = match workspace() {
        Some(ws) => workspace_change_paths(&ws),
        None => git(&["ls-files", "--others", "--exclude-standard"])
            .split('\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect(),
    };
    paths.extend(untracked);
    paths.sort();
    paths.dedup();
    let paths = filter.apply(paths);
    let total = paths.len();
    let specs: Vec<String> = paths.iter().map(|p| format!("{base}:{p}")).collect();
    let mut blobs = git_cat_file_batch(&specs);
    let changes = paths
        .into_iter()
        .enumerate()
        .filter_map(|(i, path)| {
            progress(read_progress(&path, i, total));
            let new = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(_) => {
                    filter.note_unreadable();
                    return None;
                }
            };
            let old = blobs.remove(&format!("{base}:{path}")).unwrap_or_default();
            Some(Change {
                old: Some(old),
                new: Some(new),
                diff: None,
                path,
            })
        })
        .collect();
    Input {
        changes,
        options: Options::default(),
    }
}
