// ------------------------------------------------------------ cross-commit history
use crate::git::git;
use crate::App;
use crate::Item;
use ordo::model::Change;
use ordo::model::HunkOut;
use ordo::model::Input;
use ordo::model::Options;
use ordo::model::Symbol;
use std::collections::HashMap;

// bound on how many candidate commits per direction (earlier/later) get run
// through the engine — a long-lived file's full history would otherwise stall
// the UI on a single `K` press
pub(super) const HISTORY_WINDOW: usize = 10;

pub(super) fn commit_list(args: &[&str]) -> Vec<String> {
    git(args)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// `scope.name`, or bare `name` at top level — matches the qualified form a
/// hunk's own `enclosing` field carries, so a body-edit match (found via
/// `enclosing`) and a defines-match (found via symbol identity) read the
/// same string in `label_for`.
pub(super) fn qualified_name(sym: &Symbol) -> String {
    match &sym.scope {
        Some(s) => format!("{s}.{}", sym.name),
        None => sym.name.clone(),
    }
}

/// The project's identity rule, verbatim: "tree sitter type + scope for the
/// symbol must match, otherwise it's a different symbol". Name alone is never
/// enough — a method `run` on class `A` and a module-level `run` don't match,
/// nor do two same-named, same-scope defs of different tree-sitter kinds.
pub(super) fn symbol_eq(a: &Symbol, b: &Symbol) -> bool {
    a.name == b.name && a.kind == b.kind && a.scope == b.scope
}

/// Bound `git rev-list <rev> -- <file>`'s output (`rev` itself first, if it
/// touched the file, then its nearest ancestor, then the next, ...) to the
/// nearest `window` ancestors, oldest-of-the-window first — so the entry
/// nearest `current` prints right above the CURRENT line.
pub(super) fn bound_earlier(mut shas: Vec<String>, current: &str, window: usize) -> Vec<String> {
    if shas.first().map(String::as_str) == Some(current) {
        shas.remove(0);
    }
    shas.truncate(window);
    shas.reverse();
    shas
}

/// Bound `git rev-list <rev>..HEAD -- <file>`'s output (HEAD first, walking
/// back to the nearest descendant of `rev` last) to the nearest `window`
/// descendants, nearest-to-`rev` first — so the entry nearest `rev` prints
/// right below the CURRENT line.
pub(super) fn bound_later(mut shas: Vec<String>, window: usize) -> Vec<String> {
    shas.reverse();
    shas.truncate(window);
    shas
}

/// The engine's identity (name + tree-sitter kind + qualified scope) for the
/// definition named `name` (tree-sitter kind `kind`) at 0-based `row` in
/// `content`. Runs the engine as though the whole file were freshly added, so
/// every definition in it — not just ones inside a hunk that happens to be
/// selected — shows up in some hunk's `symbols`, keyed to its own row.
pub(super) fn symbol_identity(
    path: &str,
    content: &str,
    name: &str,
    kind: &str,
    row: usize,
) -> Option<Symbol> {
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(String::new()),
            new: Some(content.to_string()),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let line = row + 1;
    file.hunks
        .iter()
        .find(|h| h.new_range[0] <= line && line <= h.new_range[1])
        .and_then(|h| h.symbols.iter().find(|s| s.name == name && s.kind == kind))
        // a hunk-boundary mismatch in the synthetic whole-file diff shouldn't
        // lose the def entirely — any hunk carrying the right name+kind is it
        .or_else(|| {
            file.hunks
                .iter()
                .flat_map(|h| h.symbols.iter())
                .find(|s| s.name == name && s.kind == kind)
        })
        .cloned()
}

/// Pull the fragment of a (possibly multi-symbol) rationale that names this
/// symbol — "adds helper; changes signature of run" reads as just "changes
/// signature of run" for `run`'s own history line. Falls back to the whole
/// rationale, which covers the enclosing-only body-edit case ("edits
/// A.run") where the fragment already names nothing else.
pub(super) fn label_for(h: &HunkOut, target: &Symbol) -> String {
    let qualified = qualified_name(target);
    h.rationale
        .split("; ")
        .find(|frag| frag.contains(target.name.as_str()) || frag.contains(qualified.as_str()))
        .unwrap_or(&h.rationale)
        .to_string()
}

/// What commit `sha` did to `target`, in ordo's own rationale wording — or
/// None when this commit touched `path` but never reached `target` itself (a
/// path-based window can't help but include unrelated changes to the file;
/// see `earlier`/`later` in `compute_history`).
pub(super) fn classify_commit(sha: &str, path: &str, target: &Symbol) -> Option<String> {
    let parent = git(&["rev-parse", "--verify", "-q", &format!("{sha}^")]);
    let parent = parent.trim();
    let old = if parent.is_empty() {
        String::new()
    } else {
        git(&["show", &format!("{parent}:{path}")])
    };
    let new = git(&["show", &format!("{sha}:{path}")]);
    let out = ordo::run(Input {
        changes: vec![Change {
            path: path.to_string(),
            old: Some(old),
            new: Some(new),
            diff: None,
        }],
        options: Options::default(),
    });
    let file = out.files.iter().find(|f| f.path == path)?;
    let qualified = qualified_name(target);
    file.hunks.iter().find_map(|h| {
        let defines_it = h.symbols.iter().any(|s| symbol_eq(s, target));
        let body_edit = h.enclosing.as_deref() == Some(qualified.as_str());
        (defines_it || body_edit).then(|| label_for(h, target))
    })
}

// Direction semantics (decided): `earlier` = ancestors of the reviewed rev,
// `later` = descendants — topological, not date-based, so this is correct
// when reviewing an old or mid-stack commit. No `--follow`: `git rev-list`
// doesn't support it at all (only `git log` does — passing it errors), so a
// rename before the window's oldest commit silently ends this symbol's
// history there. This engine call is single-file, one commit at a time, so it
// sees only same-file edits — a call site added in another file never shows.
/// How often the lines this hunk changed have changed before, and who touched
/// them last.
///
/// `HIGH_CHURN` in the engine counts hunks within *this* changeset. This is the
/// other axis: a function edited nine times in three months reads differently
/// from one untouched for two years, and a reviewer prioritises on that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Churn {
    /// commits touching these lines before the one under review, capped at
    /// `CHURN_WINDOW` — `Some(n)` where n == CHURN_WINDOW means "at least"
    pub(super) commits: usize,
    /// true when the count hit the window and the real number is higher
    pub(super) capped: bool,
    /// author and date of the most recent commit before the reviewed one
    pub(super) last: Option<(String, String)>,
}

/// How far back `hunk_churn` counts. `git log -L` walks the whole history
/// whichever way it is bounded (`-n` does not make it cheaper — measured), so
/// this caps what is *reported*, not what is walked: past a handful of edits
/// "at least N" is the same signal to a reviewer as an exact count.
pub(super) const CHURN_WINDOW: usize = 20;

/// Churn for one hunk, by walking the history of its new-side lines.
///
/// Deliberately not computed during load. Measured on a 13k-line file with 211
/// commits: 0.19s per hunk, and `-n` does not bound it because git still
/// follows the range through every commit. A 296-hunk review would spend most
/// of a minute on it, so this runs only when asked for (`H`), and caches.
///
/// The reviewed commit is itself the first entry `-L` reports; it is the change
/// being read, not history, so it is dropped.
/// Which side's line numbers the churn query follows, and whether the reviewed
/// commit leads the log.
///
/// Reviewing a commit, the new side *is* the file at that commit, and that
/// commit leads its own log — it is the change being read, not history, so it
/// is dropped. Reviewing the uncommitted area the new side is the worktree,
/// which is in no commit at all: the old side is HEAD, so that is what gets
/// followed, and HEAD is real history that must be kept.
pub(super) struct ChurnQuery {
    pub(super) rows: [usize; 2],
    pub(super) drop_leading_rev: bool,
}

pub(super) fn churn_query(it: &Item, uncommitted: bool) -> ChurnQuery {
    if uncommitted {
        ChurnQuery {
            rows: it.old_range,
            drop_leading_rev: false,
        }
    } else {
        ChurnQuery {
            rows: it.new_range,
            drop_leading_rev: true,
        }
    }
}

/// How far back per-file churn looks. Recent edits are the signal — a file
/// rewritten twice last month reads differently from one rewritten twice in
/// 2019, and an all-time count flattens that away.
pub(super) const FILE_CHURN_WINDOW: &str = "6.months";

/// How many files the eager churn pass will spend time on.
///
/// One `git rev-list` per file, measured at ~9ms. That is nothing for a normal
/// review and not nothing for a sweeping one: this repository's own history
/// holds a 78-file commit, and a mass rename runs to hundreds. Past this many
/// files the signal is dropped rather than paid for — a reviewer facing 200
/// files is not triaging by churn, and `H` still answers for any hunk they
/// stop on.
pub(super) const FILE_CHURN_MAX_FILES: usize = 100;

/// Commits touching each file in the window.
///
/// Per *file*, not per hunk, and that is the whole reason it can be eager:
/// ~9ms a file against `git log -L`'s ~200ms for one hunk. `rev-list --count`
/// rather than counting the lines of a log: git already knows the number, and
/// a hot file's log is a page of output to allocate and throw away.
pub(super) fn file_churn(
    review_sha: &str,
    paths: &[String],
    progress: &dyn Fn(String),
) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    if paths.len() > FILE_CHURN_MAX_FILES {
        return out;
    }
    for (i, path) in paths.iter().enumerate() {
        progress(format!("history… {}/{}", i + 1, paths.len()));
        let n = git(&[
            "rev-list",
            "--count",
            &format!("--since={FILE_CHURN_WINDOW}"),
            review_sha,
            "--",
            path,
        ]);
        if let Ok(n) = n.trim().parse::<usize>() {
            out.insert(path.clone(), n);
        }
    }
    out
}

pub(super) fn hunk_churn(review_sha: &str, path: &str, q: &ChurnQuery) -> Churn {
    let [r0, r1] = q.rows;
    // a pure insertion has an empty old-side range (`[n, n - 1]`); there are no
    // prior lines to follow, and git rejects an inverted range
    if r0 == 0 || r0 > r1 {
        return Churn {
            commits: 0,
            capped: false,
            last: None,
        };
    }
    let out = git(&[
        "log",
        "-L",
        &format!("{r0},{r1}:{path}"),
        "--no-patch",
        "--format=%H%x09%an%x09%ad",
        "--date=short",
        review_sha,
    ]);
    churn_from_log(&out, review_sha, q.drop_leading_rev)
}

/// Parse `git log -L`'s rows into a `Churn`. Split out from the shelling so the
/// part with the decisions in it — dropping the reviewed commit, and telling
/// "exactly the window" from "more than it" — is testable without a repo.
pub(super) fn churn_from_log(out: &str, review_sha: &str, drop_leading_rev: bool) -> Churn {
    let mut rows = out.lines().filter_map(|l| {
        let mut f = l.split('\t');
        Some((f.next()?, f.next()?.to_string(), f.next()?.to_string()))
    });
    let first = rows.next();
    // `%H` is a full sha; `review_sha` may be the abbreviation the user typed
    let leads = matches!(&first, Some((sha, _, _)) if sha.starts_with(review_sha));
    let keep: Box<dyn Iterator<Item = (&str, String, String)>> =
        match (first, drop_leading_rev && leads) {
            (_, true) | (None, _) => Box::new(rows),
            (Some(f), false) => Box::new(std::iter::once(f).chain(rows)),
        };
    // one past the window, so "exactly CHURN_WINDOW" is not reported as "at
    // least CHURN_WINDOW"
    let taken: Vec<(&str, String, String)> = keep.take(CHURN_WINDOW + 1).collect();
    let capped = taken.len() > CHURN_WINDOW;
    Churn {
        commits: taken.len().min(CHURN_WINDOW),
        capped,
        last: taken
            .first()
            .map(|(_, author, date)| (author.clone(), date.clone())),
    }
}

pub(super) fn compute_history(review_sha: &str, path: &str, target: &Symbol) -> Vec<String> {
    let earlier_all = commit_list(&["rev-list", review_sha, "--", path]);
    let earlier = bound_earlier(earlier_all, review_sha, HISTORY_WINDOW);
    let later_all = commit_list(&["rev-list", &format!("{review_sha}..HEAD"), "--", path]);
    let later = bound_later(later_all, HISTORY_WINDOW);

    let mut out = vec![];
    history_lines_for(&earlier, "earlier", path, target, &mut out);
    out.push(format!("{:<7}  {}", "CURRENT", short_sha(review_sha)));
    history_lines_for(&later, "later", path, target, &mut out);
    out
}

// One direction's rows in `compute_history`'s output: `tag` labels only the
// first commit that actually classifies, matching the reading-order convention
// where a repeated column reads as blank rather than restating itself.
pub(super) fn history_lines_for(
    shas: &[String],
    tag: &str,
    path: &str,
    target: &Symbol,
    out: &mut Vec<String>,
) {
    let mut first = true;
    for sha in shas {
        if let Some(label) = classify_commit(sha, path, target) {
            let tag = if first { tag } else { "" };
            first = false;
            out.push(format!("{:<7}  {}  {label}", tag, short_sha(sha)));
        }
    }
}

/// The `K` popup's history section: resolves the hovered def's identity, then
/// its cross-commit history (cached per identity so a repeat `K` is instant).
/// Computed synchronously on this keypress — no async restructure, kept fast
/// by `HISTORY_WINDOW` and the single-file-per-commit engine calls.
pub(super) fn history_lines(
    app: &mut App,
    path: &str,
    name: &str,
    kind: &str,
    row: usize,
    content: &str,
) -> Vec<String> {
    let Some(review_sha) = app.review_sha.clone() else {
        return vec!["history: unavailable (no commit to review from)".to_string()];
    };
    let Some(target) = symbol_identity(path, content, name, kind, row) else {
        return vec!["history: could not resolve this symbol's identity".to_string()];
    };
    let key = (
        path.to_string(),
        target.name.clone(),
        target.kind.clone(),
        target.scope.clone(),
    );
    if let Some(cached) = app.history_cache.get(&key) {
        return cached.clone();
    }
    let lines = compute_history(&review_sha, path, &target);
    app.history_cache.insert(key, lines.clone());
    lines
}
